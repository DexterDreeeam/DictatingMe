//! text 模式：只靠 KWS 命中，不做任何声学比对。

use async_trait::async_trait;

use super::{profile, validate_input, validate_keyword, EvokeModeProcessor, ProcessInput};
use crate::evoke_setup::types::{EvokeArtifact, EvokeMode, EvokeProfile};
use crate::storage::{AssetManager, StorageError};

pub struct TextProcessor;

#[async_trait]
impl EvokeModeProcessor for TextProcessor {
    fn mode(&self) -> EvokeMode {
        EvokeMode::Text
    }

    async fn process(
        &self,
        input: ProcessInput,
        assets: &AssetManager,
    ) -> Result<EvokeProfile, StorageError> {
        validate_input(&input, self.mode())?;
        let keyword_syntax = validate_keyword(assets, &input.phrase)?;
        Ok(profile(
            input.mode,
            input.phrase.clone(),
            0.5,
            EvokeArtifact::Text { keyword_syntax },
            Vec::new(),
        ))
    }
}

/// 检测期：text 模式没有独立的声学判据，直接用语音活跃度。
pub fn score(voice_activity: f32) -> f32 {
    voice_activity
}
