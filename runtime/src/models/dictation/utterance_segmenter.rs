use std::collections::VecDeque;

use crate::storage::SegmentationOptions;

const SAMPLE_RATE: usize = 16_000;

pub(super) struct UtteranceSegmenter {
    options: SegmentationOptions,
    pre_roll: VecDeque<f32>,
    active: Vec<f32>,
    speech_samples: usize,
    trailing_silence_samples: usize,
}

impl UtteranceSegmenter {
    pub(super) fn new(options: SegmentationOptions) -> Self {
        Self {
            options,
            pre_roll: VecDeque::new(),
            active: Vec::new(),
            speech_samples: 0,
            trailing_silence_samples: 0,
        }
    }

    pub(super) fn push(&mut self, samples: &[f32]) -> Vec<Vec<f32>> {
        if samples.is_empty() {
            return Vec::new();
        }
        let maximum_samples = samples_for_ms(self.options.maximum_utterance_ms);
        let mut utterances = Vec::new();
        let mut remaining = samples;

        while !remaining.is_empty() {
            if self.active.is_empty() {
                if rms(remaining) < self.options.speech_threshold {
                    self.push_pre_roll(remaining);
                    break;
                }
                self.active.extend(self.pre_roll.drain(..));
                self.speech_samples = 0;
                self.trailing_silence_samples = 0;
            }

            let available = maximum_samples.saturating_sub(self.active.len());
            let take = available.min(remaining.len());
            let chunk = &remaining[..take];
            self.active.extend_from_slice(chunk);
            if rms(chunk) >= self.options.speech_threshold {
                self.speech_samples = self.speech_samples.saturating_add(chunk.len());
                self.trailing_silence_samples = 0;
            } else {
                self.trailing_silence_samples =
                    self.trailing_silence_samples.saturating_add(chunk.len());
            }
            remaining = &remaining[take..];

            if let Some(utterance) = self.finish_if_needed() {
                utterances.push(utterance);
            }
            if take == 0 {
                break;
            }
        }
        utterances
    }

    pub(super) fn reset(&mut self) {
        self.pre_roll.clear();
        self.active.clear();
        self.speech_samples = 0;
        self.trailing_silence_samples = 0;
    }

    fn push_pre_roll(&mut self, samples: &[f32]) {
        self.pre_roll.extend(samples.iter().copied());
        let capacity = samples_for_ms(self.options.pre_roll_ms);
        while self.pre_roll.len() > capacity {
            self.pre_roll.pop_front();
        }
    }

    fn finish_if_needed(&mut self) -> Option<Vec<f32>> {
        let reached_maximum =
            self.active.len() >= samples_for_ms(self.options.maximum_utterance_ms);
        let reached_endpoint = self.speech_samples
            >= samples_for_ms(self.options.minimum_speech_ms)
            && self.trailing_silence_samples >= samples_for_ms(self.options.trailing_silence_ms);
        if !reached_maximum && !reached_endpoint {
            return None;
        }

        let enough_speech = self.speech_samples >= samples_for_ms(self.options.minimum_speech_ms);
        let utterance = std::mem::take(&mut self.active);
        self.speech_samples = 0;
        self.trailing_silence_samples = 0;
        self.pre_roll.clear();
        enough_speech.then_some(utterance)
    }
}

fn samples_for_ms(milliseconds: u64) -> usize {
    usize::try_from(milliseconds)
        .unwrap_or(usize::MAX)
        .saturating_mul(SAMPLE_RATE)
        / 1000
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mean_square = samples
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum::<f64>()
        / samples.len() as f64;
    mean_square.sqrt() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> SegmentationOptions {
        SegmentationOptions {
            pre_roll_ms: 100,
            minimum_speech_ms: 100,
            trailing_silence_ms: 100,
            maximum_utterance_ms: 1000,
            speech_threshold: 0.01,
        }
    }

    #[test]
    fn emits_after_minimum_speech_and_trailing_silence() {
        let mut segmenter = UtteranceSegmenter::new(options());
        assert!(segmenter.push(&vec![0.0; 800]).is_empty());
        assert!(segmenter.push(&vec![0.2; 1600]).is_empty());
        let utterance = segmenter.push(&vec![0.0; 1600]).remove(0);
        assert_eq!(utterance.len(), 4000);
    }

    #[test]
    fn discards_noise_shorter_than_minimum_speech() {
        let mut segmenter = UtteranceSegmenter::new(options());
        assert!(segmenter.push(&vec![0.2; 800]).is_empty());
        assert!(segmenter.push(&vec![0.0; 16_000]).is_empty());
    }

    #[test]
    fn caps_long_utterances() {
        let mut segmenter = UtteranceSegmenter::new(options());
        assert!(segmenter.push(&vec![0.2; 8000]).is_empty());
        let utterance = segmenter.push(&vec![0.2; 8000]).remove(0);
        assert_eq!(utterance.len(), 16_000);
    }

    #[test]
    fn preserves_samples_after_maximum_boundary() {
        let mut options = options();
        options.minimum_speech_ms = 50;
        options.maximum_utterance_ms = 100;
        options.trailing_silence_ms = 50;
        let mut segmenter = UtteranceSegmenter::new(options);

        let first = segmenter.push(&vec![0.2; 2400]);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].len(), 1600);

        let second = segmenter.push(&vec![0.0; 800]);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].len(), 1600);
    }
}
