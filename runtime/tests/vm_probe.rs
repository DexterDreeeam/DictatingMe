//! voiceMatch 消融实验：把管线的每一层单独开关，量出各自贡献。
//!
//! 产品定位（用户拍板）：voiceMatch **不负责拒绝他人**，只判断「这句话是不是那个唤醒词」。
//! 因此：
//!   正样本 = pos（本人说唤醒词）        —— 唤醒率
//!   中性   = imp（别人说唤醒词）        —— 既不算收益也不算代价，单独报
//!   误触   = cfz（对立词）+ freespeech（日常说话）
//!
//! 现管线：read_wav → extract_feature_sequence(绝对RMS门限) → resample_sequence(64)
//!         → average_sequences → dtw_similarity(无带约束)
//!
//! 六个可疑点，本探针逐个开关：
//!   A 绝对 RMS 门限 0.003        -> 自适应门限
//!   B **重采样到固定 64 帧**      -> 原生长度（DTW 本就该干时间规整，重采样是重复且有害）
//!   C 12 维对数能量               -> mel 滤波器组 / MFCC / MFCC+delta
//!   D 无 CMVN                    -> 逐段倒谱均值方差归一
//!   E DTW 无带约束                -> Sakoe-Chiba 带
//!   F 模板逐帧平均                -> 多模板取最大
//!
//! 主指标用 **AUC 与 TPR@固定FAR**（与阈值策略解耦），再单独报生产阈值策略的工作点。
//!
//! ```text
//! cargo test --release -p dictatingme-runtime --test vm_probe -- --ignored --nocapture
//! ```

mod support;

use std::path::PathBuf;
use std::path::Path;
use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;

use dictatingme_runtime::evoke_setup::features::read_wav_16k;

use support::augment::{apply, Condition, Rng};
use support::corpus::{assets_dir, load_free_speech, load_groups, load_noise_by_category, Role};

const FRAME: usize = 400;
const HOP: usize = 160;
const FFT_SIZE: usize = 512;
const BINS: usize = FFT_SIZE / 2;
const PROD_DIM: usize = 12;
const PROD_FRAMES: usize = 64;

// ============================ 特征 ============================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Feat {
    /// 产品原样：12 个对数间隔频带的对数能量
    LogBand12,
    /// 24 个 mel 三角滤波器的对数能量
    Mel24,
    /// mel -> DCT-II 取 c1..c13
    Mfcc13,
    /// MFCC + 一阶差分
    Mfcc13D,
}

impl Feat {
    fn dim(self) -> usize {
        match self {
            Feat::LogBand12 => 12,
            Feat::Mel24 => 24,
            Feat::Mfcc13 => 13,
            Feat::Mfcc13D => 26,
        }
    }
}

/// 复刻 `features::logarithmic_edges`（私有）。
fn logarithmic_edges(start: usize, end: usize, bands: usize) -> Vec<usize> {
    let low = start.max(1) as f32;
    let high = end as f32;
    (0..=bands)
        .map(|i| (low * (high / low).powf(i as f32 / bands as f32)).round() as usize)
        .collect()
}

fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10f32.powf(mel / 2595.0) - 1.0)
}

/// 标准三角 mel 滤波器组，20 Hz .. 7600 Hz。
fn mel_filterbank(bands: usize) -> Vec<Vec<f32>> {
    let low = hz_to_mel(20.0);
    let high = hz_to_mel(7600.0);
    let points: Vec<f32> = (0..bands + 2)
        .map(|i| {
            let mel = low + (high - low) * i as f32 / (bands + 1) as f32;
            mel_to_hz(mel) * FFT_SIZE as f32 / 16_000.0
        })
        .collect();
    (0..bands)
        .map(|b| {
            let (l, c, r) = (points[b], points[b + 1], points[b + 2]);
            (0..BINS)
                .map(|bin| {
                    let f = bin as f32;
                    if f <= l || f >= r {
                        0.0
                    } else if f <= c {
                        (f - l) / (c - l).max(1e-6)
                    } else {
                        (r - f) / (r - c).max(1e-6)
                    }
                })
                .collect()
        })
        .collect()
}

fn dct2(input: &[f32], keep: usize) -> Vec<f32> {
    let n = input.len() as f32;
    (1..=keep)
        .map(|k| {
            input
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    v * (std::f32::consts::PI * k as f32 * (i as f32 + 0.5) / n).cos()
                })
                .sum::<f32>()
                * (2.0 / n).sqrt()
        })
        .collect()
}

struct Extractor {
    fft: Arc<dyn rustfft::Fft<f32>>,
    log_edges: Vec<usize>,
    mel: Vec<Vec<f32>>,
}

impl Extractor {
    fn new() -> Self {
        let mut planner = FftPlanner::<f32>::new();
        Self {
            fft: Arc::clone(&planner.plan_fft_forward(FFT_SIZE)),
            log_edges: logarithmic_edges(2, 220, PROD_DIM),
            mel: mel_filterbank(24),
        }
    }

    fn power(&self, chunk: &[f32]) -> Vec<f32> {
        let mut spectrum = vec![Complex32::new(0.0, 0.0); FFT_SIZE];
        for (i, s) in chunk.iter().enumerate() {
            let w = 0.54
                - 0.46 * (2.0 * std::f32::consts::PI * i as f32 / (FRAME - 1) as f32).cos();
            spectrum[i].re = *s * w;
        }
        self.fft.process(&mut spectrum);
        spectrum[..BINS].iter().map(|v| v.norm_sqr()).collect()
    }

    fn frame_feature(&self, power: &[f32], feat: Feat) -> Vec<f32> {
        match feat {
            Feat::LogBand12 => (0..PROD_DIM)
                .map(|b| {
                    let s = self.log_edges[b];
                    let e = self.log_edges[b + 1].max(s + 1).min(power.len());
                    let energy = power[s..e].iter().sum::<f32>() / (e - s).max(1) as f32;
                    (energy + 1e-8).ln()
                })
                .collect(),
            Feat::Mel24 | Feat::Mfcc13 | Feat::Mfcc13D => {
                let log_mel: Vec<f32> = self
                    .mel
                    .iter()
                    .map(|filter| {
                        let e: f32 = filter.iter().zip(power).map(|(w, p)| w * p).sum();
                        (e + 1e-8).ln()
                    })
                    .collect();
                if feat == Feat::Mel24 {
                    log_mel
                } else {
                    dct2(&log_mel, 13)
                }
            }
        }
    }
}

/// 按本段音频自己的底噪定门限（20 分位当底噪，压在底噪与峰值之间）。
fn adaptive_gate(samples: &[f32]) -> f32 {
    let mut levels = Vec::new();
    let mut offset = 0;
    while offset + FRAME <= samples.len() {
        let c = &samples[offset..offset + FRAME];
        levels.push((c.iter().map(|s| s * s).sum::<f32>() / FRAME as f32).sqrt());
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

/// 能量最强的一段（窗口长度可配）。
fn crop_ms(samples: &[f32], ms: usize) -> Vec<f32> {
    let window = 16 * ms;
    const STEP: usize = 1_600;
    if samples.len() <= window {
        return samples.to_vec();
    }
    let (mut best_start, mut best_energy) = (0usize, f32::NEG_INFINITY);
    let mut start = 0;
    while start + window <= samples.len() {
        let e: f32 = samples[start..start + window].iter().map(|v| v * v).sum();
        if e > best_energy {
            best_energy = e;
            best_start = start;
        }
        start += STEP;
    }
    samples[best_start..best_start + window].to_vec()
}

fn cmvn(seq: &mut [Vec<f32>]) {
    if seq.is_empty() {
        return;
    }
    let dim = seq[0].len();
    let n = seq.len() as f32;
    let mut mean = vec![0.0; dim];
    for f in seq.iter() {
        for (m, v) in mean.iter_mut().zip(f) {
            *m += v;
        }
    }
    for m in &mut mean {
        *m /= n;
    }
    let mut var = vec![0.0; dim];
    for f in seq.iter() {
        for ((s, v), m) in var.iter_mut().zip(f).zip(&mean) {
            *s += (v - m) * (v - m);
        }
    }
    for s in &mut var {
        *s = (*s / n).sqrt().max(1e-5);
    }
    for f in seq.iter_mut() {
        for ((v, m), s) in f.iter_mut().zip(&mean).zip(&var) {
            *v = (*v - m) / s;
        }
    }
}

fn add_delta(seq: &[Vec<f32>]) -> Vec<Vec<f32>> {
    seq.iter()
        .enumerate()
        .map(|(i, f)| {
            let prev = &seq[i.saturating_sub(1)];
            let next = &seq[(i + 1).min(seq.len() - 1)];
            let mut out = f.clone();
            for (p, n) in prev.iter().zip(next) {
                out.push((n - p) * 0.5);
            }
            out
        })
        .collect()
}

fn l2_normalize(seq: &mut [Vec<f32>]) {
    for f in seq.iter_mut() {
        let norm = f.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 1e-6 {
            for v in f.iter_mut() {
                *v /= norm;
            }
        }
    }
}

fn resample(seq: &[Vec<f32>], target: usize) -> Vec<Vec<f32>> {
    if seq.is_empty() || target == 0 {
        return Vec::new();
    }
    if seq.len() == 1 {
        return vec![seq[0].clone(); target];
    }
    (0..target)
        .map(|i| {
            let pos = i as f32 * (seq.len() - 1) as f32 / (target - 1).max(1) as f32;
            let (l, r) = (pos.floor() as usize, pos.ceil() as usize);
            let frac = pos - l as f32;
            seq[l]
                .iter()
                .zip(&seq[r])
                .map(|(a, b)| a + (b - a) * frac)
                .collect()
        })
        .collect()
}

// ============================ 配置 ============================

#[derive(Clone, Copy)]
struct Cfg {
    name: &'static str,
    adaptive: bool,
    crop: bool,
    /// 裁剪窗口长度（毫秒），仅在 crop=true 时生效
    crop_ms: usize,
    /// None = 原生长度，不重采样
    resample: Option<usize>,
    feat: Feat,
    cmvn: bool,
    /// Sakoe-Chiba 带宽（相对比例），None = 无约束
    band: Option<f32>,
    multi_template: bool,
}

const BASE: Cfg = Cfg {
    name: "产品原样",
    adaptive: false,
    crop: false,
    crop_ms: 1600,
    resample: Some(PROD_FRAMES),
    feat: Feat::LogBand12,
    cmvn: false,
    band: None,
    multi_template: false,
};

fn sequence(ex: &Extractor, cfg: &Cfg, samples: &[f32]) -> Vec<Vec<f32>> {
    let owned;
    let input = if cfg.crop {
        owned = crop_ms(samples, cfg.crop_ms);
        &owned[..]
    } else {
        samples
    };
    if input.len() < FRAME {
        return Vec::new();
    }
    let gate = if cfg.adaptive {
        adaptive_gate(input)
    } else {
        0.003
    };
    let mut seq = Vec::new();
    let mut offset = 0;
    while offset + FRAME <= input.len() {
        let chunk = &input[offset..offset + FRAME];
        let rms = (chunk.iter().map(|s| s * s).sum::<f32>() / FRAME as f32).sqrt();
        if rms >= gate {
            seq.push(ex.frame_feature(&ex.power(chunk), cfg.feat));
        }
        offset += HOP;
    }
    if seq.is_empty() {
        return seq;
    }
    if cfg.cmvn {
        cmvn(&mut seq);
    }
    if cfg.feat == Feat::Mfcc13D {
        seq = add_delta(&seq);
    }
    l2_normalize(&mut seq);
    if let Some(target) = cfg.resample {
        seq = resample(&seq, target);
    }
    seq
}

// ============================ DTW ============================

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na <= f32::EPSILON || nb <= f32::EPSILON {
        return 1.0;
    }
    (1.0 - (dot / (na.sqrt() * nb.sqrt())).clamp(-1.0, 1.0)).clamp(0.0, 2.0)
}

/// 产品 `dtw_similarity` 的可配置复刻。`band` = Sakoe-Chiba 带宽比例。
fn dtw(left: &[Vec<f32>], right: &[Vec<f32>], band: Option<f32>) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let (n, m) = (left.len(), right.len());
    let limit = band.map(|b| (b * m.max(n) as f32).max(1.0));
    let mut prev = vec![f32::INFINITY; m + 1];
    prev[0] = 0.0;
    for (i, lf) in left.iter().enumerate() {
        let mut cur = vec![f32::INFINITY; m + 1];
        for (j, rf) in right.iter().enumerate() {
            if let Some(limit) = limit {
                // 对角线偏离超过带宽的格子直接跳过
                let expected = i as f32 * m as f32 / n as f32;
                if (j as f32 - expected).abs() > limit {
                    continue;
                }
            }
            let cost = cosine_distance(lf, rf);
            cur[j + 1] = cost + prev[j + 1].min(cur[j]).min(prev[j]);
        }
        prev = cur;
    }
    let total = prev[m];
    if !total.is_finite() {
        return 0.0;
    }
    (-4.0 * (total / (n + m) as f32)).exp().clamp(0.0, 1.0)
}

/// 模板：单模板（逐帧平均）或多模板（取最大相似度）。
struct Template {
    sequences: Vec<Vec<Vec<f32>>>,
    multi: bool,
}

impl Template {
    fn build(cfg: &Cfg, enrolled: Vec<Vec<Vec<f32>>>) -> Option<Self> {
        let valid: Vec<_> = enrolled.into_iter().filter(|s| !s.is_empty()).collect();
        if valid.is_empty() {
            return None;
        }
        if cfg.multi_template {
            return Some(Self { sequences: valid, multi: true });
        }
        // 逐帧平均要求等长；原生长度时先统一到中位数长度
        let target = cfg.resample.unwrap_or_else(|| {
            let mut lens: Vec<_> = valid.iter().map(|s| s.len()).collect();
            lens.sort_unstable();
            lens[lens.len() / 2]
        });
        let aligned: Vec<_> = valid.iter().map(|s| resample(s, target)).collect();
        let dim = aligned[0][0].len();
        let mut avg = vec![vec![0.0; dim]; target];
        for seq in &aligned {
            for (t, s) in avg.iter_mut().zip(seq) {
                for (a, b) in t.iter_mut().zip(s) {
                    *a += b;
                }
            }
        }
        for t in &mut avg {
            for a in t.iter_mut() {
                *a /= aligned.len() as f32;
            }
        }
        Some(Self { sequences: vec![avg], multi: false })
    }

    fn score(&self, cfg: &Cfg, probe: &[Vec<f32>]) -> f32 {
        if probe.is_empty() {
            return 0.0;
        }
        let mut best = 0.0_f32;
        for seq in &self.sequences {
            best = best.max(dtw(probe, seq, cfg.band));
        }
        best
    }

    /// 生产阈值策略：min(注册样本自相似) - 0.08，夹到 [0.52, 0.88]
    fn production_threshold(&self, cfg: &Cfg, enrolled: &[Vec<Vec<f32>>]) -> f32 {
        let min = enrolled
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| self.score(cfg, s))
            .fold(1.0_f32, f32::min);
        (min - 0.08).clamp(0.52, 0.88)
    }
}

// ============================ 指标 ============================

#[derive(Default, Clone)]
struct Bucket {
    pos: Vec<f32>,
    imp: Vec<f32>,
    cfz: Vec<f32>,
    free: Vec<f32>,
}

#[derive(Default)]
struct Scores {
    all: Bucket,
    per_cond: Vec<Bucket>,
    /// 生产阈值策略下的通过计数
    prod_pos: (u32, u32),
    prod_imp: (u32, u32),
    prod_cfz: (u32, u32),
    prod_free: (u32, u32),
    /// 每组的生产阈值，用于诊断
    thresholds: Vec<f32>,
}

fn auc(pos: &[f32], neg: &[f32]) -> f64 {
    if pos.is_empty() || neg.is_empty() {
        return f64::NAN;
    }
    let mut wins = 0.0_f64;
    for p in pos {
        for n in neg {
            wins += if p > n {
                1.0
            } else if (p - n).abs() < f32::EPSILON {
                0.5
            } else {
                0.0
            };
        }
    }
    wins / (pos.len() * neg.len()) as f64
}

/// 在把误触率压到 `far` 的全局阈值下，正样本通过率是多少。
fn tpr_at_far(pos: &[f32], neg: &[f32], far: f64) -> (f64, f32) {
    if pos.is_empty() || neg.is_empty() {
        return (f64::NAN, 0.0);
    }
    let mut sorted = neg.to_vec();
    sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());
    let allow = ((neg.len() as f64) * far).floor() as usize;
    let threshold = if allow >= sorted.len() {
        sorted[sorted.len() - 1]
    } else {
        sorted[allow]
    };
    let hits = pos.iter().filter(|v| **v > threshold).count();
    (hits as f64 / pos.len() as f64, threshold)
}

fn pct(part: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        part as f64 * 100.0 / total as f64
    }
}

// ============================ 主实验 ============================

fn conditions() -> Vec<Condition> {
    let noise = load_noise_by_category();
    let pick = |cat: &str, kind: &str| -> Option<PathBuf> {
        noise.get(cat).and_then(|paths| {
            paths
                .iter()
                .find(|p| {
                    p.file_stem()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| s.contains(kind))
                })
                .cloned()
        })
    };
    let mut out = vec![Condition::clean()];
    if let Some(path) = pick("crowd", "test") {
        out.push(Condition {
            id: "crowd@10dB".to_owned(),
            speed: 1.0,
            noise: Some(path),
            noise_category: "crowd".to_owned(),
            snr_db: 10.0,
            gain: 1.0,
        });
    }
    if let Some(path) = pick("traffic", "train") {
        out.push(Condition {
            id: "traffic@5dB".to_owned(),
            speed: 1.0,
            noise: Some(path),
            noise_category: "traffic".to_owned(),
            snr_db: 5.0,
            gain: 1.0,
        });
    }
    out
}

fn configs() -> Vec<Cfg> {
    let mut v = Vec::new();
    v.push(BASE);

    // 单项消融：每次只动一个开关
    v.push(Cfg { name: "A 自适应门限", adaptive: true, ..BASE });
    v.push(Cfg { name: "B 去掉64帧重采样", resample: None, ..BASE });
    v.push(Cfg { name: "C1 mel24", feat: Feat::Mel24, ..BASE });
    v.push(Cfg { name: "C2 MFCC13", feat: Feat::Mfcc13, ..BASE });
    v.push(Cfg { name: "C3 MFCC13+delta", feat: Feat::Mfcc13D, ..BASE });
    v.push(Cfg { name: "D CMVN", cmvn: true, ..BASE });
    v.push(Cfg { name: "E DTW带约束0.2", band: Some(0.2), ..BASE });
    v.push(Cfg { name: "F 多模板取最大", multi_template: true, ..BASE });
    v.push(Cfg { name: "G 能量裁剪", crop: true, ..BASE });

    // 递进组合
    v.push(Cfg { name: "AB", adaptive: true, resample: None, ..BASE });
    v.push(Cfg {
        name: "AB+MFCC",
        adaptive: true,
        resample: None,
        feat: Feat::Mfcc13,
        ..BASE
    });
    v.push(Cfg {
        name: "AB+MFCC+CMVN",
        adaptive: true,
        resample: None,
        feat: Feat::Mfcc13,
        cmvn: true,
        ..BASE
    });
    v.push(Cfg {
        name: "AB+MFCCd+CMVN",
        adaptive: true,
        resample: None,
        feat: Feat::Mfcc13D,
        cmvn: true,
        ..BASE
    });
    v.push(Cfg {
        name: "AB+MFCCd+CMVN+带",
        adaptive: true,
        resample: None,
        feat: Feat::Mfcc13D,
        cmvn: true,
        band: Some(0.2),
        ..BASE
    });
    v.push(Cfg {
        name: "全开(+多模板)",
        adaptive: true,
        resample: None,
        feat: Feat::Mfcc13D,
        cmvn: true,
        band: Some(0.2),
        multi_template: true,
        crop: false,
        crop_ms: 1600,
    });
    v
}

/// 第二轮：围绕胜出配置做精细消融
fn configs_round2() -> Vec<Cfg> {
    let best = Cfg {
        name: "基准 AB+MFCC+CMVN",
        adaptive: true,
        crop: false,
        crop_ms: 1600,
        resample: None,
        feat: Feat::Mfcc13,
        cmvn: true,
        band: None,
        multi_template: false,
    };
    let mut v = vec![BASE, best];
    // 逐项拆掉，看谁是必需的
    v.push(Cfg { name: "  -去掉自适应门限", adaptive: false, ..best });
    v.push(Cfg { name: "  -恢复64帧重采样", resample: Some(PROD_FRAMES), ..best });
    v.push(Cfg { name: "  -去掉CMVN", cmvn: false, ..best });
    v.push(Cfg { name: "  -换回mel24", feat: Feat::Mel24, ..best });
    v.push(Cfg { name: "  -换回12对数带", feat: Feat::LogBand12, ..best });
    // 再加东西
    v.push(Cfg { name: "  +delta", feat: Feat::Mfcc13D, ..best });
    v.push(Cfg { name: "  +能量裁剪", crop: true, ..best });
    v.push(Cfg { name: "  +多模板", multi_template: true, ..best });
    v.push(Cfg { name: "  +带0.1", band: Some(0.1), ..best });
    v.push(Cfg { name: "  +带0.3", band: Some(0.3), ..best });
    v.push(Cfg { name: "  +裁剪+多模板", crop: true, multi_template: true, ..best });
    v
}

struct Outcome {
    scores: Vec<Scores>,
    conditions: Vec<Condition>,
}

fn run_experiment(cfgs: &[Cfg]) -> Outcome {
    let groups = load_groups().expect("corpus");
    assert!(!groups.is_empty(), "语料为空");
    let free_paths = load_free_speech();
    let conditions = conditions();
    let ex = Extractor::new();

    println!(
        "分组 {} / 自由语音 {} / 条件 {} / 配置 {}",
        groups.len(),
        free_paths.len(),
        conditions.len(),
        cfgs.len()
    );

    let mut all: Vec<Scores> = (0..cfgs.len())
        .map(|_| Scores {
            per_cond: vec![Bucket::default(); conditions.len()],
            ..Scores::default()
        })
        .collect();

    // 每组 × 每配置：模板 + 生产阈值。自由语音复用同一批模板。
    let mut group_templates: Vec<Vec<(Option<Template>, f32)>> = Vec::new();
    for group in &groups {
        let enroll = group.by_role(Role::Enroll);
        let audio: Vec<Vec<f32>> = enroll
            .iter()
            .take(3)
            .filter_map(|u| read_wav_16k(&u.path).ok())
            .collect();
        if audio.len() < 3 {
            group_templates.push(Vec::new());
            continue;
        }
        let mut per_cfg = Vec::new();
        for (ci, cfg) in cfgs.iter().enumerate() {
            let seqs: Vec<_> = audio.iter().map(|a| sequence(&ex, cfg, a)).collect();
            let tmpl = Template::build(cfg, seqs.clone());
            let th = tmpl
                .as_ref()
                .map(|t| t.production_threshold(cfg, &seqs))
                .unwrap_or(0.7);
            all[ci].thresholds.push(th);
            per_cfg.push((tmpl, th));
        }
        group_templates.push(per_cfg);
    }

    for (gi, group) in groups.iter().enumerate() {
        if group_templates[gi].is_empty() {
            continue;
        }
        for (cond_index, cond) in conditions.iter().enumerate() {
            let mut rng = Rng::new(0xC0FFEE ^ ((cond_index as u64) << 8));
            for role in [Role::Positive, Role::Impostor, Role::Confusable] {
                for utt in group.by_role(role) {
                    let Ok(raw) = read_wav_16k(&utt.path) else { continue };
                    let Ok(audio) = apply(&raw, cond, &mut rng) else { continue };
                    for (ci, cfg) in cfgs.iter().enumerate() {
                        let Some((Some(tmpl), th)) = group_templates[gi].get(ci) else { continue };
                        let score = tmpl.score(cfg, &sequence(&ex, cfg, &audio));
                        let pass = score >= *th;
                        let s = &mut all[ci];
                        match role {
                            Role::Positive => {
                                s.all.pos.push(score);
                                s.per_cond[cond_index].pos.push(score);
                                s.prod_pos.1 += 1;
                                s.prod_pos.0 += pass as u32;
                            }
                            Role::Impostor => {
                                s.all.imp.push(score);
                                s.per_cond[cond_index].imp.push(score);
                                s.prod_imp.1 += 1;
                                s.prod_imp.0 += pass as u32;
                            }
                            Role::Confusable => {
                                s.all.cfz.push(score);
                                s.per_cond[cond_index].cfz.push(score);
                                s.prod_cfz.1 += 1;
                                s.prod_cfz.0 += pass as u32;
                            }
                            Role::Enroll => {}
                        }
                    }
                }
            }
        }
    }

    for (cond_index, cond) in conditions.iter().enumerate() {
        let mut rng = Rng::new(0xF2EE ^ ((cond_index as u64) << 8));
        for path in &free_paths {
            let Ok(raw) = read_wav_16k(path) else { continue };
            let Ok(audio) = apply(&raw, cond, &mut rng) else { continue };
            for (ci, cfg) in cfgs.iter().enumerate() {
                let seq = sequence(&ex, cfg, &audio);
                if seq.is_empty() {
                    continue;
                }
                for per_cfg in &group_templates {
                    let Some((Some(tmpl), th)) = per_cfg.get(ci) else { continue };
                    let score = tmpl.score(cfg, &seq);
                    let s = &mut all[ci];
                    s.all.free.push(score);
                    s.per_cond[cond_index].free.push(score);
                    s.prod_free.1 += 1;
                    s.prod_free.0 += (score >= *th) as u32;
                }
            }
        }
    }

    Outcome { scores: all, conditions }
}

fn negatives(b: &Bucket) -> Vec<f32> {
    let mut n = b.cfz.clone();
    n.extend_from_slice(&b.free);
    n
}

fn report(cfgs: &[Cfg], out: &Outcome) {
    println!("\n========== 判别力：抗日常说话（正 vs 自由语音）==========");
    println!("{:<22}{:>8}{:>12}{:>12}{:>12}", "配置", "AUC", "TPR@FAR5%", "TPR@FAR10%", "TPR@FAR20%");
    for (ci, cfg) in cfgs.iter().enumerate() {
        let b = &out.scores[ci].all;
        println!(
            "{:<22}{:>8.4}{:>11.1}%{:>11.1}%{:>11.1}%",
            cfg.name,
            auc(&b.pos, &b.free),
            tpr_at_far(&b.pos, &b.free, 0.05).0 * 100.0,
            tpr_at_far(&b.pos, &b.free, 0.10).0 * 100.0,
            tpr_at_far(&b.pos, &b.free, 0.20).0 * 100.0
        );
    }

    println!("\n========== 判别力：抗近音对立词（正 vs 对立）==========");
    println!("{:<22}{:>8}{:>12}{:>12}{:>12}", "配置", "AUC", "TPR@FAR5%", "TPR@FAR10%", "TPR@FAR20%");
    for (ci, cfg) in cfgs.iter().enumerate() {
        let b = &out.scores[ci].all;
        println!(
            "{:<22}{:>8.4}{:>11.1}%{:>11.1}%{:>11.1}%",
            cfg.name,
            auc(&b.pos, &b.cfz),
            tpr_at_far(&b.pos, &b.cfz, 0.05).0 * 100.0,
            tpr_at_far(&b.pos, &b.cfz, 0.10).0 * 100.0,
            tpr_at_far(&b.pos, &b.cfz, 0.20).0 * 100.0
        );
    }

    println!("\n========== 分条件 TPR@自由语音FAR10%（抗噪）==========");
    print!("{:<22}", "配置");
    for c in &out.conditions {
        print!("{:>14}", c.id);
    }
    println!();
    for (ci, cfg) in cfgs.iter().enumerate() {
        print!("{:<22}", cfg.name);
        for k in 0..out.conditions.len() {
            let b = &out.scores[ci].per_cond[k];
            print!("{:>13.1}%", tpr_at_far(&b.pos, &b.free, 0.10).0 * 100.0);
        }
        println!();
    }

    println!("\n========== 生产阈值策略下的实际工作点 ==========");
    println!(
        "{:<22}{:>9}{:>9}{:>9}{:>10}{:>16}",
        "配置", "识别率", "对立词", "自由语音", "他人", "阈值 min~max"
    );
    for (ci, cfg) in cfgs.iter().enumerate() {
        let s = &out.scores[ci];
        let lo = s.thresholds.iter().cloned().fold(f32::INFINITY, f32::min);
        let hi = s.thresholds.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        println!(
            "{:<22}{:>8.1}%{:>8.1}%{:>8.1}%{:>9.1}%{:>9.3}~{:.3}",
            cfg.name,
            pct(s.prod_pos.0, s.prod_pos.1),
            pct(s.prod_cfz.0, s.prod_cfz.1),
            pct(s.prod_free.0, s.prod_free.1),
            pct(s.prod_imp.0, s.prod_imp.1),
            lo,
            hi
        );
    }

    let b = &out.scores[0].all;
    println!(
        "\n样本量：正 {} / 他人 {} / 对立 {} / 自由 {}",
        b.pos.len(),
        b.imp.len(),
        b.cfz.len(),
        b.free.len()
    );
}

#[test]
#[ignore = "手动实验：voiceMatch 消融（第一轮，粗筛）"]
fn voice_match_ablation() {
    let cfgs = configs();
    let out = run_experiment(&cfgs);
    report(&cfgs, &out);
}

#[test]
#[ignore = "手动实验：voiceMatch 消融（第二轮，精细）"]
fn voice_match_ablation_round2() {
    let cfgs = configs_round2();
    let out = run_experiment(&cfgs);
    report(&cfgs, &out);
}

// ============================ 第三轮：阈值策略 ============================

/// 从 babble 噪声切出的负样本队列（模拟「随应用分发的负样本集」）。
fn cohort_audio() -> Vec<Vec<f32>> {
    let noise = load_noise_by_category();
    let mut out = Vec::new();
    let Some(paths) = noise.get("crowd") else { return out };
    for path in paths.iter().filter(|p| {
        p.file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.contains("babble") && s.contains("train"))
    }) {
        let Ok(samples) = read_wav_16k(path) else { continue };
        let window = 16_000 * 5;
        let mut start = 0;
        while start + window <= samples.len() && out.len() < 24 {
            out.push(samples[start..start + window].to_vec());
            start += window;
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Policy {
    /// 生产现状：min(注册自相似) - 0.08，夹到 [0.52, 0.88]
    Production,
    /// 队列分位：阈值 = 负样本队列得分的 q 分位
    CohortQuantile,
    /// 队列 z 归一 + 全局阈值：(raw - mean) / std >= z
    CohortZNorm,
    /// 取两者较大：既不低于队列分位，也不高于注册自相似留出的余量
    Hybrid,
}

struct Calibrated {
    threshold: f32,
    /// z 归一用
    mean: f32,
    std: f32,
    policy: Policy,
}

impl Calibrated {
    fn accept(&self, raw: f32) -> bool {
        match self.policy {
            Policy::CohortZNorm => (raw - self.mean) / self.std >= self.threshold,
            _ => raw >= self.threshold,
        }
    }
}

fn calibrate(
    policy: Policy,
    cfg: &Cfg,
    tmpl: &Template,
    enrolled: &[Vec<Vec<f32>>],
    cohort: &[Vec<Vec<f32>>],
    param: f32,
) -> Calibrated {
    let mut cohort_scores: Vec<f32> = cohort.iter().map(|s| tmpl.score(cfg, s)).collect();
    cohort_scores.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = cohort_scores.iter().sum::<f32>() / cohort_scores.len().max(1) as f32;
    let var = cohort_scores
        .iter()
        .map(|v| (v - mean) * (v - mean))
        .sum::<f32>()
        / cohort_scores.len().max(1) as f32;
    let std = var.sqrt().max(1e-4);
    let self_min = enrolled
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| tmpl.score(cfg, s))
        .fold(1.0_f32, f32::min);
    let quantile = |q: f32| -> f32 {
        if cohort_scores.is_empty() {
            return 0.7;
        }
        let idx = ((cohort_scores.len() - 1) as f32 * q).round() as usize;
        cohort_scores[idx]
    };
    let threshold = match policy {
        Policy::Production => (self_min - 0.08).clamp(0.52, 0.88),
        Policy::CohortQuantile => quantile(param),
        Policy::CohortZNorm => param,
        Policy::Hybrid => quantile(param).min(self_min - 0.02),
    };
    Calibrated { threshold, mean, std, policy }
}

#[test]
#[ignore = "手动实验：voiceMatch 阈值策略"]
fn voice_match_threshold_policy() {
    let groups = load_groups().expect("corpus");
    let free_paths = load_free_speech();
    let conditions = conditions();
    let ex = Extractor::new();
    let cohort_raw = cohort_audio();
    println!("负样本队列：{} 段 5 秒 babble", cohort_raw.len());
    assert!(!cohort_raw.is_empty(), "缺少 babble 噪声素材");

    let best = Cfg {
        name: "AB+MFCC+CMVN+裁剪",
        adaptive: true,
        crop: true,
        crop_ms: 1600,
        resample: None,
        feat: Feat::Mfcc13,
        cmvn: true,
        band: None,
        multi_template: false,
    };
    let minimal = Cfg { name: "12带+CMVN+自适应+裁剪", feat: Feat::LogBand12, ..best };
    let cfgs = [BASE, minimal, best];

    // (策略, 参数, 显示名)
    let policies: Vec<(Policy, f32, String)> = vec![
        (Policy::Production, 0.0, "生产现状".to_owned()),
        (Policy::CohortQuantile, 1.00, "队列最大值".to_owned()),
        (Policy::CohortQuantile, 0.95, "队列95分位".to_owned()),
        (Policy::CohortQuantile, 0.80, "队列80分位".to_owned()),
        (Policy::CohortZNorm, 4.0, "z>=4.0".to_owned()),
        (Policy::CohortZNorm, 3.0, "z>=3.0".to_owned()),
        (Policy::CohortZNorm, 2.0, "z>=2.0".to_owned()),
        (Policy::Hybrid, 1.00, "混合(队列max/自相似)".to_owned()),
    ];

    // [cfg][policy] -> (pos_pass, pos_total, cfz_pass, cfz_total, free_pass, free_total)
    let mut tally = vec![vec![[0u32; 6]; policies.len()]; cfgs.len()];
    let mut all_templates: Vec<Vec<(Option<Template>, Vec<Calibrated>)>> = Vec::new();

    for group in &groups {
        let enroll = group.by_role(Role::Enroll);
        let audio: Vec<Vec<f32>> = enroll
            .iter()
            .take(3)
            .filter_map(|u| read_wav_16k(&u.path).ok())
            .collect();
        if audio.len() < 3 {
            all_templates.push(Vec::new());
            continue;
        }
        let mut per_cfg = Vec::new();
        for cfg in &cfgs {
            let seqs: Vec<_> = audio.iter().map(|a| sequence(&ex, cfg, a)).collect();
            let cohort: Vec<_> = cohort_raw.iter().map(|a| sequence(&ex, cfg, a)).collect();
            let tmpl = Template::build(cfg, seqs.clone());
            let cal = match &tmpl {
                Some(t) => policies
                    .iter()
                    .map(|(p, param, _)| calibrate(*p, cfg, t, &seqs, &cohort, *param))
                    .collect(),
                None => Vec::new(),
            };
            per_cfg.push((tmpl, cal));
        }
        all_templates.push(per_cfg);
    }

    for (gi, group) in groups.iter().enumerate() {
        if all_templates[gi].is_empty() {
            continue;
        }
        for (cond_index, cond) in conditions.iter().enumerate() {
            let mut rng = Rng::new(0xC0FFEE ^ ((cond_index as u64) << 8));
            for role in [Role::Positive, Role::Confusable] {
                for utt in group.by_role(role) {
                    let Ok(raw) = read_wav_16k(&utt.path) else { continue };
                    let Ok(audio) = apply(&raw, cond, &mut rng) else { continue };
                    for (ci, cfg) in cfgs.iter().enumerate() {
                        let (Some(tmpl), cals) = (&all_templates[gi][ci].0, &all_templates[gi][ci].1)
                        else {
                            continue;
                        };
                        let score = tmpl.score(cfg, &sequence(&ex, cfg, &audio));
                        for (pi, cal) in cals.iter().enumerate() {
                            let slot = &mut tally[ci][pi];
                            let base = if role == Role::Positive { 0 } else { 2 };
                            slot[base + 1] += 1;
                            slot[base] += cal.accept(score) as u32;
                        }
                    }
                }
            }
        }
    }

    for (cond_index, cond) in conditions.iter().enumerate() {
        let mut rng = Rng::new(0xF2EE ^ ((cond_index as u64) << 8));
        for path in &free_paths {
            let Ok(raw) = read_wav_16k(path) else { continue };
            let Ok(audio) = apply(&raw, cond, &mut rng) else { continue };
            for (ci, cfg) in cfgs.iter().enumerate() {
                let seq = sequence(&ex, cfg, &audio);
                if seq.is_empty() {
                    continue;
                }
                for per_cfg in &all_templates {
                    let Some((Some(tmpl), cals)) = per_cfg.get(ci) else { continue };
                    let score = tmpl.score(cfg, &seq);
                    for (pi, cal) in cals.iter().enumerate() {
                        let slot = &mut tally[ci][pi];
                        slot[5] += 1;
                        slot[4] += cal.accept(score) as u32;
                    }
                }
            }
        }
    }

    for (ci, cfg) in cfgs.iter().enumerate() {
        println!("\n=== {} ===", cfg.name);
        println!(
            "{:<22}{:>10}{:>10}{:>10}{:>10}",
            "阈值策略", "识别率", "对立词", "自由语音", "误触率"
        );
        for (pi, (_, _, name)) in policies.iter().enumerate() {
            let t = tally[ci][pi];
            let fh = t[2] + t[4];
            let ft = t[3] + t[5];
            println!(
                "{:<22}{:>9.1}%{:>9.1}%{:>9.1}%{:>9.1}%",
                name,
                pct(t[0], t[1]),
                pct(t[2], t[3]),
                pct(t[4], t[5]),
                pct(fh, ft)
            );
        }
    }
}

// ============================ 第四轮：KWS + DTW 级联端到端 ============================
//
// 生产里 voiceMatch 的 overall = phrase_score.min(mode_score)（scoring/mod.rs:125），
// 也就是「先过 KWS，再过 DTW」。只测 DTW 会高估对立词的危害——
// 对立词得先骗过 KWS 才可能走到 DTW。

use dictatingme_runtime::audio::AudioFrame;
use support::harness::TestEnv;

const KWS_FRAME_SAMPLES: usize = 1_600;
const KWS_FRAME_MS: u64 = 100;

/// 复刻 harness::detect 的 KWS 部分：逐帧喂，命中过就算触发。
fn kws_fires(engine: &mut dictatingme_runtime::models::EvokeModelEngine, samples: &[f32]) -> bool {
    engine.reset();
    let mut fired = false;
    for (index, chunk) in samples.chunks(KWS_FRAME_SAMPLES).enumerate() {
        let frame = AudioFrame {
            samples: chunk
                .iter()
                .map(|s| (s.clamp(-1.0, 1.0) * 32_767.0) as i16)
                .collect(),
            sample_rate: 16_000,
            timestamp_ms: index as u64 * KWS_FRAME_MS,
        };
        if engine.process_frame(&frame).is_some() {
            fired = true;
        }
    }
    fired
}

#[test]
#[ignore = "手动实验：voiceMatch 级联端到端"]
fn voice_match_cascade() {
    let env = TestEnv::prepare().expect("test env");
    let groups = load_groups().expect("corpus");
    let free_paths = load_free_speech();
    let conditions = conditions();
    let ex = Extractor::new();
    let cohort_raw = cohort_audio();

    let best = Cfg {
        name: "新特征",
        adaptive: true,
        crop: true,
        crop_ms: 1600,
        resample: None,
        feat: Feat::Mfcc13,
        cmvn: true,
        band: None,
        multi_template: false,
    };
    let minimal = Cfg { name: "新特征(保12维)", feat: Feat::LogBand12, ..best };

    // (配置, 阈值策略, 参数, 名称)
    let lanes: Vec<(Cfg, Policy, f32, &str)> = vec![
        (BASE, Policy::Production, 0.0, "现状：原特征+原阈值"),
        (minimal, Policy::Production, 0.0, "新特征(12维)+原阈值"),
        (minimal, Policy::CohortQuantile, 1.00, "新特征(12维)+队列max"),
        (best, Policy::Production, 0.0, "新特征(MFCC)+原阈值"),
        (best, Policy::CohortQuantile, 1.00, "新特征(MFCC)+队列max"),
        (best, Policy::CohortQuantile, 0.95, "新特征(MFCC)+队列95%"),
    ];

    // [lane] -> [pos_pass, pos_total, cfz_pass, cfz_total, free_pass, free_total]
    let mut tally = vec![[0u32; 6]; lanes.len()];
    // KWS 单级的计数，用来把两级拆开看
    let mut kws_only = [0u32; 6];

    // 每组：KWS 引擎 + 每 lane 的模板与阈值
    for group in &groups {
        let enroll = group.by_role(Role::Enroll);
        let audio: Vec<Vec<f32>> = enroll
            .iter()
            .take(3)
            .filter_map(|u| read_wav_16k(&u.path).ok())
            .collect();
        if audio.len() < 3 {
            continue;
        }
        let Ok(mut engine) = env.new_spotter(&group.phrase) else {
            eprintln!("[skip] {} 无法建 KWS", group.id);
            continue;
        };

        let mut calibrated = Vec::new();
        for (cfg, policy, param, _) in &lanes {
            let seqs: Vec<_> = audio.iter().map(|a| sequence(&ex, cfg, a)).collect();
            let cohort: Vec<_> = cohort_raw.iter().map(|a| sequence(&ex, cfg, a)).collect();
            let tmpl = Template::build(cfg, seqs.clone());
            let cal = tmpl
                .as_ref()
                .map(|t| calibrate(*policy, cfg, t, &seqs, &cohort, *param));
            calibrated.push((tmpl, cal));
        }

        for (cond_index, cond) in conditions.iter().enumerate() {
            let mut rng = Rng::new(0xCA5CADE ^ ((cond_index as u64) << 8));
            for role in [Role::Positive, Role::Confusable] {
                for utt in group.by_role(role) {
                    let Ok(raw) = read_wav_16k(&utt.path) else { continue };
                    let Ok(audio) = apply(&raw, cond, &mut rng) else { continue };
                    let fired = kws_fires(&mut engine, &audio);
                    let base = if role == Role::Positive { 0 } else { 2 };
                    kws_only[base + 1] += 1;
                    kws_only[base] += fired as u32;
                    for (li, (cfg, _, _, _)) in lanes.iter().enumerate() {
                        let (Some(tmpl), Some(cal)) = (&calibrated[li].0, &calibrated[li].1) else {
                            continue;
                        };
                        let accept = fired
                            && cal.accept(tmpl.score(cfg, &sequence(&ex, cfg, &audio)));
                        tally[li][base + 1] += 1;
                        tally[li][base] += accept as u32;
                    }
                }
            }
            // 自由语音：每组都要过一遍（模板是分组的）
            let mut rng = Rng::new(0xF2EECA5 ^ ((cond_index as u64) << 8));
            for path in &free_paths {
                let Ok(raw) = read_wav_16k(path) else { continue };
                let Ok(audio) = apply(&raw, cond, &mut rng) else { continue };
                let fired = kws_fires(&mut engine, &audio);
                kws_only[5] += 1;
                kws_only[4] += fired as u32;
                for (li, (cfg, _, _, _)) in lanes.iter().enumerate() {
                    let (Some(tmpl), Some(cal)) = (&calibrated[li].0, &calibrated[li].1) else {
                        continue;
                    };
                    let accept =
                        fired && cal.accept(tmpl.score(cfg, &sequence(&ex, cfg, &audio)));
                    tally[li][5] += 1;
                    tally[li][4] += accept as u32;
                }
            }
        }
    }

    println!("\n========== KWS 单级（voiceMatch 的天花板）==========");
    println!(
        "命中率 正 {:.1}%  对立 {:.1}%  自由 {:.1}%",
        pct(kws_only[0], kws_only[1]),
        pct(kws_only[2], kws_only[3]),
        pct(kws_only[4], kws_only[5])
    );

    println!("\n========== 级联端到端（KWS 且 DTW）==========");
    println!(
        "{:<26}{:>10}{:>10}{:>10}",
        "方案", "识别率", "对立词误触", "自由语音误触"
    );
    for (li, (_, _, _, name)) in lanes.iter().enumerate() {
        let t = tally[li];
        println!(
            "{:<26}{:>9.1}%{:>9.1}%{:>9.1}%",
            name,
            pct(t[0], t[1]),
            pct(t[2], t[3]),
            pct(t[4], t[5])
        );
    }
    println!(
        "\n样本量：正 {} / 对立 {} / 自由 {}（= 45 条 × 20 组 × 3 条件）",
        tally[0][1], tally[0][3], tally[0][5]
    );
}

// ============================ 第五轮：叠加 KWS beam=8 ============================
//
// 上一份文档已实测：max_active_paths 从 crate 默认 4 放宽到 8，
// KWS 召回 68.8% -> 81.2%（McNemar p≈0.002），自由语音误唤醒不变。
// 这里量：抬高天花板之后，新 DTW 管线还能不能继续吃满。

use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig};

fn build_spotter_with_paths(dir: &Path, paths: i32) -> Option<KeywordSpotter> {
    let pick = |cands: &[&str]| -> Option<PathBuf> {
        cands.iter().map(|n| dir.join(n)).find(|p| p.is_file())
    };
    let mut config = KeywordSpotterConfig::default();
    config.model_config.transducer.encoder = Some(
        pick(&["encoder-epoch-12-avg-2-chunk-16-left-64.onnx", "encoder.onnx"])?
            .display()
            .to_string(),
    );
    config.model_config.transducer.decoder = Some(
        pick(&["decoder-epoch-12-avg-2-chunk-16-left-64.onnx", "decoder.onnx"])?
            .display()
            .to_string(),
    );
    config.model_config.transducer.joiner = Some(
        pick(&["joiner-epoch-12-avg-2-chunk-16-left-64.onnx", "joiner.onnx"])?
            .display()
            .to_string(),
    );
    config.model_config.tokens = Some(pick(&["tokens.txt"])?.display().to_string());
    config.model_config.num_threads = 2;
    config.max_active_paths = paths;
    config.keywords_buf = Some("n ǐ h ǎo @占位\n".to_owned());
    KeywordSpotter::create(&config)
}

fn raw_kws_fires(spotter: &KeywordSpotter, syntax: &str, samples: &[f32]) -> bool {
    let stream = spotter.create_stream_with_keywords(syntax);
    let mut fired = false;
    for chunk in samples.chunks(KWS_FRAME_SAMPLES) {
        stream.accept_waveform(16_000, chunk);
        while spotter.is_ready(&stream) {
            spotter.decode(&stream);
            if spotter.get_result(&stream).is_some_and(|r| !r.keyword.is_empty()) {
                fired = true;
            }
        }
    }
    fired
}

#[test]
#[ignore = "手动实验：voiceMatch 级联 + beam=8"]
fn voice_match_cascade_beam8() {
    let env = TestEnv::prepare().expect("test env");
    let groups = load_groups().expect("corpus");
    let free_paths = load_free_speech();
    let conditions = conditions();
    let ex = Extractor::new();
    let cohort_raw = cohort_audio();

    let spotter8 =
        build_spotter_with_paths(env.kws_model_dir(), 8).expect("spotter paths=8");

    let best = Cfg {
        name: "新特征",
        adaptive: true,
        crop: true,
        crop_ms: 1600,
        resample: None,
        feat: Feat::Mfcc13,
        cmvn: true,
        band: None,
        multi_template: false,
    };

    // 三条线：KWS单级(beam8) / beam8+原DTW / beam8+新DTW
    let mut kws8 = [0u32; 6];
    let mut old_lane = [0u32; 6];
    let mut new_lane = [0u32; 6];

    for group in &groups {
        let enroll = group.by_role(Role::Enroll);
        let audio: Vec<Vec<f32>> = enroll
            .iter()
            .take(3)
            .filter_map(|u| read_wav_16k(&u.path).ok())
            .collect();
        if audio.len() < 3 {
            continue;
        }
        let Ok(engine) = env.new_spotter(&group.phrase) else { continue };
        let syntax = engine.keyword_syntax().to_owned();
        drop(engine);

        let old_seqs: Vec<_> = audio.iter().map(|a| sequence(&ex, &BASE, a)).collect();
        let Some(old_tmpl) = Template::build(&BASE, old_seqs.clone()) else { continue };
        let old_cal = calibrate(Policy::Production, &BASE, &old_tmpl, &old_seqs, &[], 0.0);

        let new_seqs: Vec<_> = audio.iter().map(|a| sequence(&ex, &best, a)).collect();
        let cohort: Vec<_> = cohort_raw.iter().map(|a| sequence(&ex, &best, a)).collect();
        let Some(new_tmpl) = Template::build(&best, new_seqs.clone()) else { continue };
        let new_cal = calibrate(
            Policy::CohortQuantile,
            &best,
            &new_tmpl,
            &new_seqs,
            &cohort,
            1.00,
        );

        let mut eval = |samples: &[f32], base: usize| {
            let fired = raw_kws_fires(&spotter8, &syntax, samples);
            kws8[base + 1] += 1;
            kws8[base] += fired as u32;
            old_lane[base + 1] += 1;
            old_lane[base] +=
                (fired && old_cal.accept(old_tmpl.score(&BASE, &sequence(&ex, &BASE, samples))))
                    as u32;
            new_lane[base + 1] += 1;
            new_lane[base] +=
                (fired && new_cal.accept(new_tmpl.score(&best, &sequence(&ex, &best, samples))))
                    as u32;
        };

        for (cond_index, cond) in conditions.iter().enumerate() {
            let mut rng = Rng::new(0xB8 ^ ((cond_index as u64) << 8));
            for role in [Role::Positive, Role::Confusable] {
                let base = if role == Role::Positive { 0 } else { 2 };
                for utt in group.by_role(role) {
                    let Ok(raw) = read_wav_16k(&utt.path) else { continue };
                    let Ok(a) = apply(&raw, cond, &mut rng) else { continue };
                    eval(&a, base);
                }
            }
            let mut rng = Rng::new(0xB8F2 ^ ((cond_index as u64) << 8));
            for path in &free_paths {
                let Ok(raw) = read_wav_16k(path) else { continue };
                let Ok(a) = apply(&raw, cond, &mut rng) else { continue };
                eval(&a, 4);
            }
        }
    }

    println!("\n========== beam=8 下的级联 ==========");
    println!("{:<26}{:>10}{:>12}{:>14}", "方案", "识别率", "对立词误触", "自由语音误触");
    for (name, t) in [
        ("KWS单级(beam=8)天花板", kws8),
        ("beam=8 + 原DTW", old_lane),
        ("beam=8 + 新DTW", new_lane),
    ] {
        println!(
            "{:<26}{:>9.1}%{:>11.1}%{:>13.1}%",
            name,
            pct(t[0], t[1]),
            pct(t[2], t[3]),
            pct(t[4], t[5])
        );
    }
    println!("\n样本量：正 {} / 对立 {} / 自由 {}", kws8[1], kws8[3], kws8[5]);
}

/// 第六轮：专攻近音对立词——裁剪窗宽 + 时频分辨率
fn configs_crop() -> Vec<Cfg> {
    let best = Cfg {
        name: "无裁剪基准",
        adaptive: true,
        crop: false,
        crop_ms: 1600,
        resample: None,
        feat: Feat::Mfcc13,
        cmvn: true,
        band: None,
        multi_template: false,
    };
    let mut v = vec![BASE, best];
    for ms in [800usize, 1000, 1200, 1400, 1600, 2000, 2400] {
        let name: &'static str = Box::leak(format!("裁剪{ms}ms").into_boxed_str());
        v.push(Cfg { name, crop: true, crop_ms: ms, ..best });
    }
    // 裁剪 + 多模板 / 带约束，看能否在对立词上再进一步
    v.push(Cfg { name: "裁剪1200+多模板", crop: true, crop_ms: 1200, multi_template: true, ..best });
    v.push(Cfg { name: "裁剪1200+带0.15", crop: true, crop_ms: 1200, band: Some(0.15), ..best });
    v.push(Cfg { name: "裁剪1200+mel24", crop: true, crop_ms: 1200, feat: Feat::Mel24, ..best });
    v.push(Cfg { name: "裁剪1200+12维", crop: true, crop_ms: 1200, feat: Feat::LogBand12, ..best });
    v
}

#[test]
#[ignore = "手动实验：裁剪窗宽扫描（专攻对立词）"]
fn voice_match_crop_sweep() {
    let cfgs = configs_crop();
    let out = run_experiment(&cfgs);
    report(&cfgs, &out);
}

/// 第七轮：定稿配置候选（裁剪 1000ms 下比特征类型）
fn configs_final() -> Vec<Cfg> {
    let base = Cfg {
        name: "",
        adaptive: true,
        crop: true,
        crop_ms: 1000,
        resample: None,
        feat: Feat::LogBand12,
        cmvn: true,
        band: None,
        multi_template: false,
    };
    vec![
        BASE,
        Cfg { name: "1000+12维", ..base },
        Cfg { name: "1000+12维+多模板", multi_template: true, ..base },
        Cfg { name: "1000+mel24", feat: Feat::Mel24, ..base },
        Cfg { name: "1000+MFCC13", feat: Feat::Mfcc13, ..base },
        Cfg { name: "1000+MFCC13+多模板", feat: Feat::Mfcc13, multi_template: true, ..base },
        Cfg { name: "1100+12维", crop_ms: 1100, ..base },
        Cfg { name: "900+12维", crop_ms: 900, ..base },
    ]
}

#[test]
#[ignore = "手动实验：定稿配置候选"]
fn voice_match_final_candidates() {
    let cfgs = configs_final();
    let out = run_experiment(&cfgs);
    report(&cfgs, &out);
}

/// 定稿配置：自适应门限 + CMVN + 裁剪1000ms + 去定长重采样 + 保留12维
const FINAL: Cfg = Cfg {
    name: "定稿",
    adaptive: true,
    crop: true,
    crop_ms: 1000,
    resample: None,
    feat: Feat::LogBand12,
    cmvn: true,
    band: None,
    multi_template: false,
};

#[test]
#[ignore = "手动实验：voiceMatch 专属 beam 扫描（spotter 按模式独立配置）"]
fn voice_match_final_cascade() {
    let env = TestEnv::prepare().expect("test env");
    let groups = load_groups().expect("corpus");
    let free_paths = load_free_speech();
    let conditions = conditions();
    let ex = Extractor::new();
    let cohort_raw = cohort_audio();

    // spotter 是每个 profile 单独建的（lib.rs:218 拿得到 profile.mode），
    // 所以 max_active_paths 可以是 voiceMatch 专属参数，不影响其余三种模式。
    const BEAMS: [i32; 4] = [4, 8, 16, 32];
    let spotters: Vec<_> = BEAMS
        .iter()
        .map(|p| build_spotter_with_paths(env.kws_model_dir(), *p).expect("spotter"))
        .collect();

    // 每个 beam 三条线：KWS单级 / 原DTW / 新DTW
    let mut t = vec![[0u32; 6]; BEAMS.len() * 3];

    for group in &groups {
        let enroll = group.by_role(Role::Enroll);
        let audio: Vec<Vec<f32>> = enroll
            .iter()
            .take(3)
            .filter_map(|u| read_wav_16k(&u.path).ok())
            .collect();
        if audio.len() < 3 {
            continue;
        }
        let Ok(engine) = env.new_spotter(&group.phrase) else { continue };
        let syntax = engine.keyword_syntax().to_owned();
        drop(engine);

        let old_seqs: Vec<_> = audio.iter().map(|a| sequence(&ex, &BASE, a)).collect();
        let Some(old_tmpl) = Template::build(&BASE, old_seqs.clone()) else { continue };
        let old_cal = calibrate(Policy::Production, &BASE, &old_tmpl, &old_seqs, &[], 0.0);

        let new_seqs: Vec<_> = audio.iter().map(|a| sequence(&ex, &FINAL, a)).collect();
        let cohort: Vec<_> = cohort_raw.iter().map(|a| sequence(&ex, &FINAL, a)).collect();
        let Some(new_tmpl) = Template::build(&FINAL, new_seqs.clone()) else { continue };
        let new_cal = calibrate(
            Policy::CohortQuantile,
            &FINAL,
            &new_tmpl,
            &new_seqs,
            &cohort,
            1.00,
        );

        let mut eval = |samples: &[f32], base: usize| {
            // DTW 与 beam 无关，每条样本只算一次
            let old_pass = old_cal.accept(old_tmpl.score(&BASE, &sequence(&ex, &BASE, samples)));
            let new_pass = new_cal.accept(new_tmpl.score(&FINAL, &sequence(&ex, &FINAL, samples)));
            for (bi, spotter) in spotters.iter().enumerate() {
                let fired = raw_kws_fires(spotter, &syntax, samples);
                let lane = bi * 3;
                for (k, ok) in [
                    (0, fired),
                    (1, fired && old_pass),
                    (2, fired && new_pass),
                ] {
                    t[lane + k][base + 1] += 1;
                    t[lane + k][base] += ok as u32;
                }
            }
        };

        for (cond_index, cond) in conditions.iter().enumerate() {
            let mut rng = Rng::new(0xF1A1 ^ ((cond_index as u64) << 8));
            for role in [Role::Positive, Role::Confusable] {
                let base = if role == Role::Positive { 0 } else { 2 };
                for utt in group.by_role(role) {
                    let Ok(raw) = read_wav_16k(&utt.path) else { continue };
                    let Ok(a) = apply(&raw, cond, &mut rng) else { continue };
                    eval(&a, base);
                }
            }
            let mut rng = Rng::new(0xF1A1F2 ^ ((cond_index as u64) << 8));
            for path in &free_paths {
                let Ok(raw) = read_wav_16k(path) else { continue };
                let Ok(a) = apply(&raw, cond, &mut rng) else { continue };
                eval(&a, 4);
            }
        }
    }

    println!("\n========== voiceMatch 专属 beam 扫描 ==========");
    println!(
        "{:<30}{:>16}{:>18}{:>14}{:>14}",
        "方案", "唤醒率", "误触率(合计)", "└近音词", "└日常说话"
    );
    for (bi, beam) in BEAMS.iter().enumerate() {
        for (k, tag) in ["KWS单级(天花板)", "+ 原DTW", "+ 新DTW"].iter().enumerate() {
            let r = t[bi * 3 + k];
            let fh = r[2] + r[4];
            let ft = r[3] + r[5];
            println!(
                "beam={:<3}{:<22}{:>8.1}% {:>4}/{:<4}{:>9.2}% {:>4}/{:<5}{:>6.1}% {:>3}/{:<4}{:>5.1}% {:>4}/{:<5}",
                beam,
                tag,
                pct(r[0], r[1]),
                r[0],
                r[1],
                pct(fh, ft),
                fh,
                ft,
                pct(r[2], r[3]),
                r[2],
                r[3],
                pct(r[4], r[5]),
                r[4],
                r[5],
            );
        }
        println!();
    }
    println!("误触率合计口径：近音对立词 + 日常说话，分母 {} 条。", t[0][3] + t[0][5]);
}

#[test]
#[ignore = "手动实验：voiceMatch beam 的常驻 CPU 代价"]
fn voice_match_beam_cost() {
    use std::time::Instant;
    let env = TestEnv::prepare().expect("test env");
    let dir = env.kws_model_dir();
    let syntax = "x iǎo d í x iǎo d í @唤醒\n";
    let frame = vec![0.0_f32; 1_600];
    let frames = 1_200usize; // 120 秒静音，模拟常驻监听
    let seconds = frames as f64 / 10.0;

    println!("\n=== 常驻监听 CPU（{seconds:.0} 秒静音，num_threads=1）===");
    println!("{:<10}{:>12}{:>12}{:>18}", "beam", "墙钟(s)", "RTF", "单核占用");
    let mut base = 0.0_f64;
    for (i, paths) in [4_i32, 8, 16, 32].iter().enumerate() {
        let spotter = build_spotter_with_paths(dir, *paths).expect("spotter");
        // 预热
        let warm = spotter.create_stream_with_keywords(syntax);
        for _ in 0..30 {
            warm.accept_waveform(16_000, &frame);
            while spotter.is_ready(&warm) {
                spotter.decode(&warm);
            }
        }
        drop(warm);

        let stream = spotter.create_stream_with_keywords(syntax);
        let t0 = Instant::now();
        for _ in 0..frames {
            stream.accept_waveform(16_000, &frame);
            while spotter.is_ready(&stream) {
                spotter.decode(&stream);
            }
        }
        let elapsed = t0.elapsed().as_secs_f64();
        let rtf = elapsed / seconds;
        if i == 0 {
            base = rtf;
        }
        println!(
            "{:<10}{:>12.3}{:>12.4}{:>13.2}%  ({:+.0}% vs beam=4)",
            paths,
            elapsed,
            rtf,
            rtf * 100.0,
            (rtf / base - 1.0) * 100.0
        );
    }
    println!("\nRTF 0.05 = 处理 1 秒音频花 50 ms，即常驻占用约 5% 的一个核。");
}

/// 生产路径的负样本队列：直接用 classifier.ms-snsd-babble 资产（与实现方案一致）。
fn cohort_audio_production() -> Vec<Vec<f32>> {
    let path = assets_dir()
        .join(".e2e-home")
        .join("assets")
        .join("classifier")
        .join("ms-snsd-babble")
        .join("babble-16k.wav");
    let Ok(samples) = read_wav_16k(&path) else {
        return Vec::new();
    };
    let window = 16_000 * 5;
    let mut out = Vec::new();
    let mut start = 0;
    while start + window <= samples.len() {
        out.push(samples[start..start + window].to_vec());
        start += window;
    }
    out
}

#[test]
#[ignore = "手动实验：核对生产资产做队列是否与实验一致"]
fn voice_match_production_cohort() {
    let groups = load_groups().expect("corpus");
    let free_paths = load_free_speech();
    let conditions = conditions();
    let ex = Extractor::new();

    let exp_cohort = cohort_audio();
    let prod_cohort = cohort_audio_production();
    println!(
        "实验队列 {} 段 / 生产队列（classifier babble）{} 段",
        exp_cohort.len(),
        prod_cohort.len()
    );
    assert!(!prod_cohort.is_empty(), "找不到 classifier babble 资产");

    // 两种队列各校准一次，比对阈值与最终指标（DTW 单级，不过 KWS）
    let mut tally = vec![[0u32; 6]; 2];
    let mut th_stats = vec![Vec::new(); 2];

    for group in &groups {
        let enroll = group.by_role(Role::Enroll);
        let audio: Vec<Vec<f32>> = enroll
            .iter()
            .take(3)
            .filter_map(|u| read_wav_16k(&u.path).ok())
            .collect();
        if audio.len() < 3 {
            continue;
        }
        let seqs: Vec<_> = audio.iter().map(|a| sequence(&ex, &FINAL, a)).collect();
        let Some(tmpl) = Template::build(&FINAL, seqs.clone()) else { continue };

        let mut cals = Vec::new();
        for (i, raw) in [&exp_cohort, &prod_cohort].iter().enumerate() {
            let cohort: Vec<_> = raw.iter().map(|a| sequence(&ex, &FINAL, a)).collect();
            let cal = calibrate(Policy::CohortQuantile, &FINAL, &tmpl, &seqs, &cohort, 1.00);
            th_stats[i].push(cal.threshold);
            cals.push(cal);
        }

        for (cond_index, cond) in conditions.iter().enumerate() {
            let mut rng = Rng::new(0xC0DE ^ ((cond_index as u64) << 8));
            for role in [Role::Positive, Role::Confusable] {
                let base = if role == Role::Positive { 0 } else { 2 };
                for utt in group.by_role(role) {
                    let Ok(raw) = read_wav_16k(&utt.path) else { continue };
                    let Ok(a) = apply(&raw, cond, &mut rng) else { continue };
                    let score = tmpl.score(&FINAL, &sequence(&ex, &FINAL, &a));
                    for (i, cal) in cals.iter().enumerate() {
                        tally[i][base + 1] += 1;
                        tally[i][base] += cal.accept(score) as u32;
                    }
                }
            }
            let mut rng = Rng::new(0xC0DEF2 ^ ((cond_index as u64) << 8));
            for path in &free_paths {
                let Ok(raw) = read_wav_16k(path) else { continue };
                let Ok(a) = apply(&raw, cond, &mut rng) else { continue };
                let score = tmpl.score(&FINAL, &sequence(&ex, &FINAL, &a));
                for (i, cal) in cals.iter().enumerate() {
                    tally[i][5] += 1;
                    tally[i][4] += cal.accept(score) as u32;
                }
            }
        }
    }

    println!("\n========== 队列来源对比（DTW 单级）==========");
    println!("{:<26}{:>12}{:>12}{:>12}{:>20}", "队列", "通过率(正)", "对立词", "自由语音", "阈值 min~max");
    for (i, name) in ["实验用(noise/crowd-babble)", "生产用(classifier babble)"].iter().enumerate() {
        let t = tally[i];
        let lo = th_stats[i].iter().cloned().fold(f32::INFINITY, f32::min);
        let hi = th_stats[i].iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        println!(
            "{:<26}{:>11.1}%{:>11.1}%{:>11.1}%{:>13.3}~{:.3}",
            name,
            pct(t[0], t[1]),
            pct(t[2], t[3]),
            pct(t[4], t[5]),
            lo,
            hi
        );
    }
    let diff: f32 = th_stats[0]
        .iter()
        .zip(&th_stats[1])
        .map(|(a, b)| (a - b).abs())
        .sum::<f32>()
        / th_stats[0].len().max(1) as f32;
    println!("\n每组阈值的平均绝对差：{diff:.4}");
}

// ============================ 合成负样本队列 ============================
//
// 目标：不依赖任何外部素材，只用用户自己的注册录音就能标定「不像唤醒词」的边界。
// 合成负样本还有个额外好处——它与注册录音同源（同一麦克风、同一房间底噪），
// 比通用 babble 更贴近该用户的实际声学条件。
//
// 四种造法，语义各不相同：
//   Shuffle   把注册音频按 120 ms 切块打乱重排 —— 保留音色与底噪，破坏时序。
//             这正是 DTW 应该拒绝的东西：听着像你，但词不对。
//   Reverse   整段倒放 —— 同样保留音色，时序完全反过来。
//   Silence   取注册音频里能量最低的片段拼接 —— 纯环境底噪。
//   Mixed     以上三者混合。

/// 把音频按块打乱。块长取 120 ms：短于一个音节，足以打散词的结构，
/// 又不至于碎到只剩噪声。
fn shuffle_blocks(samples: &[f32], seed: u64) -> Vec<f32> {
    const BLOCK: usize = 16_000 * 120 / 1000;
    if samples.len() < BLOCK * 4 {
        return samples.to_vec();
    }
    let mut blocks: Vec<&[f32]> = samples.chunks(BLOCK).filter(|c| c.len() == BLOCK).collect();
    let mut rng = Rng::new(seed);
    // Fisher-Yates
    for i in (1..blocks.len()).rev() {
        let j = (rng.next_u64() % (i as u64 + 1)) as usize;
        blocks.swap(i, j);
    }
    blocks.concat()
}

fn reverse_audio(samples: &[f32]) -> Vec<f32> {
    let mut out = samples.to_vec();
    out.reverse();
    out
}

/// 取能量最低的若干块拼接：近似「这个环境里没人说话时的样子」。
fn quiet_blocks(samples: &[f32], target_len: usize) -> Vec<f32> {
    const BLOCK: usize = 16_000 * 120 / 1000;
    let mut blocks: Vec<(f32, &[f32])> = samples
        .chunks(BLOCK)
        .filter(|c| c.len() == BLOCK)
        .map(|c| (c.iter().map(|s| s * s).sum::<f32>(), c))
        .collect();
    if blocks.is_empty() {
        return samples.to_vec();
    }
    blocks.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = Vec::with_capacity(target_len);
    let mut index = 0;
    while out.len() < target_len && !blocks.is_empty() {
        out.extend_from_slice(blocks[index % blocks.len()].1);
        index += 1;
        if index > blocks.len() * 4 {
            break;
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cohort {
    /// 现状：外部 babble 素材
    Babble,
    Shuffle,
    Reverse,
    Silence,
    Mixed,
}

/// 从注册录音合成负样本。`enrolled_audio` 是原始波形，不是特征。
fn synth_cohort(kind: Cohort, enrolled_audio: &[Vec<f32>], babble: &[Vec<f32>]) -> Vec<Vec<f32>> {
    match kind {
        Cohort::Babble => babble.to_vec(),
        Cohort::Shuffle => enrolled_audio
            .iter()
            .enumerate()
            .flat_map(|(i, a)| {
                (0..4).map(move |k| shuffle_blocks(a, 0x5EED ^ ((i as u64) << 8) ^ k))
            })
            .collect(),
        Cohort::Reverse => enrolled_audio.iter().map(|a| reverse_audio(a)).collect(),
        Cohort::Silence => enrolled_audio
            .iter()
            .map(|a| quiet_blocks(a, a.len()))
            .collect(),
        Cohort::Mixed => {
            let mut out = Vec::new();
            for (i, a) in enrolled_audio.iter().enumerate() {
                for k in 0..2 {
                    out.push(shuffle_blocks(a, 0x5EED ^ ((i as u64) << 8) ^ k));
                }
                out.push(reverse_audio(a));
                out.push(quiet_blocks(a, a.len()));
            }
            out
        }
    }
}

#[test]
#[ignore = "手动实验：合成负样本队列 vs babble"]
fn voice_match_synthetic_cohort() {
    let groups = load_groups().expect("corpus");
    let free_paths = load_free_speech();
    let conditions = conditions();
    let ex = Extractor::new();
    let babble = cohort_audio();

    const KINDS: [(Cohort, &str); 5] = [
        (Cohort::Babble, "babble(现状,需下载)"),
        (Cohort::Shuffle, "打乱重排"),
        (Cohort::Reverse, "倒放"),
        (Cohort::Silence, "静音段"),
        (Cohort::Mixed, "混合"),
    ];

    // [kind] -> [pos_pass,pos_total, cfz_pass,cfz_total, free_pass,free_total]
    let mut tally = vec![[0u32; 6]; KINDS.len()];
    let mut thresholds = vec![Vec::new(); KINDS.len()];

    for group in &groups {
        let enroll = group.by_role(Role::Enroll);
        let audio: Vec<Vec<f32>> = enroll
            .iter()
            .take(3)
            .filter_map(|u| read_wav_16k(&u.path).ok())
            .collect();
        if audio.len() < 3 {
            continue;
        }
        let seqs: Vec<_> = audio.iter().map(|a| sequence(&ex, &FINAL, a)).collect();
        let Some(tmpl) = Template::build(&FINAL, seqs.clone()) else { continue };

        let mut cals = Vec::new();
        for (kind, _) in KINDS {
            let raw = synth_cohort(kind, &audio, &babble);
            let cohort: Vec<_> = raw.iter().map(|a| sequence(&ex, &FINAL, a)).collect();
            let cal = calibrate(Policy::CohortQuantile, &FINAL, &tmpl, &seqs, &cohort, 1.00);
            thresholds[cals.len()].push(cal.threshold);
            cals.push(cal);
        }

        for (cond_index, cond) in conditions.iter().enumerate() {
            let mut rng = Rng::new(0x5417 ^ ((cond_index as u64) << 8));
            for role in [Role::Positive, Role::Confusable] {
                let base = if role == Role::Positive { 0 } else { 2 };
                for utt in group.by_role(role) {
                    let Ok(raw) = read_wav_16k(&utt.path) else { continue };
                    let Ok(a) = apply(&raw, cond, &mut rng) else { continue };
                    let score = tmpl.score(&FINAL, &sequence(&ex, &FINAL, &a));
                    for (i, cal) in cals.iter().enumerate() {
                        tally[i][base + 1] += 1;
                        tally[i][base] += cal.accept(score) as u32;
                    }
                }
            }
            let mut rng = Rng::new(0x5417F2 ^ ((cond_index as u64) << 8));
            for path in &free_paths {
                let Ok(raw) = read_wav_16k(path) else { continue };
                let Ok(a) = apply(&raw, cond, &mut rng) else { continue };
                let score = tmpl.score(&FINAL, &sequence(&ex, &FINAL, &a));
                for (i, cal) in cals.iter().enumerate() {
                    tally[i][5] += 1;
                    tally[i][4] += cal.accept(score) as u32;
                }
            }
        }
    }

    println!("\n========== 负样本队列来源对比（DTW 单级，未过 KWS）==========");
    println!(
        "{:<22}{:>12}{:>12}{:>12}{:>20}",
        "队列来源", "通过率(正)", "对立词", "自由语音", "阈值 min~max"
    );
    for (i, (_, name)) in KINDS.iter().enumerate() {
        let t = tally[i];
        let lo = thresholds[i].iter().cloned().fold(f32::INFINITY, f32::min);
        let hi = thresholds[i].iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        println!(
            "{:<22}{:>11.1}%{:>11.1}%{:>11.1}%{:>13.3}~{:.3}",
            name,
            pct(t[0], t[1]),
            pct(t[2], t[3]),
            pct(t[4], t[5]),
            lo,
            hi
        );
    }
    println!("\n目标：合成队列的三列数字都接近 babble 那一行，就能去掉外部依赖。");
}

#[test]
#[ignore = "手动实验：合成队列的分位数扫描"]
fn voice_match_synthetic_quantile() {
    let groups = load_groups().expect("corpus");
    let free_paths = load_free_speech();
    let conditions = conditions();
    let ex = Extractor::new();
    let babble = cohort_audio();

    // (队列种类, 分位数, 标签)
    let lanes: Vec<(Cohort, f32, String)> = {
        let mut v = vec![(Cohort::Babble, 1.00, "babble 最大值(现状)".to_owned())];
        for q in [1.00_f32, 0.90, 0.75, 0.60, 0.50] {
            v.push((Cohort::Shuffle, q, format!("打乱 {:.0}分位", q * 100.0)));
        }
        for q in [0.90_f32, 0.75, 0.60] {
            v.push((Cohort::Mixed, q, format!("混合 {:.0}分位", q * 100.0)));
        }
        v
    };

    let mut tally = vec![[0u32; 6]; lanes.len()];
    let mut thresholds = vec![Vec::new(); lanes.len()];

    for group in &groups {
        let enroll = group.by_role(Role::Enroll);
        let audio: Vec<Vec<f32>> = enroll
            .iter()
            .take(3)
            .filter_map(|u| read_wav_16k(&u.path).ok())
            .collect();
        if audio.len() < 3 {
            continue;
        }
        let seqs: Vec<_> = audio.iter().map(|a| sequence(&ex, &FINAL, a)).collect();
        let Some(tmpl) = Template::build(&FINAL, seqs.clone()) else { continue };

        let mut cals = Vec::new();
        for (li, (kind, q, _)) in lanes.iter().enumerate() {
            let raw = synth_cohort(*kind, &audio, &babble);
            let cohort: Vec<_> = raw.iter().map(|a| sequence(&ex, &FINAL, a)).collect();
            let cal = calibrate(Policy::CohortQuantile, &FINAL, &tmpl, &seqs, &cohort, *q);
            thresholds[li].push(cal.threshold);
            cals.push(cal);
        }

        for (cond_index, cond) in conditions.iter().enumerate() {
            let mut rng = Rng::new(0x9A11 ^ ((cond_index as u64) << 8));
            for role in [Role::Positive, Role::Confusable] {
                let base = if role == Role::Positive { 0 } else { 2 };
                for utt in group.by_role(role) {
                    let Ok(raw) = read_wav_16k(&utt.path) else { continue };
                    let Ok(a) = apply(&raw, cond, &mut rng) else { continue };
                    let score = tmpl.score(&FINAL, &sequence(&ex, &FINAL, &a));
                    for (i, cal) in cals.iter().enumerate() {
                        tally[i][base + 1] += 1;
                        tally[i][base] += cal.accept(score) as u32;
                    }
                }
            }
            let mut rng = Rng::new(0x9A11F2 ^ ((cond_index as u64) << 8));
            for path in &free_paths {
                let Ok(raw) = read_wav_16k(path) else { continue };
                let Ok(a) = apply(&raw, cond, &mut rng) else { continue };
                let score = tmpl.score(&FINAL, &sequence(&ex, &FINAL, &a));
                for (i, cal) in cals.iter().enumerate() {
                    tally[i][5] += 1;
                    tally[i][4] += cal.accept(score) as u32;
                }
            }
        }
    }

    println!("\n========== 合成队列分位数扫描（DTW 单级）==========");
    println!(
        "{:<22}{:>12}{:>12}{:>12}{:>20}",
        "配置", "通过率(正)", "对立词", "自由语音", "阈值 min~max"
    );
    for (i, (_, _, name)) in lanes.iter().enumerate() {
        let t = tally[i];
        let lo = thresholds[i].iter().cloned().fold(f32::INFINITY, f32::min);
        let hi = thresholds[i].iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        println!(
            "{:<22}{:>11.1}%{:>11.1}%{:>11.1}%{:>13.3}~{:.3}",
            name,
            pct(t[0], t[1]),
            pct(t[2], t[3]),
            pct(t[4], t[5]),
            lo,
            hi
        );
    }
}

#[test]
#[ignore = "手动实验：合成队列的端到端级联"]
fn voice_match_synthetic_cascade() {
    let env = TestEnv::prepare().expect("test env");
    let groups = load_groups().expect("corpus");
    let free_paths = load_free_speech();
    let conditions = conditions();
    let ex = Extractor::new();
    let babble = cohort_audio();

    let spotter = build_spotter_with_paths(env.kws_model_dir(), 16).expect("spotter16");

    // 旧管线 + 旧阈值 / 新管线 + babble / 新管线 + 合成队列
    let mut t = vec![[0u32; 6]; 3];

    for group in &groups {
        let enroll = group.by_role(Role::Enroll);
        let audio: Vec<Vec<f32>> = enroll
            .iter()
            .take(3)
            .filter_map(|u| read_wav_16k(&u.path).ok())
            .collect();
        if audio.len() < 3 {
            continue;
        }
        let Ok(engine) = env.new_spotter(&group.phrase) else { continue };
        let syntax = engine.keyword_syntax().to_owned();
        drop(engine);

        let old_seqs: Vec<_> = audio.iter().map(|a| sequence(&ex, &BASE, a)).collect();
        let Some(old_tmpl) = Template::build(&BASE, old_seqs.clone()) else { continue };
        let old_cal = calibrate(Policy::Production, &BASE, &old_tmpl, &old_seqs, &[], 0.0);

        let seqs: Vec<_> = audio.iter().map(|a| sequence(&ex, &FINAL, a)).collect();
        let Some(tmpl) = Template::build(&FINAL, seqs.clone()) else { continue };

        let bab: Vec<_> = babble.iter().map(|a| sequence(&ex, &FINAL, a)).collect();
        let cal_bab = calibrate(Policy::CohortQuantile, &FINAL, &tmpl, &seqs, &bab, 1.00);

        let syn_raw = synth_cohort(Cohort::Shuffle, &audio, &babble);
        let syn: Vec<_> = syn_raw.iter().map(|a| sequence(&ex, &FINAL, a)).collect();
        let cal_syn = calibrate(Policy::CohortQuantile, &FINAL, &tmpl, &seqs, &syn, 0.60);

        let mut eval = |samples: &[f32], base: usize| {
            let fired = raw_kws_fires(&spotter, &syntax, samples);
            let old_ok = fired
                && old_cal.accept(old_tmpl.score(&BASE, &sequence(&ex, &BASE, samples)));
            let score = tmpl.score(&FINAL, &sequence(&ex, &FINAL, samples));
            for (k, ok) in [
                (0, old_ok),
                (1, fired && cal_bab.accept(score)),
                (2, fired && cal_syn.accept(score)),
            ] {
                t[k][base + 1] += 1;
                t[k][base] += ok as u32;
            }
        };

        for (cond_index, cond) in conditions.iter().enumerate() {
            let mut rng = Rng::new(0x5C1 ^ ((cond_index as u64) << 8));
            for role in [Role::Positive, Role::Confusable] {
                let base = if role == Role::Positive { 0 } else { 2 };
                for utt in group.by_role(role) {
                    let Ok(raw) = read_wav_16k(&utt.path) else { continue };
                    let Ok(a) = apply(&raw, cond, &mut rng) else { continue };
                    eval(&a, base);
                }
            }
            let mut rng = Rng::new(0x5C1F2 ^ ((cond_index as u64) << 8));
            for path in &free_paths {
                let Ok(raw) = read_wav_16k(path) else { continue };
                let Ok(a) = apply(&raw, cond, &mut rng) else { continue };
                eval(&a, 4);
            }
        }
    }

    println!("\n========== 端到端（KWS beam=16 + DTW）==========");
    println!("{:<30}{:>14}{:>16}{:>14}{:>14}", "方案", "唤醒率", "误触率(合计)", "└近音词", "└日常说话");
    for (i, name) in ["现状（旧管线+旧阈值）", "新管线 + babble队列", "新管线 + 合成队列(零依赖)"].iter().enumerate() {
        let r = t[i];
        let fh = r[2] + r[4];
        let ft = r[3] + r[5];
        println!(
            "{:<30}{:>8.1}% {:>4}/{:<4}{:>9.2}% {:>4}/{:<5}{:>6.1}%{:>13.1}%",
            name,
            pct(r[0], r[1]), r[0], r[1],
            pct(fh, ft), fh, ft,
            pct(r[2], r[3]),
            pct(r[4], r[5]),
        );
    }
}
