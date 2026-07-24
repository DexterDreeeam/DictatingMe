//! Models 模块：EvokeModel Engine + DictationModel Engine（见 brainstrom/plan.md §3.1、§5）。

pub mod onnx_session;
pub mod evoke_model;
pub mod dictation_model;

pub use onnx_session::{ModelError, OnnxSession};
pub use evoke_model::{EvokeModelEngine, WakeWordDetection};
pub use dictation_model::{DictationModelEngine, LoadState, TranscriptionUpdate};
