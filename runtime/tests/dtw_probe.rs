//! voiceMatch（DTW 模板）为什么一加噪就崩：把管线拆开逐层量，并且**正负样本一起量**。
//!
//! E2E 实测：voiceMatch clean 下 63.7%（≈ 等于 KWS 召回，说明 DTW 几乎不拒），
//! traffic@5dB 下只剩 1.2%——DTW 通过率从 ~100% 掉到 ~2%。
//!
//! 假设：`extract_feature_sequence` 里 `rms >= 0.003` 是**绝对**门限。
//! clean 音频里静音帧被丢掉，序列只剩语音帧；加噪后噪声把每一帧都抬过门限，
//! 帧数暴涨，再被 `resample_sequence(.., 64)` 压回 64 帧时关键词只占一小段——
//! 时间对齐在进 DTW 之前就已经被破坏。
//!
//! 候选修复（全部同时量正样本通过率与负样本误接受率，只看 TPR 会得出错误结论）：
//!   A. 注册阶段增广后取 min 定阈值（只动阈值）
//!   B. 自适应 RMS 门限：按本段音频的噪声底噪定，而不是写死 0.003（直接修根因）
//!   C. 按能量裁出关键词段再重采样（修时间对齐）
//!   B+C 组合
//!
//! ```text
//! cargo test --release -p dictatingme-runtime --test dtw_probe -- --ignored --nocapture
//! ```

mod support;

use std::path::PathBuf;
use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;

use dictatingme_runtime::evoke_setup::features::{
    average_sequences, dtw_similarity, extract_feature_sequence, read_wav_16k, resample_sequence,
    FEATURE_DIM, TEMPLATE_FRAMES,
};

use support::augment::{apply, Condition, Rng};
use support::corpus::{load_groups, load_noise_by_category, Role};

const FRAME: usize = 400;
const HOP: usize = 160;
const FFT_SIZE: usize = 512;

/// 复刻 `features::logarithmic_edges`（该函数是私有的）。
fn logarithmic_edges(start: usize, end: usize, bands: usize) -> Vec<usize> {
    let low = (start.max(1)) as f32;
    let high = end as f32;
    (0..=bands)
        .map(|index| {
            let ratio = index as f32 / bands as f32;
            (low * (high / low).powf(ratio)).round() as usize
        })
        .collect()
}

/// 复刻 `extract_feature_sequence`，但把 RMS 门限做成可注入的。
/// `gate` = None 表示不丢帧。
fn feature_sequence_with_gate(samples: &[f32], gate: Option<f32>) -> Vec<Vec<f32>> {
    if samples.len() < FRAME {
        return Vec::new();
    }
    let mut planner = FftPlanner::<f32>::new();
    let fft = Arc::clone(&planner.plan_fft_forward(FFT_SIZE));
    let band_edges = logarithmic_edges(2, 220, FEATURE_DIM);
    let mut sequence = Vec::new();
    let mut offset = 0;
    while offset + FRAME <= samples.len() {
        let chunk = &samples[offset..offset + FRAME];
        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / FRAME as f32).sqrt();
        if gate.is_none_or(|limit| rms >= limit) {
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
            let norm = feature.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 1e-6 {
                for value in &mut feature {
                    *value /= norm;
                }
            }
            sequence.push(feature);
        }
        offset += HOP;
    }
    sequence
}

/// 按本段音频自己的底噪定门限：取各帧 RMS 的 20 分位当噪声底，门限压在底噪与峰值之间。
/// 这样 clean 与 noisy 会保留下相近的帧数，时间对齐才不会被噪声改写。
fn adaptive_gate(samples: &[f32]) -> f32 {
    let mut levels = Vec::new();
    let mut offset = 0;
    while offset + FRAME <= samples.len() {
        let chunk = &samples[offset..offset + FRAME];
        levels.push((chunk.iter().map(|s| s * s).sum::<f32>() / FRAME as f32).sqrt());
        offset += HOP;
    }
    if levels.is_empty() {
        return 0.003;
    }
    levels.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let floor = levels[levels.len() / 5];
    let peak = levels[levels.len() * 9 / 10];
    (floor + (peak - floor) * 0.35).max(0.003)
}

/// 变体：如何把一段音频变成定长 64 帧序列。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    /// 产品原样
    Product,
    /// B：自适应 RMS 门限
    Adaptive,
    /// C：能量裁剪 + 产品门限
    Crop,
    /// B+C
    CropAdaptive,
}

const VARIANTS: [(Variant, &str); 4] = [
    (Variant::Product, "产品原样"),
    (Variant::Adaptive, "B:自适应门限"),
    (Variant::Crop, "C:能量裁剪"),
    (Variant::CropAdaptive, "B+C"),
];

/// 找出短时能量最强的 1.6 秒——够装下 3~4 字唤醒词，又不至于把整段静音卷进来。
fn crop(samples: &[f32]) -> Vec<f32> {
    const WINDOW: usize = 16_000 * 8 / 5;
    const STEP: usize = 1_600;
    if samples.len() <= WINDOW {
        return samples.to_vec();
    }
    let mut best_start = 0;
    let mut best_energy = f32::NEG_INFINITY;
    let mut start = 0;
    while start + WINDOW <= samples.len() {
        let energy: f32 = samples[start..start + WINDOW].iter().map(|v| v * v).sum();
        if energy > best_energy {
            best_energy = energy;
            best_start = start;
        }
        start += STEP;
    }
    samples[best_start..best_start + WINDOW].to_vec()
}

fn sequence_for(variant: Variant, samples: &[f32]) -> (usize, Vec<Vec<f32>>) {
    let owned;
    let input = match variant {
        Variant::Crop | Variant::CropAdaptive => {
            owned = crop(samples);
            &owned[..]
        }
        _ => samples,
    };
    let raw = match variant {
        Variant::Product | Variant::Crop => extract_feature_sequence(input),
        Variant::Adaptive | Variant::CropAdaptive => {
            feature_sequence_with_gate(input, Some(adaptive_gate(input)))
        }
    };
    let frames = raw.len();
    (frames, resample_sequence(&raw, TEMPLATE_FRAMES))
}

#[derive(Default, Clone, Copy)]
struct Stat {
    frames: f64,
    similarity: f64,
    pass: u32,
    total: u32,
}

impl Stat {
    fn add(&mut self, frames: usize, similarity: f32, threshold: f32) {
        self.frames += frames as f64;
        self.similarity += similarity as f64;
        self.total += 1;
        if similarity >= threshold {
            self.pass += 1;
        }
    }
    fn mean_frames(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.frames / self.total as f64 }
    }
    fn mean_similarity(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.similarity / self.total as f64 }
    }
    fn rate(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.pass as f64 * 100.0 / self.total as f64 }
    }
}

fn merge(stats: &[Stat]) -> Stat {
    let mut out = Stat::default();
    for stat in stats {
        out.frames += stat.frames;
        out.similarity += stat.similarity;
        out.pass += stat.pass;
        out.total += stat.total;
    }
    out
}

fn conditions() -> Vec<Condition> {
    let noise = load_noise_by_category();
    let mut list = vec![Condition::clean()];
    for (category, snr) in [("traffic", 5.0_f32), ("cafe", 10.0), ("keyboard", 15.0)] {
        if let Some(path) = noise.get(category).and_then(|files| files.first()) {
            list.push(Condition {
                id: format!("{category}@{snr:.0}dB"),
                speed: 1.0,
                noise: Some(path.clone()),
                noise_category: category.to_owned(),
                snr_db: snr,
                gain: 1.0,
            });
        }
    }
    list
}

/// 阈值策略：干净注册（生产）/ 增广注册（方案 A）。
const POLICIES: [(&str, bool); 2] = [("干净注册", false), ("A:增广注册", true)];

#[test]
#[ignore = "需要 assets/corpus 语料，手动运行"]
fn probes_dtw_noise_collapse() {
    let groups = load_groups().expect("corpus manifest");
    let conditions = conditions();
    assert!(conditions.len() > 1, "no noise assets found; DTW probe needs them");

    // [variant][policy][condition] -> 正样本 / 冒充者
    let shape = || vec![vec![vec![Stat::default(); conditions.len()]; POLICIES.len()]; VARIANTS.len()];
    let mut positive = shape();
    let mut impostor = shape();
    let mut thresholds = vec![vec![Vec::new(); POLICIES.len()]; VARIANTS.len()];

    for group in &groups {
        let paths_of = |role| -> Vec<PathBuf> {
            group.by_role(role).into_iter().map(|item| item.path.clone()).collect()
        };
        let enroll = paths_of(Role::Enroll);
        let positives = paths_of(Role::Positive);
        let impostors = paths_of(Role::Impostor);
        if enroll.is_empty() || positives.is_empty() {
            continue;
        }
        let enroll_audio: Vec<Vec<f32>> = enroll
            .iter()
            .map(|path| read_wav_16k(path).expect("enroll wav"))
            .collect();

        for (variant_index, (variant, _)) in VARIANTS.into_iter().enumerate() {
            let sequences: Vec<Vec<Vec<f32>>> = enroll_audio
                .iter()
                .map(|samples| sequence_for(variant, samples).1)
                .collect();
            let template = average_sequences(&sequences);
            let clean_min = sequences
                .iter()
                .map(|sequence| dtw_similarity(sequence, &template))
                .fold(1.0_f32, f32::min);

            // 方案 A：注册样本也过一遍各噪声条件，阈值取全体 min。
            let mut rng = Rng::new(0x5EED);
            let mut augmented_min = clean_min;
            for samples in &enroll_audio {
                for condition in conditions.iter().skip(1) {
                    let noisy = apply(samples, condition, &mut rng).expect("augment");
                    let sequence = sequence_for(variant, &noisy).1;
                    augmented_min = augmented_min.min(dtw_similarity(&sequence, &template));
                }
            }
            let policy_thresholds = [
                (clean_min - 0.08).clamp(0.52, 0.88),
                (augmented_min - 0.08).clamp(0.52, 0.88),
            ];
            for (policy_index, threshold) in policy_thresholds.iter().enumerate() {
                thresholds[variant_index][policy_index].push(*threshold);
            }

            let mut rng = Rng::new(0xC0FFEE);
            for (paths, bucket) in [(&positives, &mut positive), (&impostors, &mut impostor)] {
                for path in paths {
                    let clean = read_wav_16k(path).expect("wav");
                    for (condition_index, condition) in conditions.iter().enumerate() {
                        let samples = if condition.noise.is_none() {
                            clean.clone()
                        } else {
                            apply(&clean, condition, &mut rng).expect("augment")
                        };
                        let (frames, sequence) = sequence_for(variant, &samples);
                        let similarity = dtw_similarity(&sequence, &template);
                        for (policy_index, threshold) in policy_thresholds.iter().enumerate() {
                            bucket[variant_index][policy_index][condition_index].add(
                                frames, similarity, *threshold,
                            );
                        }
                    }
                }
            }
        }
    }

    println!("\n=== 1. 假设验证：进 DTW 之前的原始帧数（重采样到 64 帧之前）===");
    print!("{:<16}", "条件");
    for (_, name) in VARIANTS {
        print!("{name:>14}");
    }
    println!();
    for (condition_index, condition) in conditions.iter().enumerate() {
        print!("{:<16}", condition.id);
        for variant_index in 0..VARIANTS.len() {
            print!("{:>14.1}", positive[variant_index][0][condition_index].mean_frames());
        }
        println!();
    }
    println!("产品原样一列里 clean 与 noisy 的差距，就是「绝对 RMS 门限」造成的时间对齐破坏。");

    println!("\n=== 2. 正样本 DTW 相似度均值 ===");
    print!("{:<16}", "条件");
    for (_, name) in VARIANTS {
        print!("{name:>14}");
    }
    println!();
    for (condition_index, condition) in conditions.iter().enumerate() {
        print!("{:<16}", condition.id);
        for variant_index in 0..VARIANTS.len() {
            print!("{:>14.3}", positive[variant_index][0][condition_index].mean_similarity());
        }
        println!();
    }

    for (policy_index, (policy_name, _)) in POLICIES.into_iter().enumerate() {
        println!("\n=== 3.{} 阈值策略「{policy_name}」下的 DTW 层通过率（%）===", policy_index + 1);
        println!("正 = 本人念唤醒词，越高越好；冒 = 他人念同一唤醒词，voiceMatch 应当拒绝，越低越好。");
        print!("{:<16}", "条件");
        for (_, name) in VARIANTS {
            print!("{:>16}", format!("{name} 正/冒"));
        }
        println!();
        for (condition_index, condition) in conditions.iter().enumerate() {
            print!("{:<16}", condition.id);
            for variant_index in 0..VARIANTS.len() {
                let p = positive[variant_index][policy_index][condition_index].rate();
                let i = impostor[variant_index][policy_index][condition_index].rate();
                print!("{:>16}", format!("{p:.0}/{i:.0}"));
            }
            println!();
        }
        print!("{:<16}", "全条件合计");
        for variant_index in 0..VARIANTS.len() {
            let p = merge(&positive[variant_index][policy_index]).rate();
            let i = merge(&impostor[variant_index][policy_index]).rate();
            print!("{:>16}", format!("{p:.1}/{i:.1}"));
        }
        println!();
    }

    println!("\n=== 4. 注册阈值均值 ===");
    print!("{:<16}", "策略");
    for (_, name) in VARIANTS {
        print!("{name:>14}");
    }
    println!();
    for (policy_index, (policy_name, _)) in POLICIES.into_iter().enumerate() {
        print!("{policy_name:<16}");
        for variant_index in 0..VARIANTS.len() {
            let values = &thresholds[variant_index][policy_index];
            let mean = values.iter().sum::<f32>() / values.len().max(1) as f32;
            print!("{mean:>14.3}");
        }
        println!();
    }
}
