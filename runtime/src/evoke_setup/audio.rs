//! 共用音频 IO 与录音质量检测。

use std::path::Path;

use crate::audio::AudioFrame;

pub fn frames_to_16k(frames: &[AudioFrame]) -> Vec<f32> {
    let Some(first) = frames.first() else {
        return Vec::new();
    };
    let samples = frames
        .iter()
        .flat_map(|frame| frame.samples.iter().copied())
        .map(|sample| f32::from(sample) / 32768.0)
        .collect::<Vec<_>>();
    resample_linear(&samples, first.sample_rate, 16_000)
}

pub fn read_wav_16k(path: &Path) -> Result<Vec<f32>, String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|error| format!("failed to open WAV '{}': {error}", path.display()))?;
    let spec = reader.spec();
    if spec.channels == 0 {
        return Err(format!("WAV '{}' has zero channels", path.display()));
    }
    let channels = usize::from(spec.channels);
    let interleaved = match spec.sample_format {
        hound::SampleFormat::Int if spec.bits_per_sample <= 16 => reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("failed to read WAV samples: {error}"))?
            .into_iter()
            .map(|sample| f32::from(sample) / 32768.0)
            .collect::<Vec<_>>(),
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("failed to read WAV samples: {error}"))?,
        _ => {
            return Err(format!(
                "unsupported WAV format in '{}': {:?}/{} bit",
                path.display(),
                spec.sample_format,
                spec.bits_per_sample
            ))
        }
    };
    let mono = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / frame.len().max(1) as f32)
        .collect::<Vec<_>>();
    Ok(resample_linear(&mono, spec.sample_rate, 16_000))
}

pub fn write_wav_16k(path: &Path, samples: &[f32]) -> Result<(), String> {
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
        .map_err(|error| format!("failed to create WAV '{}': {error}", path.display()))?;
    for sample in samples {
        writer
            .write_sample((sample.clamp(-1.0, 1.0) * 32767.0) as i16)
            .map_err(|error| format!("failed to write WAV '{}': {error}", path.display()))?;
    }
    writer
        .finalize()
        .map_err(|error| format!("failed to finalize WAV '{}': {error}", path.display()))
}

pub fn recording_quality(samples: &[f32]) -> crate::evoke_setup::RecordingQuality {
    if samples.is_empty() {
        return crate::evoke_setup::RecordingQuality {
            accepted: false,
            rms: 0.0,
            peak: 0.0,
            clipping_ratio: 0.0,
            voiced_ratio: 0.0,
            rejection: Some("没有采集到音频".to_owned()),
        };
    }
    let peak = samples
        .iter()
        .copied()
        .map(f32::abs)
        .fold(0.0_f32, f32::max);
    let rms =
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt();
    let clipping_ratio = samples
        .iter()
        .filter(|sample| sample.abs() >= 0.985)
        .count() as f32
        / samples.len() as f32;
    let frame = 320;
    let voiced = samples
        .chunks(frame)
        .filter(|chunk| {
            let frame_rms = (chunk.iter().map(|sample| sample * sample).sum::<f32>()
                / chunk.len() as f32)
                .sqrt();
            frame_rms >= 0.008
        })
        .count();
    let total_frames = samples.len().div_ceil(frame);
    let voiced_ratio = voiced as f32 / total_frames.max(1) as f32;
    let rejection = if rms < 0.004 {
        Some("音量过低，请靠近麦克风并重试".to_owned())
    } else if clipping_ratio > 0.08 {
        Some("声音削波过多，请降低音量并重试".to_owned())
    } else if voiced_ratio < 0.08 {
        Some("没有检测到足够的语音内容".to_owned())
    } else {
        None
    };
    crate::evoke_setup::RecordingQuality {
        accepted: rejection.is_none(),
        rms,
        peak,
        clipping_ratio,
        voiced_ratio,
        rejection,
    }
}

pub(crate) fn resample_linear(samples: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
    if samples.is_empty() || input_rate == 0 || output_rate == 0 {
        return Vec::new();
    }
    if input_rate == output_rate {
        return samples.to_vec();
    }
    let output_len = ((samples.len() as u64 * output_rate as u64) / input_rate as u64) as usize;
    (0..output_len)
        .map(|index| {
            let position = index as f64 * input_rate as f64 / output_rate as f64;
            let left = position.floor() as usize;
            let right = (left + 1).min(samples.len() - 1);
            let fraction = (position - left as f64) as f32;
            samples[left] + (samples[right] - samples[left]) * fraction
        })
        .collect()
}
