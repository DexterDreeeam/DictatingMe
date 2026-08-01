//! speakerVerify 模式：用 campplus 声纹嵌入确认说话人身份。

use std::path::Path;

use async_trait::async_trait;
use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};

use super::{
    primary_asset_file, profile, validate_input, validate_keyword, EvokeModeProcessor, ProcessInput,
};
use crate::evoke_setup::audio::read_wav_16k;
use crate::evoke_setup::math::{cosine_similarity, normalize_in_place};
use crate::evoke_setup::spectral::crop_by_energy;
use crate::evoke_setup::types::{EvokeArtifact, EvokeMode, EvokeProfile};
use crate::storage::{AssetGroup, AssetManager, StorageError};

const SPEAKER_GATE_THRESHOLD: f32 = 0.75;
const SPEAKER_WINDOW_MS: usize = 2_000;

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
        let (template, threshold, centroid) = tokio::task::spawn_blocking(move || {
            let mut recordings = Vec::with_capacity(paths.len());
            for path in &paths {
                recordings.push(read_wav_16k(path).map_err(StorageError)?);
            }
            let (template, threshold) = super::voice_match::enroll(&recordings)?;

            let mut config = SpeakerEmbeddingExtractorConfig::default();
            config.model = Some(model.display().to_string());
            config.num_threads = 2;
            let extractor = SpeakerEmbeddingExtractor::create(&config).ok_or_else(|| {
                StorageError("failed to create speaker embedding extractor".to_owned())
            })?;
            let mut embeddings = Vec::with_capacity(recordings.len());
            for (index, samples) in recordings.iter().enumerate() {
                embeddings.push(extract_speaker_embedding_from_samples(
                    &extractor,
                    samples,
                    &format!("recording {index}"),
                )?);
            }
            let centroid = average_embeddings(&embeddings);
            if centroid.is_empty() {
                return Err(StorageError(
                    "speaker processing produced an empty centroid".to_owned(),
                ));
            }
            Ok::<_, StorageError>((template, threshold, centroid))
        })
        .await
        .map_err(|error| StorageError(format!("speaker processing task failed: {error}")))??;
        Ok(profile(
            input.mode,
            input.phrase,
            threshold,
            EvokeArtifact::SpeakerVerify {
                template,
                centroid,
                speaker_threshold: SPEAKER_GATE_THRESHOLD,
            },
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
    extract_speaker_embedding_from_samples(extractor, &samples, &path.display().to_string())
}

fn extract_speaker_embedding_from_samples(
    extractor: &SpeakerEmbeddingExtractor,
    samples: &[f32],
    source: &str,
) -> Result<Vec<f32>, StorageError> {
    let samples = crop_by_energy(samples, SPEAKER_WINDOW_MS);
    let stream = extractor
        .create_stream()
        .ok_or_else(|| StorageError("failed to create speaker embedding stream".to_owned()))?;
    stream.accept_waveform(16_000, &samples);
    if !extractor.is_ready(&stream) {
        return Err(StorageError(format!(
            "{source} is too short for speaker embedding"
        )));
    }
    extractor.compute(&stream).ok_or_else(|| {
        StorageError(format!("failed to compute speaker embedding for {source}"))
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
    let samples = crop_by_energy(samples, SPEAKER_WINDOW_MS);
    let Some(stream) = extractor.create_stream() else {
        return 0.0;
    };
    stream.accept_waveform(16_000, &samples);
    if !extractor.is_ready(&stream) {
        return 0.0;
    }
    extractor
        .compute(&stream)
        .map(|embedding| speaker_similarity(centroid, &embedding))
        .unwrap_or(0.0)
}
