pub mod audio;
pub mod features;
pub mod math;
pub mod modes;
pub mod spectral;
pub mod types;

/// 兼容别名：实现已迁到 `modes`。
pub mod processor {
    pub use super::modes::{
        processor_for, EvokeModeProcessor, ProcessInput,
    };
    pub use super::modes::speaker::{extract_speaker_embedding, speaker_similarity};
}

pub use modes::{processor_for, EvokeModeProcessor, ProcessInput};
pub use types::{
    EnrollmentPlan, EvokeArtifact, EvokeMode, EvokeProfile, EvokeProfileSummary, EvokeSetupPhase,
    EvokeSetupSession, RecordingPrompt, RecordingQuality, RecordingReceipt, StartEvokeSetup,
};
