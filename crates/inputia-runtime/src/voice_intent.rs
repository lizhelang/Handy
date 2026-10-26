//! 语音自动触发的确定性预筛。
//!
//! 当前 Inputia 没有自动委派产品入口，因此本模块只提供可测试的第一层
//! 分类，不触发网络、不发送文本、不改变转写结果。未来若接入本地判断模型，
//! 只能处理本模块无法确定的边界样本，并必须保留取消和低置信回退。

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VoiceIntent {
    Question,
    Request,
    Statement,
    Cancellation,
    Incomplete,
}

pub fn classify(text: &str) -> VoiceIntent {
    let value = text.trim();
    if value.is_empty() {
        return VoiceIntent::Incomplete;
    }
    let normalized = value.to_lowercase();
    if [
        "取消",
        "算了",
        "别做了",
        "停止",
        "cancel",
        "never mind",
        "stop",
    ]
    .iter()
    .any(|marker| normalized == *marker || normalized.starts_with(&format!("{marker} ")))
    {
        return VoiceIntent::Cancellation;
    }
    if value.ends_with(['？', '?'])
        || ["吗", "呢", "怎么", "为什么", "是否", "能不能"]
            .iter()
            .any(|marker| value.starts_with(marker))
    {
        return VoiceIntent::Question;
    }
    if [
        "请",
        "帮我",
        "需要",
        "请帮我",
        "please",
        "can you",
        "could you",
    ]
    .iter()
    .any(|marker| normalized.starts_with(marker))
    {
        return VoiceIntent::Request;
    }
    if value.ends_with(['，', ',', '、', ':', '：']) {
        return VoiceIntent::Incomplete;
    }
    VoiceIntent::Statement
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_safe_question_and_request_markers() {
        assert_eq!(classify("这个方案可以吗？"), VoiceIntent::Question);
        assert_eq!(classify("请帮我整理这段话"), VoiceIntent::Request);
        assert_eq!(classify("Please summarize this"), VoiceIntent::Request);
    }

    #[test]
    fn cancellation_wins_before_question_or_request() {
        assert_eq!(classify("取消"), VoiceIntent::Cancellation);
        assert_eq!(classify("stop"), VoiceIntent::Cancellation);
    }

    #[test]
    fn incomplete_and_statement_are_conservative() {
        assert_eq!(classify(""), VoiceIntent::Incomplete);
        assert_eq!(classify("先说一下，"), VoiceIntent::Incomplete);
        assert_eq!(classify("今天下午开会。"), VoiceIntent::Statement);
    }
}
