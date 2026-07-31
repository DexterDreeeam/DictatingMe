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
    /// 写死的绝对门限。classifier 与旧 voiceMatch 路径用这个。
    Absolute(f32),
    /// 以本段音频自身底噪为基准。voiceMatch 新路径用这个。
    #[allow(dead_code)]
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
            normalize_in_place(&mut feature);
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
