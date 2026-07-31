//! 兼容门面。
//!
//! 实现已按职责拆分到 `audio`、`spectral`、`math` 与 `modes`。
//! 这里保留原有的导出路径，使既有调用方（`settings.rs`、`scoring`、集成测试）
//! 无需改动 import。新代码请直接引用拆分后的模块。

pub use super::audio::{frames_to_16k, read_wav_16k, recording_quality, write_wav_16k};
pub use super::math::cosine_similarity;
pub use super::modes::classifier::{
    classifier_score, feature_sequence as extract_feature_sequence, summarize_sequence,
    train_logistic,
};
pub use super::modes::speaker::average_embeddings;
pub use super::modes::voice_match::{
    average_sequences, dtw_similarity, resample_sequence, TEMPLATE_FRAMES,
};
pub use super::spectral::FEATURE_DIM;
