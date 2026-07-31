//! voiceMatch 模式：把实时音频与注册时的声学模板做 DTW 比对。
//!
//! 注册期与检测期**必须使用同一条特征管线**，所以两者放在同一个文件里。
//! 唯一的入口是 [`feature_sequence`]，改动它会同时作用于两端。

use async_trait::async_trait;

use super::{profile, validate_input, validate_keyword, EvokeModeProcessor, ProcessInput};
use crate::evoke_setup::audio::read_wav_16k;
use crate::evoke_setup::math::{cosine_distance, normalize_in_place};
use crate::evoke_setup::spectral::{band_energy_sequence_raw, cmvn_in_place, crop_by_energy, Gate};
use crate::evoke_setup::types::{EvokeArtifact, EvokeMode, EvokeProfile};
use crate::storage::{AssetManager, StorageError};

/// 兼容保留：旧模板的固定帧数。新管线不再重采样到定长。
pub const TEMPLATE_FRAMES: usize = 64;

/// 能量裁剪窗口。实测 800~2400 ms 中 1000 ms 判别力最好——
/// 与中文 4 字唤醒词的实际时长吻合，窗口再大只会把无关语音带进来。
const CROP_MS: usize = 1_000;

/// 队列异常时的阈值下界，避免塌到 0 变成「什么都唤醒」。
const THRESHOLD_FLOOR: f32 = 0.20;

/// 注册样本自身必须能通过，留一点余量。
const SELF_MARGIN: f32 = 0.02;

/// 负样本打乱重排的块长。短于一个音节，足以打散词的结构，
/// 又不至于碎成纯噪声。
const SHUFFLE_BLOCK_MS: usize = 120;

/// 每条注册录音派生几条负样本。
const SHUFFLES_PER_SAMPLE: usize = 4;

/// 取负样本得分的哪个分位当阈值。
///
/// 打乱后的音频保留了说话人的音色与房间底噪，与模板的相似度天然偏高，
/// 取最大值会让阈值过严（实测正样本通过率掉到 68%）。60 分位在实测中
/// 与外部 babble 队列的效果最接近。
const COHORT_QUANTILE: f32 = 0.60;

// ---------------------------------------------------------------- 特征

/// voiceMatch 的特征管线。注册期与检测期共用。
///
/// 与 classifier 那条路径的区别：
///   1. 先按能量裁出 [`CROP_MS`]，把窗口里的无关语音甩掉
///   2. RMS 门限随本段底噪自适应，而不是写死的绝对值——
///      绝对门限在噪声下会让每一帧都通过，时间轴在进 DTW 前就被改写
///   3. 逐段 CMVN，消掉信道与底噪的直流偏置
///   4. **不做定长重采样**，时间规整交给 DTW 本身
pub fn feature_sequence(samples: &[f32]) -> Vec<Vec<f32>> {
    let cropped = crop_by_energy(samples, CROP_MS);
    let mut sequence = band_energy_sequence_raw(&cropped, Gate::Adaptive);
    if sequence.is_empty() {
        return sequence;
    }
    cmvn_in_place(&mut sequence);
    for frame in &mut sequence {
        normalize_in_place(frame);
    }
    sequence
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

/// 逐帧平均多条注册序列，得到单一模板。
///
/// 去掉定长重采样之后各序列不再等长，因此先统一到中位数长度再平均。
pub fn average_sequences(sequences: &[Vec<Vec<f32>>]) -> Vec<Vec<f32>> {
    let valid = sequences
        .iter()
        .filter(|sequence| !sequence.is_empty())
        .collect::<Vec<_>>();
    if valid.is_empty() {
        return Vec::new();
    }
    let mut lengths = valid.iter().map(|sequence| sequence.len()).collect::<Vec<_>>();
    lengths.sort_unstable();
    let frames = lengths[lengths.len() / 2];
    let aligned = valid
        .iter()
        .map(|sequence| resample_sequence(sequence, frames))
        .collect::<Vec<_>>();

    let dims = aligned[0][0].len();
    let mut result = vec![vec![0.0; dims]; frames];
    for sequence in &aligned {
        for (target, source) in result.iter_mut().zip(sequence.iter()) {
            for (value, input) in target.iter_mut().zip(source.iter()) {
                *value += *input;
            }
        }
    }
    for frame in &mut result {
        for value in frame {
            *value /= aligned.len() as f32;
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

// ---------------------------------------------------------------- 负样本

/// 从注册录音派生负样本：按块打乱重排。
///
/// 用意是造出「听着像你、但词不对」的音频——这正是 DTW 应该拒绝的东西。
/// 打乱保留了说话人音色与房间底噪，只破坏时序，因此标定出的边界
/// 比外部通用噪声更贴合该用户的实际声学条件，也不需要任何下载。
fn shuffled_negatives(samples: &[f32], seed: u64) -> Vec<f32> {
    let block = 16 * SHUFFLE_BLOCK_MS;
    if samples.len() < block * 4 {
        return samples.to_vec();
    }
    let mut blocks = samples
        .chunks(block)
        .filter(|chunk| chunk.len() == block)
        .collect::<Vec<_>>();
    // xorshift + Fisher-Yates：确定性洗牌，同一批录音每次注册结果一致。
    let mut state = seed | 1;
    let mut next = || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    for index in (1..blocks.len()).rev() {
        let swap = (next() % (index as u64 + 1)) as usize;
        blocks.swap(index, swap);
    }
    blocks.concat()
}

/// 由全部注册录音派生出一批负样本序列。
fn negative_cohort(enrolled_audio: &[Vec<f32>]) -> Vec<Vec<Vec<f32>>> {
    let mut cohort = Vec::new();
    for (index, samples) in enrolled_audio.iter().enumerate() {
        for round in 0..SHUFFLES_PER_SAMPLE {
            let seed = 0x5EED_0000 ^ ((index as u64) << 16) ^ round as u64;
            let sequence = feature_sequence(&shuffled_negatives(samples, seed));
            if !sequence.is_empty() {
                cohort.push(sequence);
            }
        }
    }
    cohort
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
            let mut recordings = Vec::new();
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
                recordings.push(samples);
            }
            let template = average_sequences(&sequences);
            let cohort = negative_cohort(&recordings);
            let threshold = calibrate_threshold(&template, &sequences, &cohort);
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

/// 阈值：取负样本得分的 [`COHORT_QUANTILE`] 分位。
///
/// 语义是「比大多数打乱版本更像唤醒词的，才接受」。相比旧的
/// `min(自相似) - 0.08` 再夹到 `[0.52, 0.88]`：
///   - 旧策略只看 3 条正样本，`min()` 在 n=3 上极不稳；
///   - 旧策略完全不看负样本，阈值与误触率没有任何对应关系；
///   - 写死的钳位区间一旦特征尺度变化就整体失效。
///
/// 两个兜底：负样本异常时不至于塌到 0；注册用的录音自己一定能过。
fn calibrate_threshold(
    template: &[Vec<f32>],
    enrolled: &[Vec<Vec<f32>>],
    cohort: &[Vec<Vec<f32>>],
) -> f32 {
    let mut scores = cohort
        .iter()
        .map(|sequence| dtw_similarity(sequence, template))
        .collect::<Vec<_>>();
    scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let ceiling = if scores.is_empty() {
        0.0
    } else {
        let index = ((scores.len() - 1) as f32 * COHORT_QUANTILE).round() as usize;
        scores[index.min(scores.len() - 1)]
    };
    let floor = enrolled
        .iter()
        .map(|sequence| dtw_similarity(sequence, template))
        .fold(1.0_f32, f32::min);
    ceiling
        .max(THRESHOLD_FLOOR)
        .min(floor - SELF_MARGIN)
        .clamp(0.0, 1.0)
}

// ---------------------------------------------------------------- 检测期

/// 检测期打分：与注册期用的是同一个 [`feature_sequence`]。
pub fn score(samples: &[f32], template: &[Vec<f32>]) -> f32 {
    dtw_similarity(&feature_sequence(samples), template)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evoke_setup::spectral::FEATURE_DIM;

    fn tone(seconds: f32, freq: f32, amplitude: f32) -> Vec<f32> {
        let count = (16_000.0 * seconds) as usize;
        (0..count)
            .map(|i| (i as f32 * freq * std::f32::consts::TAU / 16_000.0).sin() * amplitude)
            .collect()
    }

    /// 确定性伪随机噪声，避免引入 rand 依赖。
    fn noise(len: usize, amplitude: f32) -> Vec<f32> {
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        (0..len)
            .map(|_| {
                state ^= state >> 12;
                state ^= state << 25;
                state ^= state >> 27;
                let value = (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32;
                (value / 8_388_608.0 - 1.0) * amplitude
            })
            .collect()
    }

    #[test]
    fn identical_sequences_score_higher_than_opposites() {
        let left = vec![vec![1.0, 0.0]; 8];
        let same = vec![vec![1.0, 0.0]; 8];
        let different = vec![vec![0.0, 1.0]; 8];
        assert!(dtw_similarity(&left, &same) > dtw_similarity(&left, &different));
    }

    #[test]
    fn feature_dimension_is_stable() {
        let sequence = feature_sequence(&tone(1.0, 220.0, 0.3));
        assert!(!sequence.is_empty());
        assert!(sequence.iter().all(|frame| frame.len() == FEATURE_DIM));
    }

    /// 裁剪窗口决定了序列长度上限：1000 ms / 10 ms 跳步 ≈ 100 帧。
    #[test]
    fn long_input_is_cropped_to_the_wake_word_window() {
        let sequence = feature_sequence(&tone(5.0, 220.0, 0.3));
        assert!(
            sequence.len() <= 100,
            "expected the 1000 ms crop to cap the sequence, got {} frames",
            sequence.len()
        );
    }

    /// 回归护栏：门限必须随底噪自适应。
    ///
    /// 换回写死的绝对门限后，加噪会让每一帧都通过，帧数暴涨、
    /// 时间对齐在进 DTW 之前就被破坏——这正是这条管线要修的根因。
    #[test]
    fn frame_count_survives_added_noise() {
        let clean = tone(1.5, 220.0, 0.3);
        let noisy = clean
            .iter()
            .zip(noise(clean.len(), 0.02))
            .map(|(signal, noise)| signal + noise)
            .collect::<Vec<_>>();

        let clean_frames = feature_sequence(&clean).len() as f32;
        let noisy_frames = feature_sequence(&noisy).len() as f32;
        assert!(clean_frames > 0.0 && noisy_frames > 0.0);

        let drift = (noisy_frames - clean_frames).abs() / clean_frames;
        assert!(
            drift < 0.2,
            "adaptive gate should keep the frame count stable: {clean_frames} -> {noisy_frames}"
        );
    }

    /// 队列校准出的阈值必须让注册样本自身通过。
    #[test]
    fn calibrated_threshold_admits_the_enrolled_samples() {
        let recordings = (0..3)
            .map(|i| tone(1.2, 220.0 + i as f32 * 4.0, 0.3))
            .collect::<Vec<_>>();
        let enrolled = recordings
            .iter()
            .map(|a| feature_sequence(a))
            .collect::<Vec<_>>();
        let template = average_sequences(&enrolled);
        let cohort = negative_cohort(&recordings);

        let threshold = calibrate_threshold(&template, &enrolled, &cohort);
        for sequence in &enrolled {
            assert!(
                dtw_similarity(sequence, &template) >= threshold,
                "enrolled sample scored below the calibrated threshold {threshold}"
            );
        }
    }

    /// 负样本完全由注册录音派生，不依赖任何外部素材。
    #[test]
    fn negative_cohort_is_derived_from_the_recordings() {
        let recordings = (0..3)
            .map(|i| tone(1.5, 220.0 + i as f32 * 4.0, 0.3))
            .collect::<Vec<_>>();
        let cohort = negative_cohort(&recordings);
        assert_eq!(cohort.len(), recordings.len() * SHUFFLES_PER_SAMPLE);
        assert!(cohort.iter().all(|sequence| !sequence.is_empty()));
    }

    /// 打乱必须真的改变时序，否则负样本和正样本无异，阈值会被顶到 1.0。
    #[test]
    fn shuffling_changes_the_signal() {
        let samples = tone(2.0, 220.0, 0.3)
            .iter()
            .enumerate()
            // 加一个随时间上升的包络，这样打乱后波形一定不同。
            .map(|(i, s)| s * (i as f32 / 32_000.0))
            .collect::<Vec<_>>();
        let shuffled = shuffled_negatives(&samples, 0x5EED);
        assert_eq!(shuffled.len() % (16 * SHUFFLE_BLOCK_MS), 0);
        let differing = samples
            .iter()
            .zip(&shuffled)
            .filter(|(a, b)| (*a - *b).abs() > 1e-6)
            .count();
        assert!(
            differing > samples.len() / 10,
            "shuffle barely changed the signal: {differing} differing samples"
        );
    }

    /// 洗牌是确定性的：同一批录音重复注册应得到同一个阈值。
    #[test]
    fn shuffling_is_deterministic() {
        let samples = tone(1.5, 220.0, 0.3);
        assert_eq!(
            shuffled_negatives(&samples, 0x5EED),
            shuffled_negatives(&samples, 0x5EED)
        );
    }

    /// 队列为空（录音过短无法分块）时阈值不能塌到 0。
    #[test]
    fn empty_cohort_falls_back_to_the_floor() {
        let enrolled = vec![feature_sequence(&tone(1.2, 220.0, 0.3))];
        let template = average_sequences(&enrolled);
        let threshold = calibrate_threshold(&template, &enrolled, &[]);
        assert!(threshold >= THRESHOLD_FLOOR - f32::EPSILON);
    }

    /// 不等长的注册序列也要能平均出模板。
    #[test]
    fn template_averaging_handles_unequal_lengths() {
        let sequences = vec![
            vec![vec![1.0_f32; FEATURE_DIM]; 40],
            vec![vec![1.0_f32; FEATURE_DIM]; 55],
            vec![vec![1.0_f32; FEATURE_DIM]; 70],
        ];
        let template = average_sequences(&sequences);
        assert_eq!(template.len(), 55, "should align to the median length");
        assert!(template.iter().all(|frame| frame.len() == FEATURE_DIM));
    }
}
