use std::collections::HashSet;

const QWEN_CONTEXT_PREFIX: &str = "术语参考（仅作转写提示，未说勿写）：";
const QWEN_CONTEXT_ECHO_MIN_CHARS: usize = 16;

fn normalize_custom_words<F>(custom_words: &[String], should_reject: F) -> Vec<String>
where
    F: Fn(&str) -> bool,
{
    let mut seen = HashSet::new();
    custom_words
        .iter()
        .filter_map(|word| {
            let word = word.trim();
            if word.is_empty() || word.chars().any(char::is_control) || should_reject(word) {
                return None;
            }

            let owned = word.to_string();
            seen.insert(owned.clone()).then_some(owned)
        })
        .collect()
}

/// Qwen3-ASR 的全局词表上下文。词表是识别提示而非强制词典：模型只应在
/// 音频明确说出术语时按原样转写，不能用它补造、插入或替换未说出的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QwenContextPlan {
    context: Option<String>,
    word_count: usize,
}

impl QwenContextPlan {
    pub(crate) fn from_custom_words(custom_words: &[String]) -> Self {
        // Qwen chat/audio 控制标记必须由 transcribe-cpp 的模板层拥有；即使
        // 用户词表里出现形似标记的文本，也不能让它进入 context。
        let words = normalize_custom_words(custom_words, |word| {
            word.contains("<|") || word.contains("|>")
        });

        let context =
            (!words.is_empty()).then(|| format!("{QWEN_CONTEXT_PREFIX}{}", words.join("、")));

        Self {
            context,
            word_count: words.len(),
        }
    }

    pub(crate) fn context(&self) -> Option<&str> {
        self.context.as_deref()
    }

    pub(crate) fn word_count(&self) -> usize {
        self.word_count
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.context.is_none()
    }

    /// 删除模型在转写末尾复述的本次 Qwen context。只匹配完整 context 或
    /// 至少 16 个字符的 context 前缀，不会触及单独出现的术语本身。
    pub(crate) fn strip_trailing_echo(&self, text: &str) -> Option<String> {
        let context = self.context()?;
        let trimmed = text.trim_end();

        if trimmed.ends_with(context) {
            let prefix_len = trimmed.len() - context.len();
            return Some(trimmed[..prefix_len].trim_end().to_string());
        }

        let context_len = context.chars().count();
        for length in (QWEN_CONTEXT_ECHO_MIN_CHARS..context_len).rev() {
            let prefix = context.chars().take(length).collect::<String>();
            if trimmed.ends_with(&prefix) {
                let prefix_len = trimmed.len() - prefix.len();
                return Some(trimmed[..prefix_len].trim_end().to_string());
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::QwenContextPlan;

    #[test]
    fn qwen_context_uses_all_clean_words_without_a_fingerprint() {
        let plan = QwenContextPlan::from_custom_words(&[
            " Inputia ".to_string(),
            "罗泽群".to_string(),
            "Inputia".to_string(),
            "<|im_start|>".to_string(),
            "bad\nentry".to_string(),
        ]);

        assert_eq!(plan.word_count(), 2);
        assert_eq!(
            plan.context().as_deref(),
            Some("术语参考（仅作转写提示，未说勿写）：Inputia、罗泽群")
        );
    }

    #[test]
    fn qwen_context_keeps_all_sixty_five_valid_global_words() {
        let words = (0..65)
            .map(|index| format!("术语{index}"))
            .collect::<Vec<_>>();
        let plan = QwenContextPlan::from_custom_words(&words);

        assert_eq!(plan.word_count(), 65);
        let context = plan
            .context()
            .expect("valid word list should create context");
        assert!(context.contains("术语0"));
        assert!(context.contains("术语64"));
    }
}
