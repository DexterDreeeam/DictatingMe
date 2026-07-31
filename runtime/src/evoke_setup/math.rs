//! 共用数学工具：向量相似度、归一化、激活函数。

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let numerator = dot(left, right);
    let left_norm = dot(left, left).sqrt();
    let right_norm = dot(right, right).sqrt();
    if left_norm <= f32::EPSILON || right_norm <= f32::EPSILON {
        0.0
    } else {
        (numerator / (left_norm * right_norm)).clamp(-1.0, 1.0)
    }
}

pub(crate) fn cosine_distance(left: &[f32], right: &[f32]) -> f32 {
    (1.0 - cosine_similarity(left, right)).clamp(0.0, 2.0)
}

pub(crate) fn normalize_in_place(values: &mut [f32]) {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for value in values {
            *value /= norm;
        }
    }
}

pub(crate) fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

pub(crate) fn sigmoid(value: f32) -> f32 {
    if value >= 0.0 {
        1.0 / (1.0 + (-value).exp())
    } else {
        let exp = value.exp();
        exp / (1.0 + exp)
    }
}
