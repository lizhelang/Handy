//! 上屏后联想的公共二元表。只读打开随应用打包的资源，按前文最后一个词给出续写概率。
//!
//! 表中每个词记录出现次数 `count` 与以它开头的更长词的次数之和 `extend`。前文以词 X
//! 结尾时，补全（X 是更长词的开头）与下一个词使用同一个分母 `count(X) + extend(X)`，
//! 两类续写因此可以直接比较。
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 资源格式版本；与构建脚本写入的 `meta.format` 一致。
pub const FORMAT: &str = "1";
/// 反向最长匹配最多看的字数，与构建词表中的常用词长度相当。
const MAX_TAIL_CHARS: usize = 8;

/// 合并打分参数。默认值由开发集选定；`set_tuning` 仅供离线评测调参。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tuning {
    /// 个人证据的平滑常数：λ = n / (n + kappa)。
    pub kappa: f64,
    /// 合并概率低于该值的续写不展示。
    pub tau: f64,
    /// 前文最后一个词相同（而非完整前文相同）的个人证据的折扣。
    pub last_word: f64,
    /// 最后一个词没有续写、退到更短词时的折扣。
    pub backoff: f64,
    /// 公共补全（前文末尾是更长词的开头）的权重。开发集上补全几乎不命中且挤占位置，默认关闭。
    pub completion: f64,
    /// 单个续写的个人证据低于此值不参与合并。本机历史开发段上设 0.3 会降低首选命中，默认不设。
    pub min_personal: f64,
    /// 只有片段重合（前文与当前前文共享 2–4 字片段）的证据按锚点强度乘此系数。
    /// 开发段上与"上一个词相同"同时计入反而更差，且会推回刚打的字，默认不计入。
    pub anchor: f64,
}
pub const DEFAULT_TUNING: Tuning = Tuning {
    kappa: 1.0,
    tau: 0.003,
    last_word: 0.5,
    backoff: 0.4,
    completion: 0.0,
    min_personal: 0.0,
    anchor: 0.0,
};
static TUNING: RwLock<Tuning> = RwLock::new(DEFAULT_TUNING);

pub fn tuning() -> Tuning {
    TUNING.read().map(|t| *t).unwrap_or(DEFAULT_TUNING)
}
/// 仅供离线评测在开发集上调参；生产路径不调用。
pub fn set_tuning(value: Tuning) {
    if let Ok(mut current) = TUNING.write() {
        *current = value;
    }
}

pub(crate) fn cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x9fff | 0xf900..=0xfaff)
}

/// 打开并检查资源，返回文件摘要。登记前调用，查询时不再重复计算摘要。
pub(crate) fn inspect(path: &Path) -> Result<String> {
    let conn = open(path)?;
    drop(conn);
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(err)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer).map_err(err)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(err)?;
    let format: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key='format'", [], |r| {
            r.get(0)
        })
        .optional()
        .map_err(err)?;
    if format.as_deref() != Some(FORMAT) {
        return Err("联想表格式不受支持".into());
    }
    Ok(conn)
}

/// 已打开的资源按路径缓存；摘要变化（应用更新后重新登记）时重新打开。
static TABLES: Mutex<BTreeMap<PathBuf, (String, Connection)>> = Mutex::new(BTreeMap::new());

pub(crate) fn with_table<T>(
    path: &Path,
    sha256: &str,
    work: impl FnOnce(&Table<'_>) -> Result<T>,
) -> Result<Option<T>> {
    let mut tables = TABLES.lock().map_err(err)?;
    if tables.get(path).is_none_or(|(sha, _)| sha != sha256) {
        if !path.is_file() {
            tables.remove(path);
            return Ok(None);
        }
        tables.insert(path.to_owned(), (sha256.to_owned(), open(path)?));
    }
    let (_, conn) = tables.get(path).ok_or("联想表不可用")?;
    work(&Table { conn }).map(Some)
}

#[derive(Debug, Clone)]
struct Word {
    id: i64,
    text: String,
    count: f64,
    extend: f64,
}

/// 前文末尾可匹配的词，按长度从长到短。
#[derive(Debug, Clone, Default)]
pub(crate) struct Tail {
    words: Vec<Word>,
}
impl Tail {
    /// 最长匹配到的词，供个人补全使用。
    pub(crate) fn last_word(&self) -> Option<&str> {
        self.words.first().map(|w| w.text.as_str())
    }
}

pub(crate) struct Table<'a> {
    conn: &'a Connection,
}
impl Table<'_> {
    fn word(&self, text: &str) -> Result<Option<Word>> {
        self.conn
            .query_row(
                "SELECT id,count,extend FROM words WHERE text=?1",
                [text],
                |r| {
                    Ok(Word {
                        id: r.get(0)?,
                        text: text.to_owned(),
                        count: r.get::<_, i64>(1)? as f64,
                        extend: r.get::<_, i64>(2)? as f64,
                    })
                },
            )
            .optional()
            .map_err(err)
    }

    /// 对前文末尾连续汉字做反向最长匹配；末尾不是汉字时没有公共联想。
    pub(crate) fn tail(&self, context: &str) -> Result<Tail> {
        let chars: Vec<char> = context
            .chars()
            .rev()
            .take_while(|c| cjk(*c))
            .take(MAX_TAIL_CHARS)
            .collect();
        let mut words = Vec::new();
        for len in (1..=chars.len()).rev() {
            let text: String = chars[..len].iter().rev().collect();
            if let Some(word) = self.word(&text)? {
                words.push(word);
            }
        }
        Ok(Tail { words })
    }

    /// 最后一个词的续写及概率；它没有续写时依次退到更短的词并打折。
    pub(crate) fn continuations(&self, tail: &Tail, tuning: Tuning) -> Result<Vec<(String, f64)>> {
        let mut factor = 1.0;
        for word in &tail.words {
            let found = self.of_word(word, tuning.completion)?;
            if !found.is_empty() {
                let mut ranked: Vec<_> = found
                    .into_iter()
                    .map(|(text, p)| (text, p * factor))
                    .collect();
                ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                return Ok(ranked);
            }
            factor *= tuning.backoff;
        }
        Ok(Vec::new())
    }

    fn of_word(&self, word: &Word, completion_weight: f64) -> Result<HashMap<String, f64>> {
        let total = word.count + word.extend;
        let mut found: HashMap<String, f64> = HashMap::new();
        if total <= 0.0 {
            return Ok(found);
        }
        let mut next = self
            .conn
            .prepare_cached(
                "SELECT w.text,n.count FROM next n JOIN words w ON w.id=n.w WHERE n.h=?1 ORDER BY n.rank",
            )
            .map_err(err)?;
        let rows = next
            .query_map(params![word.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(err)?;
        for row in rows {
            let (text, count) = row.map_err(err)?;
            *found.entry(text).or_default() += count as f64 / total;
        }
        if completion_weight <= 0.0 {
            return Ok(found);
        }
        let mut completion = self
            .conn
            .prepare_cached(
                "SELECT w.text,w.count FROM completion c JOIN words w ON w.id=c.w WHERE c.h=?1 ORDER BY c.rank",
            )
            .map_err(err)?;
        let rows = completion
            .query_map(params![word.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(err)?;
        for row in rows {
            let (text, count) = row.map_err(err)?;
            if let Some(rest) = text.strip_prefix(word.text.as_str()) {
                if !rest.is_empty() {
                    *found.entry(rest.to_owned()).or_default() +=
                        completion_weight * count as f64 / total;
                }
            }
        }
        Ok(found)
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::*;

    /// 构造与构建脚本同结构的小表：words(text,count) 与 next(head,word,count)。
    pub(crate) fn build(path: &Path, words: &[(&str, i64)], next: &[(&str, &str, i64)]) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
             CREATE TABLE words(id INTEGER PRIMARY KEY,text TEXT NOT NULL UNIQUE,count INTEGER NOT NULL,extend INTEGER NOT NULL);
             CREATE TABLE next(h INTEGER NOT NULL,rank INTEGER NOT NULL,w INTEGER NOT NULL,count INTEGER NOT NULL,PRIMARY KEY(h,rank)) WITHOUT ROWID;
             CREATE TABLE completion(h INTEGER NOT NULL,rank INTEGER NOT NULL,w INTEGER NOT NULL,PRIMARY KEY(h,rank)) WITHOUT ROWID;
             INSERT INTO meta VALUES('format','1');",
        )
        .unwrap();
        let mut sorted: Vec<_> = words.to_vec();
        sorted.sort();
        let ids: HashMap<&str, i64> = sorted
            .iter()
            .enumerate()
            .map(|(i, (w, _))| (*w, i as i64 + 1))
            .collect();
        for (word, count) in &sorted {
            let extend: i64 = sorted
                .iter()
                .filter(|(other, _)| other.len() > word.len() && other.starts_with(word))
                .map(|(_, c)| c)
                .sum();
            conn.execute(
                "INSERT INTO words VALUES(?1,?2,?3,?4)",
                params![ids[word], word, count, extend],
            )
            .unwrap();
            let mut longer: Vec<_> = sorted
                .iter()
                .filter(|(other, _)| other.len() > word.len() && other.starts_with(word))
                .map(|(other, c)| (-c, *other))
                .collect();
            longer.sort();
            for (rank, (_, other)) in longer.iter().take(8).enumerate() {
                conn.execute(
                    "INSERT INTO completion VALUES(?1,?2,?3)",
                    params![ids[word], rank as i64, ids[other]],
                )
                .unwrap();
            }
        }
        let mut ranks: HashMap<&str, i64> = HashMap::new();
        let mut ordered: Vec<_> = next.to_vec();
        ordered.sort_by(|a, b| a.0.cmp(b.0).then(b.2.cmp(&a.2)));
        for (head, word, count) in ordered {
            let rank = ranks.entry(head).or_default();
            conn.execute(
                "INSERT INTO next VALUES(?1,?2,?3,?4)",
                params![ids[head], *rank, ids[word], count],
            )
            .unwrap();
            *rank += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_and_next_word_share_one_denominator_and_back_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("table.sqlite");
        fixture::build(
            &path,
            &[
                ("我", 600),
                ("我们", 300),
                ("的", 900),
                ("下午", 100),
                ("下午茶", 5),
                ("三点", 50),
                ("今天", 80),
                ("茶", 40),
            ],
            &[
                ("我", "的", 120),
                ("下午", "三点", 30),
                ("今天", "下午", 20),
                ("茶", "的", 8),
            ],
        );
        let sha = inspect(&path).unwrap();
        let full = Tuning {
            completion: 1.0,
            ..DEFAULT_TUNING
        };
        let (tail, result) = with_table(&path, &sha, |table| {
            let tail = table.tail("今天下午")?;
            Ok((tail.clone(), table.continuations(&tail, full)?))
        })
        .unwrap()
        .unwrap();
        assert_eq!(tail.last_word(), Some("下午"));
        // 下午茶 只是补全且次数少，排在真实下一个词之后。
        assert_eq!(result[0].0, "三点");
        assert!((result[0].1 - 30.0 / 105.0).abs() < 1e-9);
        assert_eq!(result[1].0, "茶");
        let me = with_table(&path, &sha, |table| {
            let tail = table.tail("我")?;
            table.continuations(&tail, full)
        })
        .unwrap()
        .unwrap();
        assert_eq!(me[0].0, "们");
        assert!((me[0].1 - 300.0 / 900.0).abs() < 1e-9);
        assert!((me[1].1 - 120.0 / 900.0).abs() < 1e-9);
        // “下午茶”自身没有续写，退到“茶”并打折。
        let tea = with_table(&path, &sha, |table| {
            let tail = table.tail("下午茶")?;
            table.continuations(&tail, full)
        })
        .unwrap()
        .unwrap();
        assert_eq!(tea[0].0, "的");
        assert!((tea[0].1 - 0.4 * 8.0 / 40.0).abs() < 1e-9);
        // 默认关闭公共补全时，“我”之后只剩真实下一个词。
        let next_only = with_table(&path, &sha, |table| {
            let tail = table.tail("我")?;
            table.continuations(&tail, DEFAULT_TUNING)
        })
        .unwrap()
        .unwrap();
        assert_eq!(next_only.len(), 1);
        assert_eq!(next_only[0].0, "的");
        let english = with_table(&path, &sha, |table| table.tail("hello")).unwrap();
        assert!(english.unwrap().last_word().is_none());
        // 最长词“三点”没有续写时退到“点”不存在，结果为空而不是报错。
        let empty = with_table(&path, &sha, |table| {
            let tail = table.tail("三点")?;
            table.continuations(&tail, full)
        })
        .unwrap()
        .unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn unsupported_or_missing_table_is_not_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.sqlite");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE meta(key TEXT,value TEXT); INSERT INTO meta VALUES('format','9');",
            )
            .unwrap();
        assert!(inspect(&path).is_err());
        let missing = dir.path().join("missing.sqlite");
        assert!(with_table(&missing, "x", |_| Ok(())).unwrap().is_none());
    }
}
