//! 测试期音频增广：变速、混噪、增益。全部在内存里完成，不落盘。

use dictatingme_runtime::evoke_setup::features::read_wav_16k;
use std::path::{Path, PathBuf};

/// 一个测试条件 = 语速 × 噪声类别 × 信噪比 × 增益。
#[derive(Debug, Clone)]
pub struct Condition {
    pub id: String,
    pub speed: f32,
    pub noise: Option<PathBuf>,
    pub noise_category: String,
    pub snr_db: f32,
    pub gain: f32,
}

impl Condition {
    pub fn clean() -> Self {
        Self {
            id: "clean".to_owned(),
            speed: 1.0,
            noise: None,
            noise_category: "none".to_owned(),
            snr_db: f32::INFINITY,
            gain: 1.0,
        }
    }
}

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

pub fn apply(samples: &[f32], condition: &Condition, rng: &mut Rng) -> Result<Vec<f32>, String> {
    let mut output = if (condition.speed - 1.0).abs() > 1e-3 {
        time_stretch(samples, condition.speed)
    } else {
        samples.to_vec()
    };
    if let Some(noise_path) = &condition.noise {
        let noise = load_noise(noise_path)?;
        mix_noise(&mut output, &noise, condition.snr_db, rng);
    }
    if (condition.gain - 1.0).abs() > 1e-3 {
        for sample in output.iter_mut() {
            *sample = (*sample * condition.gain).clamp(-1.0, 1.0);
        }
    }
    Ok(output)
}

/// 重采样式变速：语速与音高一起变，对应「说得快 / 说得慢」这种真实变化里
/// 最保守的一种近似——它对模板匹配是偏难的条件，不会把结果测得偏乐观。
fn time_stretch(samples: &[f32], speed: f32) -> Vec<f32> {
    if samples.is_empty() || speed <= 0.0 {
        return samples.to_vec();
    }
    let output_len = ((samples.len() as f32) / speed) as usize;
    (0..output_len)
        .map(|index| {
            let position = index as f32 * speed;
            let left = position.floor() as usize;
            let right = (left + 1).min(samples.len() - 1);
            let fraction = position - left as f32;
            if left >= samples.len() {
                0.0
            } else {
                samples[left] + (samples[right] - samples[left]) * fraction
            }
        })
        .collect()
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

/// 按目标信噪比把噪声混进语音。信噪比按「语音的有声段」计算，
/// 避免 5 秒窗口里大段静音把 SNR 算低。
fn mix_noise(speech: &mut [f32], noise: &[f32], snr_db: f32, rng: &mut Rng) {
    if noise.is_empty() || speech.is_empty() || !snr_db.is_finite() {
        return;
    }
    let voiced = speech
        .iter()
        .copied()
        .filter(|sample| sample.abs() >= 0.02)
        .collect::<Vec<_>>();
    let speech_rms = if voiced.len() >= speech.len() / 20 {
        rms(&voiced)
    } else {
        rms(speech)
    };
    let noise_rms = rms(noise);
    if speech_rms <= 1e-6 || noise_rms <= 1e-6 {
        return;
    }
    let target_noise_rms = speech_rms / 10f32.powf(snr_db / 20.0);
    let scale = target_noise_rms / noise_rms;
    let offset = (rng.next_u64() as usize) % noise.len();
    for (index, sample) in speech.iter_mut().enumerate() {
        let noise_sample = noise[(offset + index) % noise.len()];
        *sample = (*sample + noise_sample * scale).clamp(-1.0, 1.0);
    }
}

fn load_noise(path: &Path) -> Result<Vec<f32>, String> {
    read_wav_16k(path)
}
