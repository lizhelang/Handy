//! 知识库本地 embedding 的数据合同。
//!
//! P6 先冻结向量身份、维度和有限数值校验；typed-decision 模型不能复用为
//! embedding 模型。实际 BGE/MLX/llama.cpp worker 后续接入此合同。

use serde::{Deserialize, Serialize};

pub const MAX_DIMENSIONS: usize = 4096;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingVector {
    pub model_id: String,
    pub model_revision: String,
    pub dimensions: usize,
    pub values: Vec<f32>,
}

impl EmbeddingVector {
    pub fn validate(&self) -> Result<(), String> {
        if self.model_id.is_empty()
            || self.model_revision.is_empty()
            || self.dimensions == 0
            || self.dimensions > MAX_DIMENSIONS
            || self.values.len() != self.dimensions
            || !self.values.iter().all(|value| value.is_finite())
        {
            return Err("embedding_vector_invalid".into());
        }
        if self.values.iter().all(|value| *value == 0.0) {
            return Err("embedding_vector_zero".into());
        }
        Ok(())
    }
}

pub fn cosine_similarity(left: &EmbeddingVector, right: &EmbeddingVector) -> Result<f32, String> {
    left.validate()?;
    right.validate()?;
    if left.model_id != right.model_id
        || left.model_revision != right.model_revision
        || left.dimensions != right.dimensions
    {
        return Err("embedding_identity_mismatch".into());
    }
    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;
    for (a, b) in left.values.iter().zip(&right.values) {
        dot += a * b;
        left_norm += a * a;
        right_norm += b * b;
    }
    let denominator = left_norm.sqrt() * right_norm.sqrt();
    if !denominator.is_finite() || denominator == 0.0 {
        return Err("embedding_norm_invalid".into());
    }
    Ok((dot / denominator).clamp(-1.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector(revision: &str, values: &[f32]) -> EmbeddingVector {
        EmbeddingVector {
            model_id: "bge-m3-mlx".into(),
            model_revision: revision.into(),
            dimensions: values.len(),
            values: values.to_vec(),
        }
    }

    #[test]
    fn cosine_requires_same_model_identity() {
        let left = vector("r1", &[1.0, 0.0]);
        let right = vector("r2", &[1.0, 0.0]);
        assert_eq!(
            cosine_similarity(&left, &right).unwrap_err(),
            "embedding_identity_mismatch"
        );
    }

    #[test]
    fn cosine_is_bounded_and_deterministic() {
        let left = vector("r1", &[1.0, 0.0]);
        let right = vector("r1", &[1.0, 1.0]);
        let value = cosine_similarity(&left, &right).unwrap();
        assert!((value - 0.70710677).abs() < 1e-5);
        assert!((-1.0..=1.0).contains(&value));
    }

    #[test]
    fn rejects_nan_and_zero_vectors() {
        assert_eq!(
            vector("r1", &[0.0, 0.0]).validate().unwrap_err(),
            "embedding_vector_zero"
        );
        assert_eq!(
            vector("r1", &[f32::NAN]).validate().unwrap_err(),
            "embedding_vector_invalid"
        );
    }
}
