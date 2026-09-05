//! 保留、学习、远程使用和输出分别授权；输入目标不能冒充剪贴板来源。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryMode {
    Normal,
    Strict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceTrust {
    Verified,
    Observed,
    Unknown,
}

/// 由平台采集器判断的上下文；未知值必须显式保留。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrivacyContext {
    pub source_trust: SourceTrust,
    pub source_sensitive: bool,
    pub target_known: bool,
    pub target_sensitive: bool,
    pub secure_input: bool,
    pub transient_or_concealed: bool,
}

impl PrivacyContext {
    pub fn target_allows_personalization(&self) -> bool {
        self.target_known && !self.target_sensitive && !self.secure_input
    }

    fn blocks_capture(&self) -> bool {
        self.source_sensitive
            || self.target_sensitive
            || self.secure_input
            || self.transient_or_concealed
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivacyPolicy {
    pub epoch: u64,
    pub history_enabled: bool,
    pub history_mode: HistoryMode,
    pub learning_enabled: bool,
    pub remote_learning_terms_enabled: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrivacyDecision {
    pub capture: bool,
    pub retain: bool,
    pub learn: bool,
    pub remote_use: bool,
    pub personalized_read: bool,
    pub output: bool,
}

impl PrivacyPolicy {
    /// 过期策略不允许新增副作用；客户端必须先同步撤销屏障。
    pub fn decide(&self, event_epoch: u64, context: PrivacyContext) -> PrivacyDecision {
        if event_epoch != self.epoch {
            return PrivacyDecision::default();
        }
        let target_allowed = context.target_allows_personalization();
        let capture = self.history_enabled
            && !context.blocks_capture()
            && (self.history_mode == HistoryMode::Normal
                || context.source_trust == SourceTrust::Verified);
        let learn = self.learning_enabled
            && target_allowed
            && !context.blocks_capture()
            && context.source_trust == SourceTrust::Verified;
        PrivacyDecision {
            capture,
            retain: capture,
            learn,
            remote_use: learn && self.remote_learning_terms_enabled,
            personalized_read: target_allowed,
            output: target_allowed,
        }
    }
}

/// 只接受服务端签发版本，使用调用端单调时钟的短期租约。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotLease {
    pub policy_epoch: u64,
    pub received_at_ms: u64,
}

impl SnapshotLease {
    pub const TTL_MS: u64 = 2_000;

    pub fn is_valid(&self, current_epoch: u64, now_ms: u64) -> bool {
        current_epoch == self.policy_epoch
            && now_ms
                .checked_sub(self.received_at_ms)
                .is_some_and(|age| age < Self::TTL_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> PrivacyPolicy {
        PrivacyPolicy {
            epoch: 7,
            history_enabled: true,
            history_mode: HistoryMode::Normal,
            learning_enabled: true,
            remote_learning_terms_enabled: false,
        }
    }

    fn context() -> PrivacyContext {
        PrivacyContext {
            source_trust: SourceTrust::Verified,
            source_sensitive: false,
            target_known: true,
            target_sensitive: false,
            secure_input: false,
            transient_or_concealed: false,
        }
    }

    #[test]
    fn unknown_copy_remains_usable_but_is_never_silently_learned() {
        let unknown = PrivacyContext {
            source_trust: SourceTrust::Unknown,
            ..context()
        };
        let normal = policy().decide(7, unknown);
        assert!(normal.retain);
        assert!(!normal.learn);
        assert!(!normal.remote_use);
        let strict = PrivacyPolicy {
            history_mode: HistoryMode::Strict,
            ..policy()
        };
        assert!(!strict.decide(7, unknown).retain);
    }

    #[test]
    fn foreground_observation_does_not_prove_source() {
        let observed = PrivacyContext {
            source_trust: SourceTrust::Observed,
            ..context()
        };
        assert!(!policy().decide(7, observed).learn);
    }

    #[test]
    fn sensitive_context_blocks_persistence_and_personalized_output() {
        let sensitive = PrivacyContext {
            target_sensitive: true,
            ..context()
        };
        assert_eq!(policy().decide(7, sensitive), PrivacyDecision::default());
        let concealed = PrivacyContext {
            transient_or_concealed: true,
            ..context()
        };
        let decision = policy().decide(7, concealed);
        assert!(!decision.capture && !decision.learn && !decision.remote_use);
    }

    #[test]
    fn pause_learning_does_not_disable_history_and_remote_is_separate() {
        let paused = PrivacyPolicy {
            learning_enabled: false,
            ..policy()
        };
        assert!(paused.decide(7, context()).retain);
        assert!(!paused.decide(7, context()).learn);
        let allowed = policy().decide(7, context());
        assert!(allowed.learn);
        assert!(!allowed.remote_use);
        assert_eq!(policy().decide(6, context()), PrivacyDecision::default());
    }

    #[test]
    fn unknown_target_cannot_read_personalized_history_or_insert() {
        let unknown = PrivacyContext {
            target_known: false,
            ..context()
        };
        let decision = policy().decide(7, unknown);
        assert!(!decision.personalized_read && !decision.learn && !decision.output);
    }

    #[test]
    fn offline_lease_expires_and_policy_revocation_invalidates_immediately() {
        let lease = SnapshotLease {
            policy_epoch: 7,
            received_at_ms: 100,
        };
        assert!(lease.is_valid(7, 2_099));
        assert!(!lease.is_valid(7, 2_100));
        assert!(!lease.is_valid(8, 101));
        assert!(!lease.is_valid(7, 99));
    }
}
