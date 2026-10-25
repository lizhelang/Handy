//! 上屏后联想的公共二元表。只读打开随应用打包的资源，按前文最后一个词给出续写概率。
//!
//! 表中每个词记录出现次数 `count` 与以它开头的更长词的次数之和 `extend`。前文以词 X
//! 结尾时，补全（X 是更长词的开头）与下一个词使用同一个分母 `count(X) + extend(X)`，
//! 两类续写因此可以直接比较。
//!
//! 格式 2 另有 `punct`（词后紧跟各标点的次数）与 `emoji`（词→表情）两张表：分句里有问句
//! 线索时给出"？"，有感叹线索时给出"！"；前文以表情词结尾时给出一个表情。"。""，"不由公共表给出。
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, RwLock};

type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 资源格式版本；与构建脚本写入的 `meta.format` 一致。格式 1 没有标点与表情表，仍可读取。
pub const FORMAT: &str = "2";
const FORMATS: [&str; 2] = ["1", FORMAT];
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
    /// 用户确认回填的历史中，每出现一次"上一个词→下一个词"计入的个人证据；开发段上 0.05–0.5 差别很小。
    pub history: f64,
    /// 分句有疑问词、正反问或选择问线索时，"？"的概率取 max(P(？|X), question × P(句末|X))。
    pub question: f64,
    /// 分句有感叹线索时，"！"的概率取 max(P(！|X), exclaim × P(句末|X))。
    pub exclaim: f64,
    /// 有疑问词、正反问或选择问线索时"？"的最低概率；书面语料里问句常以动词收尾，句末比例偏低。
    pub question_floor: f64,
    /// 句末是实义词正反问或"……过没有"时"？"的最低概率。
    pub question_closing: f64,
    /// 疑问词线索之后最多再有几个字仍按问句处理。
    pub question_reach: f64,
    /// 标点低于该概率不展示。
    pub punct_tau: f64,
    /// 为 0 时不给表情，仅供评测对照。
    pub emoji: f64,
}
pub const DEFAULT_TUNING: Tuning = Tuning {
    kappa: 1.0,
    tau: 0.003,
    last_word: 0.5,
    backoff: 0.4,
    completion: 0.0,
    min_personal: 0.0,
    anchor: 0.0,
    history: 0.1,
    question: 2.0,
    exclaim: 1.0,
    question_floor: 0.0,
    question_closing: 0.1,
    question_reach: 8.0,
    punct_tau: 0.05,
    emoji: 1.0,
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

/// 单条历史最多记录的不同词对，避免超长正文占满个人层。
const MAX_HISTORY_PAIRS: usize = 256;

/// 用联想表词表做正向最长匹配，统计相邻词对；标点、英文与词表外的字处断开，不组成词对。
pub(crate) fn bigrams(text: &str, vocabulary: &HashSet<String>) -> HashMap<(String, String), u32> {
    let chars: Vec<char> = text.chars().collect();
    let mut pairs: HashMap<(String, String), u32> = HashMap::new();
    let mut previous: Option<String> = None;
    let mut i = 0;
    while i < chars.len() {
        let run = chars[i..]
            .iter()
            .take(MAX_TAIL_CHARS)
            .take_while(|c| cjk(**c))
            .count();
        let word = (1..=run)
            .rev()
            .map(|len| chars[i..i + len].iter().collect::<String>())
            .find(|word| vocabulary.contains(word));
        let Some(word) = word else {
            previous = None;
            i += 1;
            continue;
        };
        i += word.chars().count();
        if let Some(previous) = previous.replace(word.clone()) {
            if pairs.len() < MAX_HISTORY_PAIRS
                || pairs.contains_key(&(previous.clone(), word.clone()))
            {
                *pairs.entry((previous, word)).or_default() += 1;
            }
        }
    }
    pairs
}

pub(crate) fn cjk(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x9fff | 0xf900..=0xfaff)
}

/// 联想给出的标点（全角或半角）；只有问号、感叹号来自公共表，其余只可能来自个人证据。
pub(crate) fn is_punctuation(text: &str) -> bool {
    matches!(
        text,
        "，" | "。" | "？" | "！" | "；" | "：" | "," | "." | "?" | "!" | ";" | ":"
    )
}

/// 联想标点跟随这段文字里汉字后已经用过的标点；还没有时按输入法默认的"中文下用英文标点"给半角。
fn half_width_marks(context: &str) -> bool {
    let chars: Vec<char> = context.chars().collect();
    chars
        .windows(2)
        .rev()
        .find(|w| cjk(w[0]) && "，。？！；：,.?!;:".contains(w[1]))
        .is_none_or(|w| w[1].is_ascii())
}

/// 前文最后一个分句：最后一个句读之后的文字。
fn clause(context: &str) -> &str {
    let start = context
        .char_indices()
        .rev()
        .find(|(_, c)| "，。？！；：,.?!;:\n".contains(*c))
        .map_or(0, |(i, c)| i + c.len_utf8());
    &context[start..]
}

/// 句末语气词：以它结尾时直接按公共表中它后面出现"？"的比例给问号。
const QUESTION_PARTICLES: [char; 5] = ['吗', '呢', '吧', '么', '嘛'];
const PRONOUN_ME: [&str; 7] = ["什么", "怎么", "这么", "那么", "多么", "要么", "干么"];
/// 疑问代词与疑问副词；"不管/无论"之后、后接"都/也"、或在"不知道"等之后作宾语时不算问句。
const QUESTION_WORDS: [&str; 16] = [
    "什么",
    "为什么",
    "为啥",
    "怎么",
    "怎样",
    "咋",
    "啥",
    "谁",
    "哪",
    "几",
    "多少",
    "多久",
    "多大",
    "多长",
    "多远",
    "如何",
];
/// 这些词之后的疑问词或正反问是宾语从句（"不知道他为什么没来""看看能不能拆"）、转述
/// （"他问我去不去"）或任指（"好像在哪里见过""如果谁问起"），不是问句。
const NOT_QUESTION_BEFORE: [&str; 26] = [
    "不管",
    "无论",
    "不论",
    "没",
    "不知道",
    "不清楚",
    "不明白",
    "不记得",
    "不确定",
    "想知道",
    "不知",
    "了解",
    "看看",
    "问问",
    "试试",
    "好像",
    "似乎",
    "如果",
    "要是",
    "假如",
    "问我",
    "问他",
    "问她",
    "问过",
    "说不清",
    "说不准",
];
/// 疑问词后三字内出现"都/也"是任指（"怎么走都差不多"），不是问句。
const NOT_QUESTION_AFTER: [char; 2] = ['都', '也'];
/// 疑问词之后出现这些说法时，疑问词是条件或交给对方决定的话题（"谁要是问起来""怎么处理你定"）。
const NOT_QUESTION_LATER: [&str; 9] = [
    "要是",
    "如果",
    "你定",
    "说了算",
    "你决定",
    "随你",
    "都行",
    "都可以",
    "看着办",
];
/// 正反问中不表疑问的固定说法。
const NOT_A_NOT_A: [&str; 3] = ["动不动", "时不时", "差不多"];
/// 能愿动词的正反问（"能不能""要不要"）后面通常还有动词，不能单凭它判断到了句末。
const MODALS: [char; 9] = ['能', '要', '会', '是', '可', '有', '该', '敢', '愿'];

/// 分句里的问句线索。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Cue {
    /// 最后一个线索之后还有几个字；问句的疑问词一般离句末不远。
    pub(crate) distance: usize,
    /// 句末就是实义词的正反问（"难不难""抢没抢到"）或"……过没有"，几乎总是问句。
    pub(crate) closing: bool,
}

/// 分句里有疑问词、正反问（"是不是""有没有"）或"A 还是 B"选择问的线索时返回线索位置。
pub(crate) fn question_cue(clause: &str) -> Option<Cue> {
    let chars: Vec<char> = clause.chars().collect();
    let text = |from: usize, to: usize| chars[from..to].iter().collect::<String>();
    let mut last: Option<(usize, bool)> = None;
    let mut found = |end: usize, closing: bool| {
        if last.is_none_or(|(l, _)| end >= l) {
            last = Some((end, closing));
        }
    };
    for word in QUESTION_WORDS {
        let length = word.chars().count();
        for start in 0..chars.len().saturating_sub(length - 1) {
            if text(start, start + length) != word {
                continue;
            }
            let before = text(0, start);
            let after = text(start + length, chars.len());
            let indefinite = match word {
                "几" => {
                    after.starts_with('乎') || before.ends_with(['好', '这', '那', '了', '十'])
                }
                "哪" => after.starts_with('怕') || before.ends_with('天'),
                // "水果什么的"表示列举未尽；"不怎么"是程度。
                "什么" => after == "的",
                "怎么" => before.ends_with('不'),
                _ => false,
            };
            // 同一疑问词出现两次是"想吃什么就点什么"式的照应，表任指。
            if !indefinite
                && clause.matches(word).count() == 1
                && !NOT_QUESTION_BEFORE.iter().any(|w| before.contains(w))
                && !after
                    .chars()
                    .take(4)
                    .any(|c| NOT_QUESTION_AFTER.contains(&c))
                && !NOT_QUESTION_LATER.iter().any(|w| after.contains(w))
            {
                found(start + length, false);
            }
        }
    }
    for (i, w) in chars.windows(3).enumerate() {
        if w[0] == w[2]
            && matches!(w[1], '不' | '没')
            && !matches!(w[0], '不' | '没')
            && cjk(w[0])
            && !NOT_A_NOT_A.contains(&text(i, i + 3).as_str())
            && !NOT_QUESTION_BEFORE.iter().any(|b| text(0, i).contains(b))
            && !NOT_QUESTION_LATER
                .iter()
                .any(|l| text(i + 3, chars.len()).contains(l))
        {
            found(i + 3, !MODALS.contains(&w[0]) && chars.len() - (i + 3) <= 1);
        }
    }
    // "还是"前面有内容且不是紧跟主语时才是选择问；"我还是觉得"是陈述。
    for (i, _) in clause.match_indices("还是") {
        if clause[..i]
            .chars()
            .last()
            .is_some_and(|c| cjk(c) && !"我你他她它们咱".contains(c))
            && !NOT_QUESTION_LATER
                .iter()
                .any(|l| clause[i + "还是".len()..].contains(l))
        {
            found(clause[..i].chars().count() + 2, false);
        }
    }
    if clause.ends_with("过没有") || clause.ends_with("了没有") {
        found(chars.len(), true);
    }
    last.map(|(end, closing)| Cue {
        distance: chars.len() - end,
        closing,
    })
}

/// "吧"表推测而非建议或感叹：紧跟"了"，或分句里有"应该/大概/可能/没/不会/是"等；
/// 带"太"的是感叹（"太夸张了吧"）。
fn supposition(clause: &str) -> bool {
    let head = clause.trim_end_matches('吧');
    !head.contains('太')
        && (head.ends_with('了')
            || [
                "应该", "大概", "可能", "也许", "没", "不会", "是", "觉得", "肯定", "一定",
            ]
            .iter()
            .any(|w| head.contains(w)))
}

/// 感叹线索："太……了"、程度词加"啊/呀"、句首"哇/天哪"等。
pub(crate) fn exclaim_cue(clause: &str) -> bool {
    let degree = ["太", "真", "好", "多么", "超", "简直", "非常", "特别"];
    ["哇", "哎呀", "天哪", "天啊"]
        .iter()
        .any(|w| clause.starts_with(w))
        || (clause.contains('太') && clause.ends_with(['了', '啦']))
        || (degree.iter().any(|d| clause.contains(d)) && clause.ends_with(['啊', '呀', '哇']))
}

/// 句末语气词：表情词后面跟这些字时，去掉一个再匹配。
const TRAILING_PARTICLES: [char; 11] = [
    '了', '啦', '呀', '啊', '哦', '喔', '哟', '嘛', '吧', '哇', '耶',
];
/// 单字表情词只在这些程度词之后触发，例如"好冷""太累"。
const DEGREE_WORDS: [&str; 10] = [
    "好", "太", "真", "很", "超", "挺", "巨", "特别", "非常", "有点",
];
/// 否定之后不给表情，"不开心"不应推荐笑脸。
const NEGATIONS: [&str; 3] = ["不", "没", "别"];
const MAX_EMOJI_KEYWORD_CHARS: usize = 6;

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
    if !format.as_deref().is_some_and(|f| FORMATS.contains(&f)) {
        return Err("联想表格式不受支持".into());
    }
    Ok(conn)
}

fn has_extras(conn: &Connection) -> Result<bool> {
    conn.query_row(
        "SELECT count(*)=2 FROM sqlite_master WHERE type='table' AND name IN ('punct','emoji')",
        [],
        |r| r.get(0),
    )
    .map_err(err)
}

/// 已打开的资源按路径缓存；摘要变化（应用更新后重新登记）时重新打开。
static TABLES: Mutex<BTreeMap<PathBuf, (String, Connection, bool)>> = Mutex::new(BTreeMap::new());

pub(crate) fn with_table<T>(
    path: &Path,
    sha256: &str,
    work: impl FnOnce(&Table<'_>) -> Result<T>,
) -> Result<Option<T>> {
    let mut tables = TABLES.lock().map_err(err)?;
    if tables.get(path).is_none_or(|(sha, _, _)| sha != sha256) {
        if !path.is_file() {
            tables.remove(path);
            return Ok(None);
        }
        let conn = open(path)?;
        let extras = has_extras(&conn)?;
        tables.insert(path.to_owned(), (sha256.to_owned(), conn, extras));
    }
    let (_, conn, extras) = tables.get(path).ok_or("联想表不可用")?;
    work(&Table {
        conn,
        extras: *extras,
    })
    .map(Some)
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
    /// 是否带标点与表情表（格式 2）。
    extras: bool,
}
impl Table<'_> {
    /// 全部收录词，回填切分历史时一次载入。
    pub(crate) fn vocabulary(&self) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT text FROM words").map_err(err)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(err)?;
        rows.collect::<std::result::Result<_, _>>().map_err(err)
    }

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

    /// 句末标点、概率与能否排第一，概率与续写同一分母。分句以疑问语气词结尾或有问句线索时才给
    /// "？"，有感叹线索时才给"！"；最后一个词没有标点统计时退到更短的词并打折。只有语气词结尾的
    /// 问号可以排第一：疑问词线索在句中也成立，问号排第一会挡住更可能的续写。
    pub(crate) fn punctuation(
        &self,
        tail: &Tail,
        context: &str,
        tuning: Tuning,
    ) -> Result<Vec<(String, f64, bool)>> {
        if !self.extras {
            return Ok(Vec::new());
        }
        let clause = clause(context);
        // "我们走吧""多少吃一点吧"是建议，"下班了吧""是新来的吧"才是推测问；建议句里的疑问词
        // 也是任指，整句不算问句。
        let suggestion = clause.ends_with('吧') && !supposition(clause);
        let cue = question_cue(clause)
            .filter(|cue| !suggestion && cue.distance as f64 <= tuning.question_reach);
        let question = cue.is_some();
        // "在吃饭呢""正忙着呢"是陈述；没有其他问句线索时不算疑问语气。
        // "什么""怎么""这么"等词尾的"么"不是语气词。
        let particle = clause.ends_with(QUESTION_PARTICLES)
            && !PRONOUN_ME.iter().any(|w| clause.ends_with(w))
            && !suggestion
            && (question || !(clause.ends_with('呢') && clause.contains(['在', '着', '还', '正'])));
        let exclaim = !particle && !question && exclaim_cue(clause);
        if !particle && !question && !exclaim {
            return Ok(Vec::new());
        }
        let mark = if exclaim { "！" } else { "？" };
        // 最后一个词没有标点统计时退到更短的词并打折。
        let (mut raw, mut end, mut factor) = (0.0, 0.0, 1.0);
        for word in &tail.words {
            let marks = self.marks(word)?;
            if !marks.is_empty() {
                raw = marks.get(mark).copied().unwrap_or(0.0) * factor;
                end = marks.values().sum::<f64>() * factor;
                break;
            }
            factor *= tuning.backoff;
        }
        let p = if exclaim {
            raw.max(tuning.exclaim * end)
        } else if question {
            let floor = if cue.is_some_and(|cue| cue.closing) {
                tuning.question_closing
            } else {
                tuning.question_floor
            };
            raw.max(tuning.question * end).max(floor)
        } else {
            raw
        };
        let shown = match (mark, half_width_marks(context)) {
            ("？", true) => "?",
            ("！", true) => "!",
            (mark, _) => mark,
        };
        Ok(if p >= tuning.punct_tau {
            vec![(shown.to_owned(), p, particle)]
        } else {
            Vec::new()
        })
    }

    fn marks(&self, word: &Word) -> Result<HashMap<String, f64>> {
        let total = word.count + word.extend;
        if total <= 0.0 {
            return Ok(HashMap::new());
        }
        let mut stmt = self
            .conn
            .prepare_cached("SELECT mark,count FROM punct WHERE h=?1")
            .map_err(err)?;
        let rows = stmt
            .query_map(params![word.id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(err)?;
        rows.map(|row| row.map(|(mark, count)| (mark, count as f64 / total)))
            .collect::<std::result::Result<_, _>>()
            .map_err(err)
    }

    /// 前文以表情词结尾时的表情，按合适程度排序。表情词必须是前文末尾的完整词，不能是更长词的
    /// 后半截；单字词只在程度词之后；否定词之后不给；末尾的语气词去掉一个再匹配。
    pub(crate) fn emoji(&self, context: &str) -> Result<Vec<String>> {
        if !self.extras {
            return Ok(Vec::new());
        }
        let mut run: Vec<char> = context
            .chars()
            .rev()
            .take_while(|c| cjk(*c))
            .take(MAX_TAIL_CHARS)
            .collect();
        run.reverse();
        let mut texts = vec![run.clone()];
        if run.len() > 1 && run.last().is_some_and(|c| TRAILING_PARTICLES.contains(c)) {
            texts.push(run[..run.len() - 1].to_vec());
        }
        for text in texts {
            let suffix = |length: usize| text[text.len() - length..].iter().collect::<String>();
            let mut longest_word = 0;
            for length in (1..=text.len()).rev() {
                if self.word(&suffix(length))?.is_some() {
                    longest_word = length;
                    break;
                }
            }
            for length in (1..=text.len().min(MAX_EMOJI_KEYWORD_CHARS)).rev() {
                let found = self.emoji_of(&suffix(length))?;
                if found.is_empty() {
                    continue;
                }
                let head: String = text[..text.len() - length].iter().collect();
                if NEGATIONS.iter().any(|n| head.ends_with(n)) {
                    return Ok(Vec::new());
                }
                let degree = DEGREE_WORDS.iter().any(|d| head.ends_with(d));
                if degree || (length > 1 && longest_word <= length) {
                    return Ok(found);
                }
            }
        }
        Ok(Vec::new())
    }

    fn emoji_of(&self, keyword: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT emoji FROM emoji WHERE keyword=?1 ORDER BY rank")
            .map_err(err)?;
        let rows = stmt
            .query_map([keyword], |r| r.get::<_, String>(0))
            .map_err(err)?;
        rows.collect::<std::result::Result<_, _>>().map_err(err)
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

    /// 在 `build` 的基础上加格式 2 的标点与表情表。
    pub(crate) fn build_with_extras(
        path: &Path,
        words: &[(&str, i64)],
        next: &[(&str, &str, i64)],
        marks: &[(&str, &str, i64)],
        emoji: &[(&str, &str)],
    ) {
        build(path, words, next);
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "UPDATE meta SET value='2' WHERE key='format';
             CREATE TABLE punct(h INTEGER NOT NULL,mark TEXT NOT NULL,count INTEGER NOT NULL,PRIMARY KEY(h,mark)) WITHOUT ROWID;
             CREATE TABLE emoji(keyword TEXT NOT NULL,rank INTEGER NOT NULL,emoji TEXT NOT NULL,PRIMARY KEY(keyword,rank)) WITHOUT ROWID;",
        )
        .unwrap();
        for (word, mark, count) in marks {
            conn.execute(
                "INSERT INTO punct SELECT id,?2,?3 FROM words WHERE text=?1",
                params![word, mark, count],
            )
            .unwrap();
        }
        for (rank, (keyword, value)) in emoji.iter().enumerate() {
            conn.execute(
                "INSERT INTO emoji VALUES(?1,?2,?3)",
                params![keyword, rank as i64, value],
            )
            .unwrap();
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
    fn history_is_split_into_adjacent_word_pairs_without_crossing_punctuation() {
        let vocabulary: HashSet<String> = ["我们", "明天", "开会", "下午", "的", "会议"]
            .into_iter()
            .map(String::from)
            .collect();
        let pairs = bigrams("我们明天开会，下午的会议abc我们开会", &vocabulary);
        assert_eq!(pairs[&("我们".into(), "明天".into())], 1);
        assert_eq!(pairs[&("明天".into(), "开会".into())], 1);
        assert_eq!(pairs[&("我们".into(), "开会".into())], 1);
        assert_eq!(pairs[&("的".into(), "会议".into())], 1);
        // 逗号与英文处断开，不产生跨句词对。
        assert!(!pairs.contains_key(&("开会".into(), "下午".into())));
        assert!(!pairs.contains_key(&("会议".into(), "我们".into())));
    }

    #[test]
    fn question_cues_skip_reported_indefinite_and_conditional_uses() {
        for clause in [
            "你叫什么名字",
            "你几点下班",
            "这周的周报谁来写",
            "你去不去",
            "你喝茶还是咖啡",
            "文件核对过没有",
        ] {
            assert!(question_cue(clause).is_some(), "{clause}");
        }
        for clause in [
            "什么都行",
            "不管谁来",
            "我不知道他为什么没来",
            "他问我去不去",
            "你想吃什么就点什么",
            "谁要是问起来",
            "水果什么的",
            "几乎没人",
            "哪怕下雨",
            "天哪",
            "动不动就生气",
            "我还是觉得不错",
            "怎么处理你说了算",
            "我们明天开会",
        ] {
            assert!(question_cue(clause).is_none(), "{clause}");
        }
        // 句末就是实义词的正反问几乎总是问句；能愿动词的正反问后面还要接动词。
        assert!(question_cue("明天的考试难不难").unwrap().closing);
        assert!(!question_cue("你能不能").unwrap().closing);
        assert_eq!(question_cue("你叫什么名字").unwrap().distance, 2);
        assert!(exclaim_cue("太好了") && exclaim_cue("真漂亮啊") && !exclaim_cue("我们开会"));
        assert_eq!(clause("好的，你几点到"), "你几点到");
    }

    #[test]
    fn question_mark_and_emoji_follow_cues_on_a_format_two_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("table.sqlite");
        fixture::build_with_extras(
            &path,
            &[
                ("吗", 1000),
                ("吧", 1000),
                ("名字", 1000),
                ("开会", 1000),
                ("哈哈", 100),
                ("开心", 100),
                ("不开心", 100),
                ("冷", 100),
                ("我", 100),
                ("的", 100),
            ],
            &[("名字", "的", 50)],
            &[
                ("吗", "？", 900),
                ("吧", "？", 90),
                ("吧", "。", 400),
                ("名字", "。", 300),
                ("名字", "？", 5),
                ("开会", "。", 300),
            ],
            &[("哈哈", "😄"), ("开心", "😊"), ("冷", "🥶")],
        );
        let sha = inspect(&path).unwrap();
        let ask = |context: &str| {
            with_table(&path, &sha, |table| {
                let tail = table.tail(context)?;
                Ok((
                    table.punctuation(&tail, context, DEFAULT_TUNING)?,
                    table.emoji(context)?,
                ))
            })
            .unwrap()
            .unwrap()
        };
        // 语气词结尾：问号概率取公共表比例，可以排第一。
        let (marks, _) = ask("你认识我吗");
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].0, "?");
        assert!((marks[0].1 - 0.9).abs() < 1e-9 && marks[0].2);
        // 疑问词线索：按句末比例加权，但不能排第一。
        let (marks, _) = ask("你叫什么名字");
        assert!((marks[0].1 - 2.0 * 305.0 / 1000.0).abs() < 1e-9 && !marks[0].2);
        // 陈述句、建议"吧"不给问号；公共表不给句号。
        assert!(ask("我们明天开会").0.is_empty());
        assert!(ask("我们走吧").0.is_empty());
        assert_eq!(ask("他下班了吧").0[0].0, "?");
        // 这段文字里已经用过全角标点时给全角问号。
        assert_eq!(ask("好的，他下班了吧").0[0].0, "？");
        assert_eq!(ask("好的,他下班了吧").0[0].0, "?");
        // 表情：完整词、去掉句末语气词、程度词加单字；否定与单字不给。
        assert_eq!(ask("哈哈").1, vec!["😄".to_string()]);
        assert_eq!(ask("我好开心啊").1, vec!["😊".to_string()]);
        assert_eq!(ask("今天好冷").1, vec!["🥶".to_string()]);
        assert!(ask("冷").1.is_empty());
        assert!(ask("我不开心").1.is_empty());
        assert!(ask("有点不开心").1.is_empty());
    }

    #[test]
    fn format_one_table_still_answers_without_marks_or_emoji() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("table.sqlite");
        fixture::build(&path, &[("吗", 10), ("哈哈", 10)], &[]);
        let sha = inspect(&path).unwrap();
        let (marks, emoji) = with_table(&path, &sha, |table| {
            let tail = table.tail("你好吗")?;
            Ok((
                table.punctuation(&tail, "你好吗", DEFAULT_TUNING)?,
                table.emoji("哈哈")?,
            ))
        })
        .unwrap()
        .unwrap();
        assert!(marks.is_empty() && emoji.is_empty());
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
