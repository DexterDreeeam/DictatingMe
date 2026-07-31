//! 共用频谱底座：分帧、FFT、对数间隔频带能量。
//!
//! voiceMatch 与 classifier 共用这一层。两者的差别在**门限策略**与**后处理**上，
//! 而不是在频谱计算上，所以这里把门限做成参数注入。

use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;

use super::math::normalize_in_place;

/// 频带数。voiceMatch 与 classifier 共用同一维度。
pub const FEATURE_DIM: usize = 12;

pub(crate) const FRAME: usize = 400;
pub(crate) const HOP: usize = 160;
pub(crate) const FFT_SIZE: usize = 512;

/// 逐帧 RMS 门限策略。
#[derive(Debug, Clone, Copy)]
pub(crate) enum Gate {
    /// 写死的绝对门限。classifier 用这个。
    Absolute(f32),
    /// 以本段音频自身底噪为基准。voiceMatch 用这个。
    Adaptive,
}

/// 各帧的 RMS。裁剪与自适应门限都要用。
pub(crate) fn frame_levels(samples: &[f32]) -> Vec<f32> {
    let mut levels = Vec::new();
    let mut offset = 0;
    while offset + FRAME <= samples.len() {
        let chunk = &samples[offset..offset + FRAME];
        levels.push((chunk.iter().map(|s| s * s).sum::<f32>() / FRAME as f32).sqrt());
        offset += HOP;
    }
    levels
}

/// 按本段音频自己的底噪定门限：20 分位当噪声底、90 分位当峰值，
/// 门限压在两者之间。这样干净与带噪音频会保留下相近的帧数，
/// 时间对齐才不会被噪声改写。
pub(crate) fn adaptive_gate(samples: &[f32]) -> f32 {
    let mut levels = frame_levels(samples);
    if levels.is_empty() {
        return ABSOLUTE_GATE;
    }
    levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let floor = levels[levels.len() / 5];
    let peak = levels[levels.len() * 9 / 10];
    (floor + (peak - floor) * 0.35).max(ABSOLUTE_GATE)
}

/// 历史上写死的绝对门限值。
pub(crate) const ABSOLUTE_GATE: f32 = 0.003;

impl Gate {
    fn resolve(self, samples: &[f32]) -> f32 {
        match self {
            Self::Absolute(value) => value,
            Self::Adaptive => adaptive_gate(samples),
        }
    }
}

/// 分帧 → Hamming 窗 → FFT → 对数间隔频带的对数能量 → 逐帧 L2 归一。
///
/// 低于门限的帧会被丢弃，因此输出帧数取决于门限策略。
pub(crate) fn band_energy_sequence(samples: &[f32], gate: Gate) -> Vec<Vec<f32>> {
    let mut sequence = band_energy_sequence_raw(samples, gate);
    for frame in &mut sequence {
        normalize_in_place(frame);
    }
    sequence
}

/// 同 [`band_energy_sequence`]，但**不做逐帧 L2 归一**。
///
/// voiceMatch 需要先做 CMVN 再归一，顺序反了 CMVN 会失效——
/// 归一后每帧模长恒为 1，逐维均值统计就不再反映信道特性。
pub(crate) fn band_energy_sequence_raw(samples: &[f32], gate: Gate) -> Vec<Vec<f32>> {
    if samples.len() < FRAME {
        return Vec::new();
    }
    let limit = gate.resolve(samples);
    let mut planner = FftPlanner::<f32>::new();
    let fft = Arc::clone(&planner.plan_fft_forward(FFT_SIZE));
    let band_edges = logarithmic_edges(2, 220, FEATURE_DIM);
    let mut sequence = Vec::new();
    let mut offset = 0;
    while offset + FRAME <= samples.len() {
        let chunk = &samples[offset..offset + FRAME];
        let rms = (chunk.iter().map(|sample| sample * sample).sum::<f32>() / FRAME as f32).sqrt();
        if rms >= limit {
            let mut spectrum = vec![Complex32::new(0.0, 0.0); FFT_SIZE];
            for (index, sample) in chunk.iter().enumerate() {
                let window = 0.54
                    - 0.46 * (2.0 * std::f32::consts::PI * index as f32 / (FRAME - 1) as f32).cos();
                spectrum[index].re = *sample * window;
            }
            fft.process(&mut spectrum);
            let power = spectrum[..FFT_SIZE / 2]
                .iter()
                .map(|value| value.norm_sqr())
                .collect::<Vec<_>>();
            let mut feature = Vec::with_capacity(FEATURE_DIM);
            for band in 0..FEATURE_DIM {
                let start = band_edges[band];
                let end = band_edges[band + 1].max(start + 1).min(power.len());
                let energy =
                    power[start..end].iter().copied().sum::<f32>() / (end - start).max(1) as f32;
                feature.push((energy + 1e-8).ln());
            }
            sequence.push(feature);
        }
        offset += HOP;
    }
    sequence
}

fn logarithmic_edges(start: usize, end: usize, bands: usize) -> Vec<usize> {
    let start = (start.max(1) as f32).ln();
    let end = (end.max(2) as f32).ln();
    (0..=bands)
        .map(|index| {
            let fraction = index as f32 / bands.max(1) as f32;
            (start + (end - start) * fraction).exp().round() as usize
        })
        .collect()
}

/// 截出短时能量最强的 `window_ms` 毫秒。
///
/// 唤醒词只占录音窗口的一小段，其余部分是静音或无关语音。
/// 把它们带进 DTW 会稀释相似度，也给近音词更多可对齐的自由度。
pub(crate) fn crop_by_energy(samples: &[f32], window_ms: usize) -> Vec<f32> {
    const STEP: usize = 1_600; // 100 ms
    let window = 16 * window_ms; // 16 kHz
    if samples.len() <= window {
        return samples.to_vec();
    }
    let mut best_start = 0;
    let mut best_energy = f32::NEG_INFINITY;
    let mut start = 0;
    while start + window <= samples.len() {
        let energy = samples[start..start + window]
            .iter()
            .map(|sample| sample * sample)
            .sum::<f32>();
        if energy > best_energy {
            best_energy = energy;
            best_start = start;
        }
        start += STEP;
    }
    samples[best_start..best_start + window].to_vec()
}

/// 逐段倒谱均值方差归一。
///
/// 消掉信道响应与稳态底噪带来的直流偏置，使注册环境与使用环境的差异
/// 不再直接体现在特征上。
pub(crate) fn cmvn_in_place(sequence: &mut [Vec<f32>]) {
    let Some(first) = sequence.first() else {
        return;
    };
    let dims = first.len();
    let count = sequence.len() as f32;

    let mut mean = vec![0.0_f32; dims];
    for frame in sequence.iter() {
        for (target, value) in mean.iter_mut().zip(frame) {
            *target += *value;
        }
    }
    for value in &mut mean {
        *value /= count;
    }

    let mut deviation = vec![0.0_f32; dims];
    for frame in sequence.iter() {
        for ((target, value), average) in deviation.iter_mut().zip(frame).zip(&mean) {
            *target += (value - average).powi(2);
        }
    }
    for value in &mut deviation {
        *value = (*value / count).sqrt().max(1e-5);
    }

    for frame in sequence.iter_mut() {
        for ((value, average), spread) in frame.iter_mut().zip(&mean).zip(&deviation) {
            *value = (*value - average) / spread;
        }
    }
}
