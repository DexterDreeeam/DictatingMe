/// 确定性伪随机数发生器（xorshift64*），保证同一个 seed 永远产出同一份语料。
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

    /// [0, 1) 之间的均匀分布。
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }
}

pub fn resample_linear(samples: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
    if samples.is_empty() || input_rate == 0 || output_rate == 0 || input_rate == output_rate {
        return samples.to_vec();
    }
    let output_len = ((samples.len() as u64 * u64::from(output_rate)) / u64::from(input_rate)) as usize;
    (0..output_len)
        .map(|index| {
            let position = index as f64 * f64::from(input_rate) / f64::from(output_rate);
            let left = position.floor() as usize;
            let right = (left + 1).min(samples.len() - 1);
            let fraction = (position - left as f64) as f32;
            samples[left] + (samples[right] - samples[left]) * fraction
        })
        .collect()
}

pub fn peak(samples: &[f32]) -> f32 {
    samples.iter().copied().map(f32::abs).fold(0.0, f32::max)
}

pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

/// 把语音段归一化到目标峰值，消除 TTS 不同说话人之间的音量差异，
/// 让后续的 gain（模拟大声/小声/远场）是唯一的响度变量。
pub fn normalize_peak(samples: &mut [f32], target: f32) {
    let current = peak(samples);
    if current > 1e-6 {
        let scale = target / current;
        for sample in samples.iter_mut() {
            *sample *= scale;
        }
    }
}

pub fn trim_silence(samples: &[f32], threshold: f32) -> &[f32] {
    let start = samples
        .iter()
        .position(|sample| sample.abs() >= threshold)
        .unwrap_or(0);
    let end = samples
        .iter()
        .rposition(|sample| sample.abs() >= threshold)
        .map(|index| index + 1)
        .unwrap_or(samples.len());
    &samples[start.min(end)..end]
}

/// 极简的早期反射混响，用来模拟「离麦克风更远」的录音。
pub fn far_field(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    let taps = [
        (0.011_f32, 0.34_f32),
        (0.019, 0.22),
        (0.031, 0.15),
        (0.047, 0.09),
    ];
    let mut output = samples.to_vec();
    for (delay_s, amount) in taps {
        let delay = (delay_s * sample_rate as f32) as usize;
        if delay == 0 || delay >= samples.len() {
            continue;
        }
        for index in delay..samples.len() {
            output[index] += samples[index - delay] * amount;
        }
    }
    // 远场同时意味着高频衰减，用一阶低通近似。
    let mut previous = 0.0_f32;
    for sample in output.iter_mut() {
        previous = previous * 0.42 + *sample * 0.58;
        *sample = previous;
    }
    output
}

/// 非常轻微的室内本底噪声，避免出现「绝对静音」这种真实录音里不存在的片段。
pub fn room_tone(len: usize, level: f32, rng: &mut Rng) -> Vec<f32> {
    let mut previous = 0.0_f32;
    (0..len)
        .map(|_| {
            let white = rng.range(-1.0, 1.0);
            previous = previous * 0.86 + white * 0.14;
            previous * level
        })
        .collect()
}

/// 把一段语音放进固定时长的「录音」里：前后补本底噪声，位置随机但可复现。
pub fn place_in_window(
    speech: &[f32],
    total_len: usize,
    noise_level: f32,
    rng: &mut Rng,
) -> Vec<f32> {
    let mut track = room_tone(total_len, noise_level, rng);
    if speech.is_empty() {
        return track;
    }
    let speech = if speech.len() > total_len {
        &speech[..total_len]
    } else {
        speech
    };
    let headroom = total_len - speech.len();
    // 留出至少 0.15 s 的引导静音，其余位置随机。
    let lead_min = ((total_len as f32) * 0.03) as usize;
    let offset = if headroom > lead_min {
        lead_min + (rng.next_u64() as usize % (headroom - lead_min + 1))
    } else {
        headroom / 2
    };
    for (index, sample) in speech.iter().enumerate() {
        track[offset + index] += *sample;
    }
    for sample in track.iter_mut() {
        *sample = sample.clamp(-1.0, 1.0);
    }
    track
}

pub fn write_wav_16k(path: &std::path::Path, samples: &[f32]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create '{}': {error}", parent.display()))?;
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|error| format!("failed to create '{}': {error}", path.display()))?;
    for sample in samples {
        writer
            .write_sample((sample.clamp(-1.0, 1.0) * 32_767.0) as i16)
            .map_err(|error| format!("failed to write '{}': {error}", path.display()))?;
    }
    writer
        .finalize()
        .map_err(|error| format!("failed to finalize '{}': {error}", path.display()))
}
