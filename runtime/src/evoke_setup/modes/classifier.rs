//! classifier 模式：在特征摘要上训一个逻辑回归。
//!
//! 正样本来自用户注册录音，负样本来自随应用分发的 babble 噪声。

use async_trait::async_trait;

use super::{
    primary_asset_file, profile, validate_input, validate_keyword, EvokeModeProcessor, ProcessInput,
};
use crate::evoke_setup::audio::read_wav_16k;
use crate::evoke_setup::math::{dot, sigmoid};
use crate::evoke_setup::spectral::{band_energy_sequence, Gate, ABSOLUTE_GATE, FEATURE_DIM};
use crate::evoke_setup::math::normalize_in_place;
use crate::evoke_setup::types::{EvokeArtifact, EvokeMode, EvokeProfile};
use crate::storage::{AssetGroup, AssetManager, StorageError};

/// classifier 的特征管线：帧序列 -> 均值与标准差拼接。
pub fn feature_sequence(samples: &[f32]) -> Vec<Vec<f32>> {
    band_energy_sequence(samples, Gate::Absolute(ABSOLUTE_GATE))
}

pub fn summarize_sequence(sequence: &[Vec<f32>]) -> Vec<f32> {
    if sequence.is_empty() {
        return vec![0.0; FEATURE_DIM * 2];
    }
    let dims = sequence[0].len();
    let mut mean = vec![0.0; dims];
    for frame in sequence {
        for (target, value) in mean.iter_mut().zip(frame) {
            *target += *value;
        }
    }
    for value in &mut mean {
        *value /= sequence.len() as f32;
    }
    let mut stddev = vec![0.0; dims];
    for frame in sequence {
        for ((target, value), avg) in stddev.iter_mut().zip(frame).zip(&mean) {
            *target += (value - avg).powi(2);
        }
    }
    for value in &mut stddev {
        *value = (*value / sequence.len() as f32).sqrt();
    }
    mean.extend(stddev);
    normalize_in_place(&mut mean);
    mean
}

pub fn train_logistic(positives: &[Vec<f32>], negatives: &[Vec<f32>]) -> (Vec<f32>, f32) {
    let dims = positives
        .first()
        .or_else(|| negatives.first())
        .map(Vec::len)
        .unwrap_or(FEATURE_DIM * 2);
    let mut weights = vec![0.0; dims];
    let mut bias = 0.0;
    let mut samples = Vec::new();
    samples.extend(positives.iter().cloned().map(|feature| (feature, 1.0)));
    samples.extend(negatives.iter().cloned().map(|feature| (feature, 0.0)));
    if samples.is_empty() {
        return (weights, bias);
    }
    for epoch in 0..500 {
        let rate = 0.18 / (1.0 + epoch as f32 * 0.004);
        let mut grad_w = vec![0.0; dims];
        let mut grad_b = 0.0;
        for (feature, label) in &samples {
            let prediction = sigmoid(dot(&weights, feature) + bias);
            let error = prediction - label;
            for ((gradient, value), weight) in grad_w.iter_mut().zip(feature).zip(&weights) {
                *gradient += error * value + 0.001 * weight;
            }
            grad_b += error;
        }
        let scale = 1.0 / samples.len() as f32;
        for (weight, gradient) in weights.iter_mut().zip(grad_w) {
            *weight -= rate * gradient * scale;
        }
        bias -= rate * grad_b * scale;
    }
    (weights, bias)
}

pub fn classifier_score(weights: &[f32], bias: f32, feature: &[f32]) -> f32 {
    sigmoid(dot(weights, feature) + bias)
}

pub struct ClassifierProcessor;

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
                positives.push(summarize_sequence(&feature_sequence(&samples)));
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
                    negatives.push(summarize_sequence(&feature_sequence(
                        &noise[start..start + window],
                    )));
                }
                if negatives.len() == before {
                    negatives.push(summarize_sequence(&feature_sequence(&noise)));
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

/// 检测期打分。
pub fn score(weights: &[f32], bias: f32, samples: &[f32]) -> f32 {
    let feature = summarize_sequence(&feature_sequence(samples));
    classifier_score(weights, bias, &feature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logistic_training_separates_simple_examples() {
        let (weights, bias) = train_logistic(&[vec![1.0, 0.0], vec![0.9, 0.1]], &[vec![0.0, 1.0]]);
        assert!(
            classifier_score(&weights, bias, &[1.0, 0.0])
                > classifier_score(&weights, bias, &[0.0, 1.0])
        );
    }
}
