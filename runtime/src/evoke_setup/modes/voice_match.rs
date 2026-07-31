//! voiceMatch 模式：把实时音频与注册时的声学模板做 DTW 比对。
//!
//! 注册期与检测期**必须使用同一条特征管线**，所以两者放在同一个文件里。
//! 唯一的入口是 [`feature_sequence`]，改动它会同时作用于两端。

use async_trait::async_trait;

use super::{profile, validate_input, validate_keyword, EvokeModeProcessor, ProcessInput};
use crate::evoke_setup::audio::read_wav_16k;
use crate::evoke_setup::math::cosine_distance;
use crate::evoke_setup::spectral::{band_energy_sequence, Gate};
use crate::evoke_setup::types::{EvokeArtifact, EvokeMode, EvokeProfile};
use crate::storage::{AssetManager, StorageError};

/// 模板固定帧数。序列在进 DTW 之前会被线性重采样到这个长度。
pub const TEMPLATE_FRAMES: usize = 64;

// ---------------------------------------------------------------- 特征

/// voiceMatch 的特征管线。注册期与检测期共用。
pub fn feature_sequence(samples: &[f32]) -> Vec<Vec<f32>> {
    let raw = band_energy_sequence(samples, Gate::Absolute(crate::evoke_setup::spectral::ABSOLUTE_GATE));
    resample_sequence(&raw, TEMPLATE_FRAMES)
}

/// 把不等长的序列线性重采样到定长。
pub fn resample_sequence(sequence: &[Vec<f32>], target_frames: usize) -> Vec<Vec<f32>> {
    if sequence.is_empty() || target_frames == 0 {
        return Vec::new();
    }
    if sequence.len() == 1 {
        return vec![sequence[0].clone(); target_frames];
    }
    (0..target_frames)
        .map(|index| {
            let position =
                index as f32 * (sequence.len() - 1) as f32 / (target_frames - 1).max(1) as f32;
            let left = position.floor() as usize;
            let right = position.ceil() as usize;
            let fraction = position - left as f32;
            sequence[left]
                .iter()
                .zip(&sequence[right])
                .map(|(a, b)| a + (b - a) * fraction)
                .collect()
        })
        .collect()
}

/// 逐帧平均多条注册序列，得到单一模板。要求各序列等长。
pub fn average_sequences(sequences: &[Vec<Vec<f32>>]) -> Vec<Vec<f32>> {
    let valid = sequences
        .iter()
        .filter(|sequence| !sequence.is_empty())
        .collect::<Vec<_>>();
    if valid.is_empty() {
        return Vec::new();
    }
    let frames = valid[0].len();
    let dims = valid[0][0].len();
    let mut result = vec![vec![0.0; dims]; frames];
    for sequence in &valid {
        for (target, source) in result.iter_mut().zip(sequence.iter()) {
            for (value, input) in target.iter_mut().zip(source.iter()) {
                *value += *input;
            }
        }
    }
    for frame in &mut result {
        for value in frame {
            *value /= valid.len() as f32;
        }
    }
    result
}

/// 动态时间规整相似度。允许两条序列不等长。
pub fn dtw_similarity(left: &[Vec<f32>], right: &[Vec<f32>]) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let mut previous = vec![f32::INFINITY; right.len() + 1];
    previous[0] = 0.0;
    for left_frame in left {
        let mut current = vec![f32::INFINITY; right.len() + 1];
        for (index, right_frame) in right.iter().enumerate() {
            let cost = cosine_distance(left_frame, right_frame);
            current[index + 1] =
                cost + previous[index + 1].min(current[index]).min(previous[index]);
        }
        previous = current;
    }
    let distance = previous[right.len()] / (left.len() + right.len()) as f32;
    (-4.0 * distance).exp().clamp(0.0, 1.0)
}

// ---------------------------------------------------------------- 注册期

pub struct VoiceMatchProcessor;

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
                let sequence = feature_sequence(&samples);
                if sequence.is_empty() {
                    return Err(StorageError(format!(
                        "recording '{}' produced no acoustic features",
                        path.display()
                    )));
                }
                sequences.push(sequence);
            }
            let template = average_sequences(&sequences);
            let threshold = calibrate_threshold(&template, &sequences);
            Ok::<_, StorageError>((template, threshold))
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

/// 阈值：注册样本自相似的最小值往下留 0.08 余量，再夹到合理区间。
fn calibrate_threshold(template: &[Vec<f32>], enrolled: &[Vec<Vec<f32>>]) -> f32 {
    let minimum = enrolled
        .iter()
        .map(|sequence| dtw_similarity(sequence, template))
        .fold(1.0_f32, f32::min);
    (minimum - 0.08).clamp(0.52, 0.88)
}

// ---------------------------------------------------------------- 检测期

/// 检测期打分：与注册期用的是同一个 [`feature_sequence`]。
pub fn score(samples: &[f32], template: &[Vec<f32>]) -> f32 {
    dtw_similarity(&feature_sequence(samples), template)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_sequences_score_higher_than_opposites() {
        let left = vec![vec![1.0, 0.0]; 8];
        let same = vec![vec![1.0, 0.0]; 8];
        let different = vec![vec![0.0, 1.0]; 8];
        assert!(dtw_similarity(&left, &same) > dtw_similarity(&left, &different));
    }

    #[test]
    fn feature_dimension_is_stable() {
        use crate::evoke_setup::spectral::FEATURE_DIM;
        let samples = (0..16_000)
            .map(|i| (i as f32 * 220.0 * std::f32::consts::TAU / 16_000.0).sin() * 0.3)
            .collect::<Vec<_>>();
        let sequence = feature_sequence(&samples);
        assert_eq!(sequence.len(), TEMPLATE_FRAMES);
        assert!(sequence.iter().all(|frame| frame.len() == FEATURE_DIM));
    }
}
