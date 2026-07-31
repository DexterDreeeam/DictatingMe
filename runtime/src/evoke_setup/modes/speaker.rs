//! speakerVerify 模式：用 campplus 声纹嵌入确认说话人身份。

use std::path::Path;

use async_trait::async_trait;
use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};

use super::{
    primary_asset_file, profile, validate_input, validate_keyword, EvokeModeProcessor, ProcessInput,
};
use crate::evoke_setup::audio::read_wav_16k;
use crate::evoke_setup::math::{cosine_similarity, normalize_in_place};
use crate::evoke_setup::types::{EvokeArtifact, EvokeMode, EvokeProfile};
use crate::storage::{AssetGroup, AssetManager, StorageError};

pub struct SpeakerProcessor;

#[async_trait]
impl EvokeModeProcessor for SpeakerProcessor {
    fn mode(&self) -> EvokeMode {
        EvokeMode::SpeakerVerify
    }

    async fn process(
        &self,
        input: ProcessInput,
        assets: &AssetManager,
    ) -> Result<EvokeProfile, StorageError> {
        validate_input(&input, self.mode())?;
        validate_keyword(assets, &input.phrase)?;
        let required_assets = assets.descriptors_for_group(AssetGroup::SpeakerRecognition)?;
        let required_asset_ids = required_assets
            .iter()
            .map(|descriptor| descriptor.id.clone())
            .collect();
        let descriptor = assets
            .primary_descriptor(AssetGroup::SpeakerRecognition)?
            .clone();
        let model = primary_asset_file(assets, &descriptor)?;
        let paths = input.recording_paths.clone();
        let centroid = tokio::task::spawn_blocking(move || {
            let mut config = SpeakerEmbeddingExtractorConfig::default();
            config.model = Some(model.display().to_string());
            config.num_threads = 2;
            let extractor = SpeakerEmbeddingExtractor::create(&config).ok_or_else(|| {
                StorageError("failed to create speaker embedding extractor".to_owned())
            })?;
            let mut embeddings = Vec::new();
            for path in &paths {
                embeddings.push(extract_speaker_embedding(&extractor, path)?);
            }
            Ok::<_, StorageError>(average_embeddings(&embeddings))
        })
        .await
        .map_err(|error| StorageError(format!("speaker processing task failed: {error}")))??;
        if centroid.is_empty() {
            return Err(StorageError(
                "speaker processing produced an empty centroid".to_owned(),
            ));
        }
        Ok(profile(
            input.mode,
            input.phrase,
            0.68,
            EvokeArtifact::SpeakerVerify { centroid },
            required_asset_ids,
        ))
    }
}

/// 多条注册嵌入取平均并归一。
pub fn average_embeddings(embeddings: &[Vec<f32>]) -> Vec<f32> {
    let Some(first) = embeddings.first() else {
        return Vec::new();
    };
    let mut result = vec![0.0; first.len()];
    for embedding in embeddings {
        for (target, value) in result.iter_mut().zip(embedding) {
            *target += *value;
        }
    }
    for value in &mut result {
        *value /= embeddings.len() as f32;
    }
    normalize_in_place(&mut result);
    result
}

pub fn extract_speaker_embedding(
    extractor: &SpeakerEmbeddingExtractor,
    path: &Path,
) -> Result<Vec<f32>, StorageError> {
    let samples = read_wav_16k(path).map_err(StorageError)?;
    let stream = extractor
        .create_stream()
        .ok_or_else(|| StorageError("failed to create speaker embedding stream".to_owned()))?;
    stream.accept_waveform(16_000, &samples);
    if !extractor.is_ready(&stream) {
        return Err(StorageError(format!(
            "recording '{}' is too short for speaker embedding",
            path.display()
        )));
    }
    extractor.compute(&stream).ok_or_else(|| {
        StorageError(format!(
            "failed to compute speaker embedding for '{}'",
            path.display()
        ))
    })
}

pub fn speaker_similarity(centroid: &[f32], embedding: &[f32]) -> f32 {
    ((cosine_similarity(centroid, embedding) + 1.0) * 0.5).clamp(0.0, 1.0)
}

/// 检测期打分：实时音频的嵌入与注册质心的相似度。
pub fn score(
    extractor: &SpeakerEmbeddingExtractor,
    centroid: &[f32],
    samples: &[f32],
) -> f32 {
    let Some(stream) = extractor.create_stream() else {
        return 0.0;
    };
    stream.accept_waveform(16_000, samples);
    if !extractor.is_ready(&stream) {
        return 0.0;
    }
    extractor
        .compute(&stream)
        .map(|embedding| speaker_similarity(centroid, &embedding))
        .unwrap_or(0.0)
}
