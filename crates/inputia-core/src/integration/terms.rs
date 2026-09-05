//! 学习词只用作有预算的识别提示，不自动进入模糊替换。

use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermEvidence {
    ExplicitUserTerm,
    ConfirmedCorrection,
    SelectedTypedTerm { uses: u32, sessions: u32 },
    UnconfirmedVoice,
    UnconfirmedClipboard,
}

impl TermEvidence {
    fn priority(self) -> u8 {
        match self {
            Self::ExplicitUserTerm => 0,
            Self::ConfirmedCorrection => 1,
            _ => 2,
        }
    }

    pub fn is_eligible(self) -> bool {
        match self {
            Self::ExplicitUserTerm | Self::ConfirmedCorrection => true,
            Self::SelectedTypedTerm { uses, sessions } => uses >= 3 && sessions >= 2,
            Self::UnconfirmedVoice | Self::UnconfirmedClipboard => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermRejection {
    Unconfirmed,
    InvalidLength,
    ControlToken,
    SensitivePattern,
    SentenceOrPath,
}

fn contains_control_token(text: &str) -> bool {
    text.chars().any(char::is_control) || text.contains("<|") || text.contains("|>")
}

/// 审慎识别可学习的短词组；这不是通用秘密检测器，来源策略仍须单独执行。
pub fn validate_term(text: &str, evidence: TermEvidence) -> Result<String, TermRejection> {
    if !evidence.is_eligible() {
        return Err(TermRejection::Unconfirmed);
    }
    if contains_control_token(text) {
        return Err(TermRejection::ControlToken);
    }
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars = normalized.chars().count();
    if !(2..=32).contains(&chars) {
        return Err(TermRejection::InvalidLength);
    }
    let lower = normalized.to_ascii_lowercase();
    if normalized
        .chars()
        .all(|c| c.is_numeric() || c == ' ' || c == '-')
        || normalized.contains('@')
        || [
            "sk-",
            "sk_",
            "bearer ",
            "password=",
            "token=",
            "api_key=",
            "eyj",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
        || (chars >= 24
            && normalized.is_ascii()
            && !normalized.contains(' ')
            && normalized.bytes().any(|c| c.is_ascii_digit())
            && normalized.bytes().any(|c| c.is_ascii_alphabetic()))
    {
        return Err(TermRejection::SensitivePattern);
    }
    if normalized.contains([
        '/', '\\', '\n', '。', '！', '？', '，', ';', '；', ':', '?', '!',
    ]) || normalized.split_whitespace().count() > 4
    {
        return Err(TermRejection::SentenceOrPath);
    }
    Ok(normalized)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HotwordBudget {
    pub max_words: usize,
    pub max_bytes: usize,
    pub learned_max_words: usize,
    pub learned_max_bytes: usize,
}

impl Default for HotwordBudget {
    fn default() -> Self {
        Self {
            max_words: 256,
            max_bytes: 16 * 1024,
            learned_max_words: 64,
            learned_max_bytes: 4 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotwordError {
    ExplicitTermsExceedModelBudget,
    InvalidExplicitTerm,
}

/// 生成会话快照；显式词超出模型预算时返回错误，不静默丢弃。
pub fn build_hotwords(
    explicit: &[String],
    candidates: &[(String, TermEvidence)],
    budget: HotwordBudget,
) -> Result<Vec<String>, HotwordError> {
    let mut words = Vec::new();
    let mut seen = HashSet::new();
    let mut bytes = 0usize;
    for term in explicit {
        if contains_control_token(term) {
            return Err(HotwordError::InvalidExplicitTerm);
        }
        let term = term.trim();
        if term.is_empty() || !seen.insert(term.to_owned()) {
            continue;
        }
        // 手工词的既有格式策略由模型 adapter 保留；预算在此统一检查。
        bytes = bytes.saturating_add(term.len());
        if words.len() >= budget.max_words || bytes > budget.max_bytes {
            return Err(HotwordError::ExplicitTermsExceedModelBudget);
        }
        words.push(term.to_owned());
    }
    let mut ordered = candidates.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(_, evidence)| evidence.priority());
    let mut learned_count = 0;
    let mut learned_bytes = 0usize;
    for (text, evidence) in ordered {
        let Ok(term) = validate_term(text, *evidence) else {
            continue;
        };
        if seen.contains(&term)
            || words.len() >= budget.max_words
            || learned_count >= budget.learned_max_words
            || bytes.saturating_add(term.len()) > budget.max_bytes
            || learned_bytes.saturating_add(term.len()) > budget.learned_max_bytes
        {
            continue;
        }
        learned_count += 1;
        learned_bytes += term.len();
        bytes += term.len();
        seen.insert(term.clone());
        words.push(term);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_terms_cannot_bypass_model_control_token_checks() {
        for value in ["<|im_start|>", "a\nb", "term\0", "\nInputia"] {
            assert_eq!(
                build_hotwords(&[value.into()], &[], HotwordBudget::default()),
                Err(HotwordError::InvalidExplicitTerm)
            );
        }
        assert_eq!(
            build_hotwords(&[" a ".into()], &[], HotwordBudget::default()).unwrap(),
            vec!["a"]
        );
    }

    #[test]
    fn raw_asr_and_clipboard_are_not_automatically_promoted() {
        for source in [
            TermEvidence::UnconfirmedVoice,
            TermEvidence::UnconfirmedClipboard,
        ] {
            assert_eq!(
                validate_term("Inputia", source),
                Err(TermRejection::Unconfirmed)
            );
        }
        assert!(validate_term(
            "Inputia",
            TermEvidence::SelectedTypedTerm {
                uses: 99,
                sessions: 1
            }
        )
        .is_err());
        assert!(validate_term(
            "Inputia",
            TermEvidence::SelectedTypedTerm {
                uses: 3,
                sessions: 2
            }
        )
        .is_ok());
    }

    #[test]
    fn learned_candidates_reject_paragraphs_paths_secrets_and_control_markers() {
        for text in [
            "这是一个很长的段落。不能直接作为热词",
            "/Users/example/file.txt",
            "sk-testsecret",
            "123456",
            "name@example.com",
            "<|im_start|>",
            "line\nbreak",
            "abcd1234abcd1234abcd1234abcd1234",
            "a",
            "词",
        ] {
            assert!(
                validate_term(text, TermEvidence::ConfirmedCorrection).is_err(),
                "unexpected accepted fixture"
            );
        }
        assert!(validate_term("语言模型", TermEvidence::ConfirmedCorrection).is_ok());
        assert!(validate_term("Node.js", TermEvidence::ConfirmedCorrection).is_ok());
    }

    #[test]
    fn explicit_terms_win_and_unconfirmed_sources_do_not_consume_budget() {
        let words = build_hotwords(
            &["Inputia".into()],
            &[
                ("错误转写".into(), TermEvidence::UnconfirmedVoice),
                ("Inputia".into(), TermEvidence::ConfirmedCorrection),
                (
                    "常用词".into(),
                    TermEvidence::SelectedTypedTerm {
                        uses: 3,
                        sessions: 2,
                    },
                ),
                ("确认词".into(), TermEvidence::ConfirmedCorrection),
            ],
            HotwordBudget {
                max_words: 2,
                ..HotwordBudget::default()
            },
        )
        .unwrap();
        assert_eq!(words, vec!["Inputia", "确认词"]);
    }

    #[test]
    fn unicode_byte_budget_and_explicit_overflow_are_visible() {
        assert_eq!(
            build_hotwords(
                &["中文术语".into()],
                &[],
                HotwordBudget {
                    max_bytes: 5,
                    ..HotwordBudget::default()
                }
            ),
            Err(HotwordError::ExplicitTermsExceedModelBudget)
        );
        let words = build_hotwords(
            &[],
            &[("中文术语".into(), TermEvidence::ConfirmedCorrection)],
            HotwordBudget {
                learned_max_bytes: 5,
                ..HotwordBudget::default()
            },
        )
        .unwrap();
        assert!(words.is_empty());
    }
}
