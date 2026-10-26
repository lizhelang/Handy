//! 本地类 JEV 判断协议与边界校验。
//!
//! 这一层只定义有界 `choice` / `noul` / `score` 判断，不负责启动具体模型，
//! 也不允许模型结果直接修改输入、知识库或权限状态。MLX worker 和未来的
//! CUDA/其他本地 backend 都必须遵守同一份合同。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_STATE_BYTES: usize = 512 * 1024;
pub const MAX_QUESTIONS: usize = 32;
pub const MAX_CHOICES: usize = 64;
pub const MAX_TEXT_CHARS: usize = 4_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalTextClass {
    Normal,
    Code,
    Noise,
    Sensitive,
}

/// 保守的本地预筛：只做明显模式识别，不能替代用户授权或完整 DLP。
/// Sensitive/Noise 默认不进入自动学习；模型不可用时仍使用此规则结果。
pub fn classify_text_safety(text: &str) -> LocalTextClass {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().count() < 2 {
        return LocalTextClass::Noise;
    }
    let lower = trimmed.to_ascii_lowercase();
    let sensitive_markers = [
        "password=",
        "passwd=",
        "token=",
        "api_key=",
        "apikey=",
        "secret=",
        "authorization: bearer",
        "-----begin ",
        "sk-",
    ];
    if sensitive_markers
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return LocalTextClass::Sensitive;
    }
    let code_markers = ["fn ", "def ", "import ", "select ", "```", "{", "};"];
    if code_markers.iter().any(|marker| lower.contains(marker)) {
        return LocalTextClass::Code;
    }
    let useful = trimmed
        .chars()
        .filter(|character| character.is_alphanumeric())
        .count();
    if useful < 2 {
        LocalTextClass::Noise
    } else {
        LocalTextClass::Normal
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DecisionRequest {
    pub protocol_version: u32,
    pub request_id: String,
    pub model_id: String,
    pub state: Value,
    pub questions: BTreeMap<String, DecisionQuestion>,
    #[serde(default)]
    pub limits: DecisionLimits,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DecisionLimits {
    #[serde(default = "default_deadline_ms")]
    pub deadline_ms: u64,
    #[serde(default = "default_max_input_chars")]
    pub max_input_chars: usize,
}

impl Default for DecisionLimits {
    fn default() -> Self {
        Self {
            deadline_ms: default_deadline_ms(),
            max_input_chars: default_max_input_chars(),
        }
    }
}

fn default_deadline_ms() -> u64 {
    250
}

fn default_max_input_chars() -> usize {
    MAX_TEXT_CHARS
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionQuestion {
    Noul {
        instructions: String,
    },
    Choice {
        instructions: String,
        criteria: BTreeMap<String, String>,
    },
    Score {
        instructions: String,
        criteria: Vec<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DecisionResponse {
    pub protocol_version: u32,
    pub request_id: String,
    pub model_id: String,
    #[serde(default)]
    pub model_revision: Option<String>,
    pub answers: BTreeMap<String, DecisionAnswer>,
    #[serde(default)]
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DecisionAnswer {
    Noul {
        probability: f32,
        confidence: f32,
        #[serde(default)]
        abstained: bool,
    },
    Choice {
        selected: String,
        probabilities: BTreeMap<String, f32>,
        confidence: f32,
        #[serde(default)]
        abstained: bool,
    },
    Score {
        selected: usize,
        probabilities: Vec<f32>,
        confidence: f32,
        #[serde(default)]
        abstained: bool,
    },
}

pub fn validate_request(request: &DecisionRequest, encoded_bytes: usize) -> Result<(), String> {
    if encoded_bytes > MAX_REQUEST_BYTES {
        return Err("decision_request_too_large".into());
    }
    if request.protocol_version != PROTOCOL_VERSION {
        return Err("decision_protocol_version".into());
    }
    valid_id(&request.request_id, 128, "request_id")?;
    valid_id(&request.model_id, 256, "model_id")?;
    let state_bytes =
        serde_json::to_vec(&request.state).map_err(|_| "decision_state_encode".to_string())?;
    if state_bytes.len() > MAX_STATE_BYTES {
        return Err("decision_state_too_large".into());
    }
    if request.questions.is_empty() || request.questions.len() > MAX_QUESTIONS {
        return Err("decision_question_count".into());
    }
    if !(1..=2_000).contains(&request.limits.deadline_ms) {
        return Err("decision_deadline".into());
    }
    if !(1..=MAX_TEXT_CHARS).contains(&request.limits.max_input_chars) {
        return Err("decision_input_limit".into());
    }
    for (name, question) in &request.questions {
        valid_id(name, 128, "question_id")?;
        validate_question(question)?;
    }
    Ok(())
}

pub fn validate_response(
    request: &DecisionRequest,
    response: &DecisionResponse,
) -> Result<(), String> {
    if response.protocol_version != PROTOCOL_VERSION {
        return Err("decision_response_protocol_version".into());
    }
    if response.request_id != request.request_id {
        return Err("decision_response_request_id".into());
    }
    valid_id(&response.model_id, 256, "response_model_id")?;
    if response.answers.len() != request.questions.len()
        || response
            .answers
            .keys()
            .any(|key| !request.questions.contains_key(key))
    {
        return Err("decision_response_questions".into());
    }
    for (name, question) in &request.questions {
        let answer = response
            .answers
            .get(name)
            .ok_or_else(|| "decision_response_missing_answer".to_string())?;
        validate_answer(question, answer)?;
    }
    Ok(())
}

fn validate_question(question: &DecisionQuestion) -> Result<(), String> {
    match question {
        DecisionQuestion::Noul { instructions } => validate_text(instructions, "instructions"),
        DecisionQuestion::Choice {
            instructions,
            criteria,
        } => {
            validate_text(instructions, "instructions")?;
            if criteria.is_empty() || criteria.len() > MAX_CHOICES {
                return Err("decision_choice_count".into());
            }
            for (key, value) in criteria {
                valid_id(key, 128, "choice_id")?;
                validate_text(value, "choice_description")?;
            }
            Ok(())
        }
        DecisionQuestion::Score {
            instructions,
            criteria,
        } => {
            validate_text(instructions, "instructions")?;
            if criteria.len() < 2 || criteria.len() > MAX_CHOICES {
                return Err("decision_score_count".into());
            }
            criteria
                .iter()
                .try_for_each(|value| validate_text(value, "score_description"))
        }
    }
}

fn validate_answer(question: &DecisionQuestion, answer: &DecisionAnswer) -> Result<(), String> {
    match (question, answer) {
        (
            DecisionQuestion::Noul { .. },
            DecisionAnswer::Noul {
                probability,
                confidence,
                ..
            },
        ) => {
            probability_range(*probability)?;
            probability_range(*confidence)
        }
        (
            DecisionQuestion::Choice { criteria, .. },
            DecisionAnswer::Choice {
                selected,
                probabilities,
                confidence,
                ..
            },
        ) => {
            if !criteria.contains_key(selected) || probabilities.len() != criteria.len() {
                return Err("decision_choice_answer".into());
            }
            for key in criteria.keys() {
                let value = probabilities
                    .get(key)
                    .ok_or_else(|| "decision_choice_probability".to_string())?;
                probability_range(*value)?;
            }
            probability_range(*confidence)
        }
        (
            DecisionQuestion::Score { criteria, .. },
            DecisionAnswer::Score {
                selected,
                probabilities,
                confidence,
                ..
            },
        ) => {
            if *selected >= criteria.len() || probabilities.len() != criteria.len() {
                return Err("decision_score_answer".into());
            }
            probabilities
                .iter()
                .try_for_each(|value| probability_range(*value))?;
            probability_range(*confidence)
        }
        _ => Err("decision_answer_type".into()),
    }
}

fn valid_id(value: &str, max: usize, field: &str) -> Result<(), String> {
    if value.is_empty() || value.chars().count() > max || value.chars().any(char::is_control) {
        return Err(format!("decision_{field}"));
    }
    Ok(())
}

fn validate_text(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty()
        || value.chars().count() > MAX_TEXT_CHARS
        || value.chars().any(char::is_control)
    {
        return Err(format!("decision_{field}"));
    }
    Ok(())
}

fn probability_range(value: f32) -> Result<(), String> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err("decision_probability".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(question: DecisionQuestion) -> DecisionRequest {
        DecisionRequest {
            protocol_version: PROTOCOL_VERSION,
            request_id: "r1".into(),
            model_id: "laya-multilingual-mlx".into(),
            state: serde_json::json!({"text":"测试"}),
            questions: [("q".into(), question)].into_iter().collect(),
            limits: DecisionLimits::default(),
        }
    }

    #[test]
    fn rejects_unbounded_or_controlled_requests() {
        let mut req = request(DecisionQuestion::Noul {
            instructions: "是否相关".into(),
        });
        req.request_id = "bad\n".into();
        assert_eq!(
            validate_request(&req, 10).unwrap_err(),
            "decision_request_id"
        );
        req.request_id = "r1".into();
        req.limits.deadline_ms = 0;
        assert_eq!(validate_request(&req, 10).unwrap_err(), "decision_deadline");
    }

    #[test]
    fn validates_choice_identity_and_probabilities() {
        let req = request(DecisionQuestion::Choice {
            instructions: "选择类别".into(),
            criteria: [("a".into(), "A".into()), ("b".into(), "B".into())]
                .into_iter()
                .collect(),
        });
        validate_request(&req, 10).unwrap();
        let response = DecisionResponse {
            protocol_version: PROTOCOL_VERSION,
            request_id: "r1".into(),
            model_id: "laya-multilingual-mlx".into(),
            model_revision: Some("sha256:test".into()),
            answers: [(
                "q".into(),
                DecisionAnswer::Choice {
                    selected: "a".into(),
                    probabilities: [("a".into(), 0.8), ("b".into(), 0.2)].into_iter().collect(),
                    confidence: 0.8,
                    abstained: false,
                },
            )]
            .into_iter()
            .collect(),
            elapsed_ms: 3,
        };
        validate_response(&req, &response).unwrap();
    }

    #[test]
    fn rejects_wrong_answer_shape_and_nan() {
        let req = request(DecisionQuestion::Noul {
            instructions: "是否相关".into(),
        });
        let response = DecisionResponse {
            protocol_version: PROTOCOL_VERSION,
            request_id: "r1".into(),
            model_id: "laya-multilingual-mlx".into(),
            model_revision: None,
            answers: [(
                "q".into(),
                DecisionAnswer::Noul {
                    probability: f32::NAN,
                    confidence: 0.5,
                    abstained: false,
                },
            )]
            .into_iter()
            .collect(),
            elapsed_ms: 1,
        };
        assert_eq!(
            validate_response(&req, &response).unwrap_err(),
            "decision_probability"
        );
    }

    #[test]
    fn text_safety_gate_fails_closed_on_obvious_secrets() {
        assert_eq!(
            classify_text_safety("token=example"),
            LocalTextClass::Sensitive
        );
        assert_eq!(classify_text_safety("fn main() {}"), LocalTextClass::Code);
        assert_eq!(classify_text_safety("!"), LocalTextClass::Noise);
        assert_eq!(
            classify_text_safety("项目接口需要重试"),
            LocalTextClass::Normal
        );
    }
}
