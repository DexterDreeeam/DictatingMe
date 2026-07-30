//! Models 模块：EvokeModel Engine + DictationModel Engine（见 brainstrom/plan.md §3.1、§5）。

pub mod dictation;
pub mod evoke_model;
pub mod onnx_session;

pub use dictation::{DictationModelEngine, DictationModelSpec, LoadState, TranscriptionUpdate};
pub use evoke_model::{EvokeModelEngine, WakeWordDetection};
pub use onnx_session::{ModelError, OnnxSession};
