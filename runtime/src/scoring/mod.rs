use std::collections::VecDeque;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};

use crate::audio::AudioFrame;
use crate::evoke_setup::audio::{frames_to_16k, recording_quality};
use crate::evoke_setup::modes;
use crate::evoke_setup::{EvokeArtifact, EvokeMode, EvokeProfile};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvokeScore {
    pub overall: f32,
    pub threshold: f32,
    pub voice_activity: f32,
    pub phrase_score: f32,
    pub mode_score: f32,
    pub accepted: bool,
    pub mode: EvokeMode,
}

pub struct ScoringSystem {
    profile: EvokeProfile,
    sensitivity: f32,
    frames: VecDeque<AudioFrame>,
    phrase_score: f32,
    last_keyword_hit_ms: Option<u64>,
    speaker: Option<SpeakerEmbeddingExtractor>,
    preview_score: Option<EvokeScore>,
}

impl ScoringSystem {
    pub fn new(
        profile: EvokeProfile,
        sensitivity: f32,
        speaker_model: Option<&Path>,
    ) -> Result<Self, String> {
        let speaker = if profile.mode == EvokeMode::SpeakerVerify {
            let model = speaker_model.ok_or_else(|| {
                "speaker profile is active but the speaker model is unavailable".to_owned()
            })?;
            let mut config = SpeakerEmbeddingExtractorConfig::default();
            config.model = Some(model.display().to_string());
            config.num_threads = 2;
            Some(
                SpeakerEmbeddingExtractor::create(&config)
                    .ok_or_else(|| "failed to create speaker scoring extractor".to_owned())?,
            )
        } else {
            None
        };
        Ok(Self {
            profile,
            sensitivity: sanitize_sensitivity(sensitivity),
            frames: VecDeque::new(),
            phrase_score: 0.0,
            last_keyword_hit_ms: None,
            speaker,
            preview_score: None,
        })
    }

    pub fn profile_id(&self) -> &str {
        &self.profile.id
    }

    pub fn set_sensitivity(&mut self, sensitivity: f32) {
        self.sensitivity = sanitize_sensitivity(sensitivity);
    }

    pub fn reset(&mut self) {
        self.frames.clear();
        self.phrase_score = 0.0;
        self.last_keyword_hit_ms = None;
        self.preview_score = None;
    }

    pub fn push_frame(&mut self, frame: &AudioFrame, keyword_hit: bool) {
        self.frames.push_back(frame.clone());
        let newest = frame.timestamp_ms;
        while self
            .frames
            .front()
            .is_some_and(|oldest| newest.saturating_sub(oldest.timestamp_ms) > 5_200)
        {
            self.frames.pop_front();
        }
        if keyword_hit {
            self.last_keyword_hit_ms = Some(newest);
        }
        self.phrase_score = self
            .last_keyword_hit_ms
            .map(|hit| keyword_score_for_age(newest.saturating_sub(hit)))
            .unwrap_or(0.0);
    }

    pub fn preview(&mut self) -> EvokeScore {
        let raw = self.score(false);
        let smoothed = smooth_preview(self.preview_score.as_ref(), raw);
        self.preview_score = Some(smoothed.clone());
        smoothed
    }

    pub fn evaluate_candidate(&mut self) -> EvokeScore {
        self.phrase_score = 1.0;
        self.score(true)
    }

    fn score(&mut self, full_mode_evaluation: bool) -> EvokeScore {
        let frames = self.frames.iter().cloned().collect::<Vec<_>>();
        let samples = frames_to_16k(&frames);
        let quality = recording_quality(&samples);
        let voice_activity = ((quality.rms - 0.003) / 0.06).clamp(0.0, 1.0).powf(0.35)
            * (0.4 + 0.6 * quality.voiced_ratio.clamp(0.0, 1.0));
        let mode_score = self.mode_score(&samples, full_mode_evaluation, voice_activity);
        let overall = match self.profile.mode {
            EvokeMode::Text => self.phrase_score * 0.82 + voice_activity * 0.18,
            EvokeMode::VoiceMatch | EvokeMode::SpeakerVerify | EvokeMode::Classifier => {
                if full_mode_evaluation {
                    self.phrase_score.min(mode_score)
                } else {
                    mode_score * 0.8 + voice_activity * 0.2
                }
            }
        }
        .clamp(0.0, 1.0);
        let threshold =
            (self.profile.threshold - (self.sensitivity - 0.5) * 0.24).clamp(0.20, 0.92);
        EvokeScore {
            overall,
            threshold,
            voice_activity,
            phrase_score: self.phrase_score,
            mode_score,
            accepted: overall >= threshold,
            mode: self.profile.mode,
        }
    }

    fn mode_score(
        &mut self,
        samples: &[f32],
        full_mode_evaluation: bool,
        voice_activity: f32,
    ) -> f32 {
        match &self.profile.artifact {
            EvokeArtifact::Text { .. } => modes::text::score(voice_activity),
            EvokeArtifact::VoiceMatch { template } => modes::voice_match::score(samples, template),
            EvokeArtifact::SpeakerVerify { centroid } => {
                if !full_mode_evaluation {
                    return voice_activity * 0.55;
                }
                let Some(extractor) = &self.speaker else {
                    return 0.0;
                };
                modes::speaker::score(extractor, centroid, samples)
            }
            EvokeArtifact::Classifier { weights, bias } => {
                modes::classifier::score(weights, *bias, samples)
            }
        }
    }
}

fn keyword_score_for_age(age_ms: u64) -> f32 {
    const HOLD_MS: u64 = 800;
    const RELEASE_TIME_MS: f32 = 700.0;
    if age_ms <= HOLD_MS {
        1.0
    } else {
        (-((age_ms - HOLD_MS) as f32) / RELEASE_TIME_MS).exp()
    }
}

fn smooth_preview(previous: Option<&EvokeScore>, raw: EvokeScore) -> EvokeScore {
    let baseline = EvokeScore {
        overall: 0.0,
        threshold: raw.threshold,
        voice_activity: 0.0,
        phrase_score: 0.0,
        mode_score: 0.0,
        accepted: false,
        mode: raw.mode,
    };
    let previous = previous
        .filter(|score| score.mode == raw.mode)
        .unwrap_or(&baseline);
    let overall = smooth_component(previous.overall, raw.overall);
    let threshold = smooth_component(previous.threshold, raw.threshold);
    let accepted = if previous.accepted {
        overall >= threshold - 0.035
    } else {
        overall >= threshold + 0.01
    };
    EvokeScore {
        overall,
        threshold,
        voice_activity: smooth_component(previous.voice_activity, raw.voice_activity),
        phrase_score: smooth_component(previous.phrase_score, raw.phrase_score),
        mode_score: smooth_component(previous.mode_score, raw.mode_score),
        accepted,
        mode: raw.mode,
    }
}

fn smooth_component(previous: f32, target: f32) -> f32 {
    let alpha = if target >= previous { 0.28 } else { 0.14 };
    let value = previous + (target - previous) * alpha;
    if (target - value).abs() < 0.001 {
        target
    } else {
        value.clamp(0.0, 1.0)
    }
}

fn sanitize_sensitivity(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.65
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_profile() -> EvokeProfile {
        EvokeProfile {
            id: "text".to_owned(),
            mode: EvokeMode::Text,
            phrase: "你好".to_owned(),
            threshold: 0.5,
            artifact: EvokeArtifact::Text {
                keyword_syntax: "n ǐ h ǎo".to_owned(),
            },
            required_asset_ids: Vec::new(),
            created_at_ms: 0,
        }
    }

    #[test]
    fn keyword_hit_raises_real_score() {
        let mut system = ScoringSystem::new(text_profile(), 0.65, None).unwrap();
        let frame = AudioFrame {
            samples: vec![2_000; 1_600],
            sample_rate: 16_000,
            timestamp_ms: 100,
        };
        system.push_frame(&frame, false);
        let idle = system.preview();
        system.push_frame(&frame, true);
        let hit = system.preview();
        assert!(hit.overall > idle.overall);
        assert!(hit.phrase_score > idle.phrase_score);
        assert!(hit.phrase_score < 1.0);
    }

    #[test]
    fn voice_match_preview_moves_before_keyword_hit() {
        let frames = (0..50)
            .map(|frame_index| AudioFrame {
                samples: (0..1_600)
                    .map(|sample_index| {
                        let sample = frame_index * 1_600 + sample_index;
                        ((sample as f32 * 220.0 * std::f32::consts::TAU / 16_000.0).sin() * 6_000.0)
                            as i16
                    })
                    .collect(),
                sample_rate: 16_000,
                timestamp_ms: frame_index as u64 * 100,
            })
            .collect::<Vec<_>>();
        let samples = frames_to_16k(&frames);
        let template = modes::voice_match::feature_sequence(&samples);
        let profile = EvokeProfile {
            id: "voice-match".to_owned(),
            mode: EvokeMode::VoiceMatch,
            phrase: "你好".to_owned(),
            threshold: 0.6,
            artifact: EvokeArtifact::VoiceMatch { template },
            required_asset_ids: Vec::new(),
            created_at_ms: 0,
        };
        let mut system = ScoringSystem::new(profile, 0.65, None).unwrap();
        for frame in &frames {
            system.push_frame(frame, false);
        }

        let preview = system.preview();

        assert_eq!(preview.phrase_score, 0.0);
        assert!(preview.voice_activity > 0.0);
        assert!(preview.mode_score > 0.0);
        assert!(preview.overall > 0.0);
    }

    #[test]
    fn preview_score_approaches_targets_without_single_frame_jumps() {
        let raw = EvokeScore {
            overall: 1.0,
            threshold: 0.6,
            voice_activity: 0.8,
            phrase_score: 1.0,
            mode_score: 0.9,
            accepted: true,
            mode: EvokeMode::VoiceMatch,
        };

        let first = smooth_preview(None, raw.clone());
        let second = smooth_preview(Some(&first), raw);

        assert!(first.overall > 0.0 && first.overall < 0.4);
        assert!(second.overall > first.overall && second.overall < 1.0);
        assert!((second.overall - first.overall) < 0.3);
    }

    #[test]
    fn preview_acceptance_has_release_hysteresis() {
        let previous = EvokeScore {
            overall: 0.62,
            threshold: 0.6,
            voice_activity: 0.6,
            phrase_score: 0.62,
            mode_score: 0.62,
            accepted: true,
            mode: EvokeMode::VoiceMatch,
        };
        let raw = EvokeScore {
            overall: 0.5,
            threshold: 0.6,
            voice_activity: 0.5,
            phrase_score: 0.5,
            mode_score: 0.5,
            accepted: false,
            mode: EvokeMode::VoiceMatch,
        };

        let smoothed = smooth_preview(Some(&previous), raw);

        assert!(smoothed.accepted);
        assert!(smoothed.overall < previous.overall);
    }

    #[test]
    fn keyword_score_holds_long_enough_for_preview_sampling() {
        assert_eq!(keyword_score_for_age(0), 1.0);
        assert_eq!(keyword_score_for_age(800), 1.0);
        assert!(keyword_score_for_age(900) > 0.8);
        assert!(keyword_score_for_age(2_500) < 0.1);
    }

    #[test]
    fn held_text_keyword_crosses_preview_threshold() {
        let mut system = ScoringSystem::new(text_profile(), 1.0, None).unwrap();
        let mut score = system.preview();
        for (index, keyword_hit) in [true, false, false, false].into_iter().enumerate() {
            let frame = AudioFrame {
                samples: vec![2_000; 1_600],
                sample_rate: 16_000,
                timestamp_ms: index as u64 * 100,
            };
            system.push_frame(&frame, keyword_hit);
            score = system.preview();
        }

        assert!(score.phrase_score > 0.6);
        assert!(score.overall >= score.threshold);
        assert!(score.accepted);
    }
}
