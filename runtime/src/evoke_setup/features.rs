use std::path::Path;
use std::sync::Arc;

use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;

use crate::audio::AudioFrame;

pub const FEATURE_DIM: usize = 12;
pub const TEMPLATE_FRAMES: usize = 64;

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

pub fn extract_feature_sequence(samples: &[f32]) -> Vec<Vec<f32>> {
    const FRAME: usize = 400;
    const HOP: usize = 160;
    const FFT_SIZE: usize = 512;
    if samples.len() < FRAME {
        return Vec::new();
    }
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);
    let fft = Arc::clone(&fft);
    let band_edges = logarithmic_edges(2, 220, FEATURE_DIM);
    let mut sequence = Vec::new();
    let mut offset = 0;
    while offset + FRAME <= samples.len() {
        let chunk = &samples[offset..offset + FRAME];
        let rms = (chunk.iter().map(|sample| sample * sample).sum::<f32>() / FRAME as f32).sqrt();
        if rms >= 0.003 {
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

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let numerator = dot(left, right);
    let left_norm = dot(left, left).sqrt();
    let right_norm = dot(right, right).sqrt();
    if left_norm <= f32::EPSILON || right_norm <= f32::EPSILON {
        0.0
    } else {
        (numerator / (left_norm * right_norm)).clamp(-1.0, 1.0)
    }
}

pub fn average_embeddings(embeddings: &[Vec<f32>]) -> Vec<f32> {
    let Some(first) = embeddings.first() else {
        return Vec::new();
    };
    let mut result = vec![0.0; first.len()];
    for embedding in embeddings {
        for (target, value) in result.iter_mut().zip(embedding) {
            *target += *value;
        }
    }
    for value in &mut result {
        *value /= embeddings.len() as f32;
    }
    normalize_in_place(&mut result);
    result
}

fn resample_linear(samples: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
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

fn cosine_distance(left: &[f32], right: &[f32]) -> f32 {
    (1.0 - cosine_similarity(left, right)).clamp(0.0, 2.0)
}

fn normalize_in_place(values: &mut [f32]) {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for value in values {
            *value /= norm;
        }
    }
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

fn sigmoid(value: f32) -> f32 {
    if value >= 0.0 {
        1.0 / (1.0 + (-value).exp())
    } else {
        let exp = value.exp();
        exp / (1.0 + exp)
    }
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
    fn logistic_training_separates_simple_examples() {
        let (weights, bias) = train_logistic(&[vec![1.0, 0.0], vec![0.9, 0.1]], &[vec![0.0, 1.0]]);
        assert!(
            classifier_score(&weights, bias, &[1.0, 0.0])
                > classifier_score(&weights, bias, &[0.0, 1.0])
        );
    }
}
