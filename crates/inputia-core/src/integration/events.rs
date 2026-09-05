//! 用来源库身份及记录版本标识事件，正文相同不代表同一个用户行为。

/// 有界、不含路径或控制符的外部标识。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Identifier(String);

impl Identifier {
    /// 验证传输层提供的标识；禁止将未经验证的值拼进持久化键。
    pub fn parse(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 160
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_:.".contains(&byte))
        {
            return Err("invalid identifier");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 物理来源库和记录组成身份；数据库换代必须使用新的 store_id。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SourceRecord {
    pub store_id: Identifier,
    pub record_id: Identifier,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Voice,
    Clipboard,
    SavedSnippet,
    TypedTerm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventOperation {
    Upsert,
    Delete,
    Forget,
}

/// 内容变更与消费水位分开；序列必须由源数据库事务分配。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventIdentity {
    pub event_id: Identifier,
    pub origin_instance_id: Identifier,
    pub source: SourceRecord,
    pub source_revision: u64,
    pub source_sequence: u64,
    pub policy_epoch: u64,
    pub correlation_id: Option<Identifier>,
}

impl EventIdentity {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.source_revision == 0 || self.source_sequence == 0 {
            return Err("revision and sequence must be positive");
        }
        Ok(())
    }
}

/// 消费端不可越过缺口；游标只在应用变更成功的事务中推进。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequenceDecision {
    Replay,
    Next,
    Gap,
}

pub fn check_sequence(last_applied: u64, incoming: u64) -> SequenceDecision {
    if incoming <= last_applied {
        SequenceDecision::Replay
    } else if last_applied.checked_add(1) == Some(incoming) {
        SequenceDecision::Next
    } else {
        SequenceDecision::Gap
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_cannot_smuggle_paths_or_control_characters() {
        for value in ["", "a/b", "x\ny", "abc\0", "项目名称", "a b"] {
            assert!(Identifier::parse(value).is_err());
        }
        assert!(Identifier::parse("x".repeat(161)).is_err());
        assert!(Identifier::parse("profile-1:record_99.v2").is_ok());
    }

    #[test]
    fn source_identity_survives_equal_numeric_ids_in_different_stores() {
        let first = SourceRecord {
            store_id: Identifier::parse("store-a").unwrap(),
            record_id: Identifier::parse("1").unwrap(),
        };
        let second = SourceRecord {
            store_id: Identifier::parse("store-b").unwrap(),
            record_id: Identifier::parse("1").unwrap(),
        };
        assert_ne!(first, second);
    }

    #[test]
    fn missing_event_cannot_advance_cursor_and_maximum_does_not_wrap() {
        assert_eq!(check_sequence(3, 3), SequenceDecision::Replay);
        assert_eq!(check_sequence(3, 2), SequenceDecision::Replay);
        assert_eq!(check_sequence(3, 4), SequenceDecision::Next);
        assert_eq!(check_sequence(3, 5), SequenceDecision::Gap);
        assert_eq!(check_sequence(u64::MAX, 0), SequenceDecision::Replay);
    }
}
