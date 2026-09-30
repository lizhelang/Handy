//! 查询绑定的旧学习域快照。仅复用排序算法，不持数据库或进程租约，也不代表隐私授权。
use crate::{AppPolicy, LocalMemory, MemoryTerm};
use std::collections::HashSet;

pub const MAX_SNAPSHOT_TERMS: usize = 1_024;
pub const MAX_SNAPSHOT_TEXT_BYTES: usize = 512 * 1_024;
pub const MAX_RESULT_LIMIT: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryQuery {
    Rank { candidate_texts: Vec<String> },
    Completion { prefix: String, limit: usize },
    EnglishCompletion { prefix: String, limit: usize },
    Clipboard { limit: usize },
    VoiceHotwords { limit: usize },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    InvalidQuery,
    InvalidTerm,
    DuplicateTerm,
    BudgetExceeded,
    QueryMismatch,
    StorageUnavailable,
}
impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidQuery => "memory snapshot query invalid",
            Self::InvalidTerm => "memory snapshot term invalid",
            Self::DuplicateTerm => "memory snapshot term duplicated",
            Self::BudgetExceeded => "memory snapshot budget exceeded",
            Self::QueryMismatch => "memory snapshot query changed",
            Self::StorageUnavailable => "memory snapshot storage unavailable",
        })
    }
}
impl std::error::Error for SnapshotError {}
type Result<T> = std::result::Result<T, SnapshotError>;

impl MemoryQuery {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Rank { candidate_texts } => {
                if candidate_texts.is_empty() || candidate_texts.len() > MAX_SNAPSHOT_TERMS {
                    return Err(SnapshotError::InvalidQuery);
                }
                let mut bytes = 0usize;
                for text in candidate_texts {
                    if text.is_empty() || text.chars().any(char::is_control) {
                        return Err(SnapshotError::InvalidQuery);
                    }
                    bytes = bytes
                        .checked_add(text.len())
                        .ok_or(SnapshotError::BudgetExceeded)?;
                    if bytes > MAX_SNAPSHOT_TEXT_BYTES {
                        return Err(SnapshotError::BudgetExceeded);
                    }
                }
            }
            Self::Completion { prefix, limit } | Self::EnglishCompletion { prefix, limit } => {
                if prefix.len() > 1_024
                    || prefix.chars().any(char::is_control)
                    || !(1..=MAX_RESULT_LIMIT).contains(limit)
                {
                    return Err(SnapshotError::InvalidQuery);
                }
            }
            Self::Clipboard { limit } | Self::VoiceHotwords { limit } => {
                if !(1..=MAX_RESULT_LIMIT).contains(limit) {
                    return Err(SnapshotError::InvalidQuery);
                }
            }
        }
        Ok(())
    }
}

pub(crate) fn validate_terms(terms: &[MemoryTerm]) -> Result<()> {
    if terms.len() > MAX_SNAPSHOT_TERMS {
        return Err(SnapshotError::BudgetExceeded);
    }
    let mut bytes = 0usize;
    let mut seen = HashSet::new();
    for term in terms {
        bytes = bytes
            .checked_add(term.text.len())
            .ok_or(SnapshotError::BudgetExceeded)?;
        if bytes > MAX_SNAPSHOT_TEXT_BYTES {
            return Err(SnapshotError::BudgetExceeded);
        }
        if term.text.is_empty()
            || term.text.chars().any(char::is_control)
            || crate::normalize_term(term.text.clone()) != term.text
        {
            return Err(SnapshotError::InvalidTerm);
        }
        if !seen.insert(&term.text) {
            return Err(SnapshotError::DuplicateTerm);
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct MemorySnapshot {
    query: MemoryQuery,
    terms: Vec<MemoryTerm>,
}
impl MemorySnapshot {
    /// 检查值域和查询成员关系；调用者必须另行核对发送者身份、完整响应及租约。
    pub fn new(query: MemoryQuery, terms: Vec<MemoryTerm>) -> Result<Self> {
        query.validate()?;
        validate_terms(&terms)?;
        let valid = match &query {
            MemoryQuery::Rank { candidate_texts } => terms
                .iter()
                .all(|term| candidate_texts.contains(&term.text)),
            MemoryQuery::Completion { prefix, limit } => {
                terms.len() <= *limit && terms.iter().all(|term| term.text.starts_with(prefix))
            }
            MemoryQuery::EnglishCompletion { prefix, limit } => {
                let prefix = prefix.trim();
                terms.len() <= *limit
                    && terms.iter().all(|term| {
                        prefix.len() >= 2
                            && prefix.is_ascii()
                            && crate::is_ascii_word(&term.text)
                            && term.text.len() > prefix.len()
                            && term
                                .text
                                .to_ascii_lowercase()
                                .starts_with(&prefix.to_ascii_lowercase())
                    })
            }
            MemoryQuery::Clipboard { limit } => {
                terms.len() <= *limit && terms.iter().all(|term| term.clipboard_count > 0)
            }
            MemoryQuery::VoiceHotwords { limit } => {
                terms.len() <= *limit
                    && terms
                        .iter()
                        .all(|term| term.typed_count > 0 || term.voice_count > 0)
            }
        };
        if !valid {
            return Err(SnapshotError::QueryMismatch);
        }
        Ok(Self { query, terms })
    }
    pub fn query(&self) -> &MemoryQuery {
        &self.query
    }
    pub fn terms(&self) -> &[MemoryTerm] {
        &self.terms
    }
    /// 查询改变必须重新取得快照。纯值返回不能延长任何运行时租约。
    pub fn memory_for(&self, query: &MemoryQuery, policy: AppPolicy) -> Result<LocalMemory> {
        if query != &self.query {
            return Err(SnapshotError::QueryMismatch);
        }
        LocalMemory::from_snapshot(policy, self.terms.clone())
    }
}

/// 借用唯一服务写者的连接；不打开第二连接、不写用户库、不全量读取旧词库。
#[cfg(feature = "sqlite-memory")]
pub fn read_query_snapshot(
    connection: &rusqlite::Connection,
    query: &MemoryQuery,
) -> Result<MemorySnapshot> {
    use rusqlite::params;
    query.validate()?;
    const COLUMNS: &str = "text,typed_count,voice_count,clipboard_count,last_used_tick";
    const SCORE: &str =
        "MIN(2147483647,typed_count*60+voice_count*35+clipboard_count*20+MIN(last_used_tick,10))";
    let db_err = |_| SnapshotError::StorageUnavailable;
    let mut terms = Vec::new();
    let mut bytes = 0usize;
    match query {
        MemoryQuery::Rank { candidate_texts } => {
            // 一条 SELECT 固定 SQLite 读快照；不在每个候选之间开启新的读取事务。
            let placeholders = vec!["?"; candidate_texts.len()].join(",");
            let mut statement = connection
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM inputia_terms WHERE text IN ({placeholders})"
                ))
                .map_err(db_err)?;
            let mut rows = statement
                .query(rusqlite::params_from_iter(candidate_texts))
                .map_err(db_err)?;
            while let Some(row) = rows.next().map_err(db_err)? {
                terms.push(read_term(row, &mut bytes)?);
            }
        }
        other => {
            let (filter, order, prefix, limit) = match other {
                MemoryQuery::Completion { prefix, limit } => ("substr(text,1,length(?1))=?1", format!("{SCORE} DESC,rowid ASC"), prefix.clone(), *limit),
                MemoryQuery::EnglishCompletion { prefix, limit } => {
                    let prefix = prefix.trim();
                    if prefix.len() < 2 || !prefix.is_ascii() { return MemorySnapshot::new(query.clone(), vec![]); }
                    ("text NOT GLOB '*[^a-zA-Z0-9_-]*' AND length(text)>length(?1) AND substr(lower(text),1,length(?1))=?1", format!("{SCORE} DESC,text COLLATE BINARY ASC"), prefix.to_ascii_lowercase(), *limit)
                }
                MemoryQuery::Clipboard { limit } => ("clipboard_count>0 AND ?1=''", "MIN(2147483647,clipboard_count*100+MIN(last_used_tick,10)) DESC,last_used_tick DESC,text COLLATE BINARY ASC".into(), String::new(), *limit),
                MemoryQuery::VoiceHotwords { limit } => ("(typed_count>0 OR voice_count>0) AND ?1=''", format!("{SCORE} DESC,rowid ASC"), String::new(), *limit),
                MemoryQuery::Rank { .. } => unreachable!(),
            };
            let mut statement = connection
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM inputia_terms WHERE {filter} ORDER BY {order} LIMIT ?2"
                ))
                .map_err(db_err)?;
            let mut rows = statement
                .query(params![prefix, limit as i64])
                .map_err(db_err)?;
            while let Some(row) = rows.next().map_err(db_err)? {
                terms.push(read_term(row, &mut bytes)?);
            }
        }
    }
    MemorySnapshot::new(query.clone(), terms)
}

#[cfg(feature = "sqlite-memory")]
fn read_term(row: &rusqlite::Row<'_>, bytes: &mut usize) -> Result<MemoryTerm> {
    let db_err = |_| SnapshotError::StorageUnavailable;
    let text = row
        .get_ref(0)
        .map_err(db_err)?
        .as_str()
        .map_err(|_| SnapshotError::InvalidTerm)?;
    *bytes = bytes
        .checked_add(text.len())
        .ok_or(SnapshotError::BudgetExceeded)?;
    if *bytes > MAX_SNAPSHOT_TEXT_BYTES {
        return Err(SnapshotError::BudgetExceeded);
    }
    Ok(MemoryTerm {
        text: text.into(),
        typed_count: row.get(1).map_err(db_err)?,
        voice_count: row.get(2).map_err(db_err)?,
        clipboard_count: row.get(3).map_err(db_err)?,
        last_used_tick: row.get(4).map_err(db_err)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Candidate, MemorySource};
    fn term(text: &str) -> MemoryTerm {
        MemoryTerm {
            text: text.into(),
            typed_count: 3,
            voice_count: 1,
            clipboard_count: 2,
            last_used_tick: 12,
        }
    }
    #[test]
    fn snapshot_rejects_duplicates_budgets_and_changed_queries() {
        let query = MemoryQuery::Rank {
            candidate_texts: vec!["Inputia".into(), "其他".into()],
        };
        assert!(matches!(
            MemorySnapshot::new(query.clone(), vec![term("Inputia"), term("Inputia")]),
            Err(SnapshotError::DuplicateTerm)
        ));
        assert!(matches!(
            MemorySnapshot::new(query.clone(), vec![term("不在查询中")]),
            Err(SnapshotError::QueryMismatch)
        ));
        assert!(matches!(
            LocalMemory::from_snapshot(AppPolicy::default(), vec![term("a\0b")]),
            Err(SnapshotError::InvalidTerm)
        ));
        assert!(matches!(
            LocalMemory::from_snapshot(
                AppPolicy::default(),
                vec![term(&"x".repeat(MAX_SNAPSHOT_TEXT_BYTES + 1))]
            ),
            Err(SnapshotError::BudgetExceeded)
        ));
        let snapshot = MemorySnapshot::new(query.clone(), vec![term("Inputia")]).unwrap();
        assert!(snapshot.memory_for(&query, AppPolicy::default()).is_ok());
        assert!(matches!(
            snapshot.memory_for(&MemoryQuery::Clipboard { limit: 1 }, AppPolicy::default()),
            Err(SnapshotError::QueryMismatch)
        ));
        assert!(MemoryQuery::Rank {
            candidate_texts: vec!["a".into(); MAX_SNAPSHOT_TERMS + 1]
        }
        .validate()
        .is_err());
    }
    #[test]
    fn maximum_counts_and_tick_saturate_without_changing_normal_ranking() {
        let mut huge = term("Inputia");
        huge.typed_count = u32::MAX;
        huge.voice_count = u32::MAX;
        huge.clipboard_count = u32::MAX;
        huge.last_used_tick = u64::MAX;
        let mut memory = LocalMemory::from_snapshot(AppPolicy::default(), vec![huge]).unwrap();
        let mut candidate = Candidate::new("0", "Inputia");
        candidate.base_score = i32::MAX;
        candidate.memory_score = 1;
        let ranked = memory.rank_candidates(vec![candidate]);
        assert_eq!(ranked[0].memory_score, i32::MAX);
        assert_eq!(ranked[0].final_score(), i32::MAX);
        assert_eq!(memory.clipboard_candidates(1)[0].memory_score, i32::MAX);
        memory.learn(
            MemorySource::Typed,
            "Inputia",
            &crate::AppContext::new("com.example.editor"),
        );
        assert_eq!(memory.terms[0].typed_count, u32::MAX);
        assert_eq!(memory.terms[0].last_used_tick, u64::MAX);
    }
    #[cfg(feature = "sqlite-memory")]
    fn populated() -> crate::SqliteMemory {
        let mut memory = crate::SqliteMemory::open_in_memory(AppPolicy::default()).unwrap();
        let context = crate::AppContext::new("com.example.editor");
        for (source, text) in [
            (MemorySource::Typed, "Inputia"),
            (MemorySource::Typed, "inputiaCore"),
            (MemorySource::Voice, "Inputia"),
            (MemorySource::Clipboard, "inputia_clipboard"),
            (MemorySource::Voice, "inputiaCore"),
            (MemorySource::Typed, "inputiaCore"),
            (MemorySource::Voice, "中文词"),
            (MemorySource::Clipboard, "中文旧词"),
            (MemorySource::Clipboard, "中文词"),
            (MemorySource::Typed, "a%literal"),
            (MemorySource::Typed, "abother"),
        ] {
            memory.learn(source, text, &context).unwrap();
        }
        memory
    }
    #[cfg(feature = "sqlite-memory")]
    #[test]
    fn each_query_matches_previous_full_database_algorithm() {
        let memory = populated();
        for prefix in ["中文", "a%", "", "inputia"] {
            for limit in [1, 3, 128] {
                let query = MemoryQuery::Completion {
                    prefix: prefix.into(),
                    limit,
                };
                let snapshot = memory
                    .query_snapshot(&query)
                    .unwrap()
                    .memory_for(&query, AppPolicy::default())
                    .unwrap();
                assert_eq!(
                    snapshot.completion_candidates(prefix, limit),
                    memory.completion_candidates(prefix, limit).unwrap()
                );
            }
        }
        for prefix in ["in", "IN", " Inputia ", "中文", "i"] {
            for limit in [1, 3, 128] {
                let query = MemoryQuery::EnglishCompletion {
                    prefix: prefix.into(),
                    limit,
                };
                let snapshot = memory
                    .query_snapshot(&query)
                    .unwrap()
                    .memory_for(&query, AppPolicy::default())
                    .unwrap();
                assert_eq!(
                    snapshot.english_completion_candidates(prefix, limit),
                    memory.english_completion_candidates(prefix, limit).unwrap()
                );
            }
        }
        for limit in [1, 3, 128] {
            let query = MemoryQuery::Clipboard { limit };
            let snapshot = memory
                .query_snapshot(&query)
                .unwrap()
                .memory_for(&query, AppPolicy::default())
                .unwrap();
            assert_eq!(
                snapshot.clipboard_candidates(limit),
                memory.clipboard_candidates(limit).unwrap()
            );
            let query = MemoryQuery::VoiceHotwords { limit };
            let snapshot = memory
                .query_snapshot(&query)
                .unwrap()
                .memory_for(&query, AppPolicy::default())
                .unwrap();
            assert_eq!(
                snapshot.voice_hotwords(limit),
                memory.voice_hotwords(limit).unwrap()
            );
        }
    }
    #[cfg(feature = "sqlite-memory")]
    #[test]
    fn exact_candidate_query_retains_rare_terms_outside_global_top_n() {
        let memory = populated();
        for n in 0..1_050 {
            memory
                .conn
                .execute(
                    "INSERT INTO inputia_terms VALUES(?1,100,0,0,100)",
                    [format!("热门{n}")],
                )
                .unwrap();
        }
        let query = MemoryQuery::Rank {
            candidate_texts: vec!["中文词".into(), "未学".into(), "中文词".into()],
        };
        let snapshot = memory.query_snapshot(&query).unwrap();
        assert_eq!(snapshot.terms().len(), 1);
        let candidates = vec![Candidate::new("1", "未学"), Candidate::new("2", "中文词")];
        let local = snapshot.memory_for(&query, AppPolicy::default()).unwrap();
        assert_eq!(
            local.rank_candidates(candidates.clone()),
            memory.rank_candidates(candidates).unwrap()
        );
        let big = MemoryQuery::Rank {
            candidate_texts: (0..1_024).map(|n| format!("热门{n}")).collect(),
        };
        assert_eq!(memory.query_snapshot(&big).unwrap().terms().len(), 1_024);
    }
    #[cfg(feature = "sqlite-memory")]
    #[test]
    fn database_invalid_counts_and_oversized_text_do_not_wrap_or_truncate() {
        let memory = populated();
        memory
            .conn
            .execute(
                "UPDATE inputia_terms SET typed_count=-1 WHERE text='Inputia'",
                [],
            )
            .unwrap();
        let query = MemoryQuery::Rank {
            candidate_texts: vec!["Inputia".into()],
        };
        assert!(matches!(
            memory.query_snapshot(&query),
            Err(SnapshotError::StorageUnavailable)
        ));
        memory
            .conn
            .execute(
                "UPDATE inputia_terms SET typed_count=4294967296 WHERE text='Inputia'",
                [],
            )
            .unwrap();
        assert!(matches!(
            memory.query_snapshot(&query),
            Err(SnapshotError::StorageUnavailable)
        ));
        memory
            .conn
            .execute(
                "INSERT INTO inputia_terms VALUES(?1,10000,0,0,1)",
                [&"x".repeat(MAX_SNAPSHOT_TEXT_BYTES + 1)],
            )
            .unwrap();
        assert!(matches!(
            memory.query_snapshot(&MemoryQuery::Completion {
                prefix: "x".into(),
                limit: 1
            }),
            Err(SnapshotError::BudgetExceeded)
        ));
    }
}
