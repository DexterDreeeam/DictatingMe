//! 四种唤醒模式的实现。
//!
//! 每个模式**同时拥有注册期与检测期**的逻辑：
//!   - `Processor::process`  —— setup 阶段，把录音变成 `EvokeArtifact` 与阈值
//!   - `score`               —— detect 阶段，把实时音频与 artifact 比对出 mode_score
//!
//! 这样安排是有意的：两者用的特征管线必须一致，放在同一个文件里
//! 就不会出现「改了注册期忘了改检测期」这种静默失效。

pub mod classifier;
pub mod speaker;
pub mod text;
pub mod voice_match;

use std::path::PathBuf;

use async_trait::async_trait;

use super::types::{EvokeArtifact, EvokeMode, EvokeProfile};
use crate::models::evoke_model::keyword_syntax_for_model;
use crate::storage::{now_ms, AssetDescriptor, AssetKind, AssetManager, StorageError};

pub struct ProcessInput {
    pub mode: EvokeMode,
    pub phrase: String,
    pub recording_paths: Vec<PathBuf>,
}

#[async_trait]
pub trait EvokeModeProcessor: Send + Sync {
    fn mode(&self) -> EvokeMode;
    async fn process(
        &self,
        input: ProcessInput,
        assets: &AssetManager,
    ) -> Result<EvokeProfile, StorageError>;
}

pub fn processor_for(mode: EvokeMode) -> Box<dyn EvokeModeProcessor> {
    match mode {
        EvokeMode::Text => Box::new(text::TextProcessor),
        EvokeMode::VoiceMatch => Box::new(voice_match::VoiceMatchProcessor),
        EvokeMode::SpeakerVerify => Box::new(speaker::SpeakerProcessor),
        EvokeMode::Classifier => Box::new(classifier::ClassifierProcessor),
    }
}

// ---------------------------------------------------------------- 共用工具

pub(crate) fn validate_input(
    input: &ProcessInput,
    expected: EvokeMode,
) -> Result<(), StorageError> {
    if input.mode != expected {
        return Err(StorageError(
            "processor received the wrong evoke mode".to_owned(),
        ));
    }

    let phrase = input.phrase.trim();
    if phrase.is_empty() || phrase.chars().count() > 32 {
        return Err(StorageError(
            "wake phrase must contain between 1 and 32 characters".to_owned(),
        ));
    }
    if input.recording_paths.len() != expected.required_recordings() as usize {
        return Err(StorageError(format!(
            "{} mode requires {} recordings, found {}",
            expected.as_str(),
            expected.required_recordings(),
            input.recording_paths.len()
        )));
    }
    Ok(())
}

pub(crate) fn validate_keyword(
    assets: &AssetManager,
    phrase: &str,
) -> Result<String, StorageError> {
    let descriptor = assets.first_descriptor_of_kind(AssetKind::PresetEvoke)?;
    let path = assets.asset_path(descriptor)?;
    keyword_syntax_for_model(&path, phrase, 0.65).map_err(|error| StorageError(error.0))
}

pub(crate) fn profile(
    mode: EvokeMode,
    phrase: String,
    threshold: f32,
    artifact: EvokeArtifact,
    required_asset_ids: Vec<String>,
) -> EvokeProfile {
    EvokeProfile {
        id: uuid::Uuid::new_v4().to_string(),
        mode,
        phrase,
        threshold,
        artifact,
        required_asset_ids,
        created_at_ms: now_ms(),
    }
}

pub(crate) fn primary_asset_file(
    assets: &AssetManager,
    descriptor: &AssetDescriptor,
) -> Result<PathBuf, StorageError> {
    let file = descriptor.files.first().ok_or_else(|| {
        StorageError(format!("asset '{}' has no registered files", descriptor.id))
    })?;
    Ok(assets.asset_path(descriptor)?.join(&file.path))
}
