use std::path::{Path, PathBuf};

use async_trait::async_trait;
use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};

use super::features::{
    average_embeddings, average_sequences, classifier_score, cosine_similarity, dtw_similarity,
    extract_feature_sequence, read_wav_16k, resample_sequence, summarize_sequence, train_logistic,
    TEMPLATE_FRAMES,
};
use super::{EvokeArtifact, EvokeMode, EvokeProfile};
use crate::models::evoke_model::keyword_syntax_for_model;
use crate::storage::{now_ms, AssetDescriptor, AssetGroup, AssetKind, AssetManager, StorageError};

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
        EvokeMode::Text => Box::new(TextProcessor),
        EvokeMode::VoiceMatch => Box::new(VoiceMatchProcessor),
        EvokeMode::SpeakerVerify => Box::new(SpeakerProcessor),
        EvokeMode::Classifier => Box::new(ClassifierProcessor),
    }
}

struct TextProcessor;

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

struct VoiceMatchProcessor;

#[async_trait]
impl EvokeModeProcessor for VoiceMatchProcessor {
    fn mode(&self) -> EvokeMode {
        EvokeMode::VoiceMatch
    }

    async fn process(
        &self,
        input: ProcessInput,
        assets: &AssetManager,
    ) -> Result<EvokeProfile, StorageError> {
        validate_input(&input, self.mode())?;
        validate_keyword(assets, &input.phrase)?;
        let paths = input.recording_paths.clone();
        let (template, threshold) = tokio::task::spawn_blocking(move || {
            let mut sequences = Vec::new();
            for path in &paths {
                let samples = read_wav_16k(path).map_err(StorageError)?;
                let sequence =
                    resample_sequence(&extract_feature_sequence(&samples), TEMPLATE_FRAMES);
                if sequence.is_empty() {
                    return Err(StorageError(format!(
                        "recording '{}' produced no acoustic features",
                        path.display()
                    )));
                }
                sequences.push(sequence);
            }
            let template = average_sequences(&sequences);
            let minimum = sequences
                .iter()
                .map(|sequence| dtw_similarity(sequence, &template))
                .fold(1.0_f32, f32::min);
            Ok::<_, StorageError>((template, (minimum - 0.08).clamp(0.52, 0.88)))
        })
        .await
        .map_err(|error| StorageError(format!("voice template task failed: {error}")))??;
        Ok(profile(
            input.mode,
            input.phrase,
            threshold,
            EvokeArtifact::VoiceMatch { template },
            Vec::new(),
        ))
    }
}

struct SpeakerProcessor;

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

struct ClassifierProcessor;

#[async_trait]
impl EvokeModeProcessor for ClassifierProcessor {
    fn mode(&self) -> EvokeMode {
        EvokeMode::Classifier
    }

    async fn process(
        &self,
        input: ProcessInput,
        assets: &AssetManager,
    ) -> Result<EvokeProfile, StorageError> {
        validate_input(&input, self.mode())?;
        validate_keyword(assets, &input.phrase)?;
        let descriptors = assets
            .descriptors_for_group(AssetGroup::ClassifierRecognition)?
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let required_asset_ids = descriptors
            .iter()
            .map(|descriptor| descriptor.id.clone())
            .collect();
        let negative_paths = descriptors
            .iter()
            .map(|descriptor| primary_asset_file(assets, descriptor))
            .collect::<Result<Vec<_>, _>>()?;
        let positive_paths = input.recording_paths.clone();
        let (weights, bias, threshold) = tokio::task::spawn_blocking(move || {
            let mut positives = Vec::new();
            for path in &positive_paths {
                let samples = read_wav_16k(path).map_err(StorageError)?;
                positives.push(summarize_sequence(&extract_feature_sequence(&samples)));
            }
            let window = 16_000 * 5;
            let mut negatives = Vec::new();
            for negative_path in negative_paths {
                let noise = read_wav_16k(&negative_path).map_err(StorageError)?;
                let before = negatives.len();
                for start in (0..noise.len().saturating_sub(window))
                    .step_by((window / 2).max(1))
                    .take(12)
                {
                    negatives.push(summarize_sequence(&extract_feature_sequence(
                        &noise[start..start + window],
                    )));
                }
                if negatives.len() == before {
                    negatives.push(summarize_sequence(&extract_feature_sequence(&noise)));
                }
            }
            let (weights, bias) = train_logistic(&positives, &negatives);
            let positive_floor = positives
                .iter()
                .map(|feature| classifier_score(&weights, bias, feature))
                .fold(1.0_f32, f32::min);
            let negative_ceiling = negatives
                .iter()
                .map(|feature| classifier_score(&weights, bias, feature))
                .fold(0.0_f32, f32::max);
            let threshold = ((positive_floor + negative_ceiling) * 0.5).clamp(0.52, 0.82);
            Ok::<_, StorageError>((weights, bias, threshold))
        })
        .await
        .map_err(|error| StorageError(format!("classifier training task failed: {error}")))??;
        Ok(profile(
            input.mode,
            input.phrase,
            threshold,
            EvokeArtifact::Classifier { weights, bias },
            required_asset_ids,
        ))
    }
}

fn validate_input(input: &ProcessInput, expected: EvokeMode) -> Result<(), StorageError> {
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

fn validate_keyword(assets: &AssetManager, phrase: &str) -> Result<String, StorageError> {
    let descriptor = assets.first_descriptor_of_kind(AssetKind::PresetEvoke)?;
    let path = assets.asset_path(descriptor)?;
    keyword_syntax_for_model(&path, phrase, 0.65).map_err(|error| StorageError(error.0))
}

fn profile(
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

fn primary_asset_file(
    assets: &AssetManager,
    descriptor: &AssetDescriptor,
) -> Result<PathBuf, StorageError> {
    let file = descriptor.files.first().ok_or_else(|| {
        StorageError(format!("asset '{}' has no registered files", descriptor.id))
    })?;
    Ok(assets.asset_path(descriptor)?.join(&file.path))
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
