//! EvokeModel Engine：ONNX 唤醒词模型（见 plan.md §3.1、§5）。
//!
//! 目标 <20MB，常驻内存，长期运行；自训练小型关键词检测模型（类 openWakeWord 思路）。
//! 仅在 `State::Listening` 态激活；检测到唤醒词是触发 `DictationModelEngine` 加载的唯一入口。

use super::onnx_session::{
    path_string, require_file, resolve_model_file, validate_model_directory, ModelError,
    OnnxSession,
};
use crate::audio::AudioFrame;
use pinyin::ToPinyin;
use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig, LinearResampler, OnlineStream};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// 唤醒词检测结果。
#[derive(Debug, Clone, PartialEq)]
pub struct WakeWordDetection {
    /// 置信度 0.0-1.0，与 `sensitivity` 阈值比较后决定是否上报
    pub confidence: f32,
    pub detected_at_ms: u64,
}

/// EvokeModel Engine。
pub struct EvokeModelEngine {
    _session: OnnxSession,
    /// 敏感度（对应 EvokeWord 设置页滑杆，0.0=不易误触发 ~ 1.0=更容易唤醒）
    sensitivity: f32,
    /// 当前生效的唤醒词（同一时间只能有 1 个生效，见 plan.md §6.3）
    active_word: String,
    keyword_syntax: String,
    expected_keyword: String,
    token_set: HashSet<String>,
    runtime: KwsRuntime,
    audio: AudioConverter,
}

impl EvokeModelEngine {
    /// 常驻加载：Runtime 启动时调用一次，此后长期驻留内存。
    ///
    /// `max_active_paths` 是 KWS 解码的 beam 宽度，由调用方按唤醒模式决定，
    /// 见 [`EvokeMode::kws_max_active_paths`]。
    ///
    /// [`EvokeMode::kws_max_active_paths`]: crate::evoke_setup::EvokeMode::kws_max_active_paths
    pub fn new(
        model_path: &str,
        active_word: String,
        max_active_paths: i32,
    ) -> Result<Self, ModelError> {
        let session = OnnxSession::load(model_path)?;
        let directory = validate_model_directory(model_path)?;
        let files = KwsModelFiles::resolve(&directory)?;
        let token_set = load_token_set(&files.tokens)?;
        let sensitivity = 0.65;
        let normalized = normalize_keyword(&active_word, &token_set)?;
        let keyword_syntax = normalized.with_threshold(sensitivity_to_threshold(sensitivity));
        let runtime = KwsRuntime::create(&files, &keyword_syntax, max_active_paths)?;

        Ok(Self {
            _session: session,
            sensitivity,
            active_word,
            keyword_syntax,
            expected_keyword: normalized.alias,
            token_set,
            runtime,
            audio: AudioConverter::default(),
        })
    }

    pub fn active_word(&self) -> &str {
        &self.active_word
    }

    /// 当前生效的 sherpa-onnx keyword 语法（含派生出的拼音 token），用于诊断。
    pub fn keyword_syntax(&self) -> &str {
        &self.keyword_syntax
    }

    /// 切换生效唤醒词（EvokeWord 设置页），可能需要重新加载/微调模型权重。
    pub fn set_active_word(&mut self, word: String) -> Result<(), ModelError> {
        let normalized = normalize_keyword(&word, &self.token_set)?;
        let keyword_syntax = normalized.with_threshold(sensitivity_to_threshold(self.sensitivity));
        let stream = self
            .runtime
            .spotter
            .create_stream_with_keywords(&keyword_syntax);

        self.runtime.stream = stream;
        self.audio.reset();
        self.active_word = word;
        self.expected_keyword = normalized.alias;
        self.keyword_syntax = keyword_syntax;
        Ok(())
    }

    pub fn sensitivity(&self) -> f32 {
        self.sensitivity
    }

    /// 设置敏感度（EvokeWord 设置页滑杆）。
    pub fn set_sensitivity(&mut self, value: f32) {
        let value = sanitize_sensitivity(value);
        let normalized =
            NormalizedKeyword::from_syntax(&self.keyword_syntax, &self.expected_keyword);
        let keyword_syntax = normalized.with_threshold(sensitivity_to_threshold(value));
        let stream = self
            .runtime
            .spotter
            .create_stream_with_keywords(&keyword_syntax);

        self.runtime.stream = stream;
        self.audio.reset();
        self.sensitivity = value;
        self.keyword_syntax = keyword_syntax;
    }

    pub fn reset(&mut self) {
        self.runtime.stream = self
            .runtime
            .spotter
            .create_stream_with_keywords(&self.keyword_syntax);
        self.audio.reset();
    }

    /// 送入一帧音频，返回是否检测到唤醒词（仅在 `State::Listening` 态被调用）。
    pub fn process_frame(&mut self, frame: &AudioFrame) -> Option<WakeWordDetection> {
        let samples = self.audio.convert(frame)?;
        if samples.is_empty() {
            return None;
        }

        self.runtime.stream.accept_waveform(16_000, &samples);
        while self.runtime.spotter.is_ready(&self.runtime.stream) {
            self.runtime.spotter.decode(&self.runtime.stream);
            let Some(result) = self.runtime.spotter.get_result(&self.runtime.stream) else {
                continue;
            };
            if !result.keyword.is_empty() {
                tracing::debug!(
                    detected_keyword = %result.keyword,
                    expected_keyword = %self.expected_keyword,
                    matched = result.keyword == self.expected_keyword,
                    "wake keyword result"
                );
            }
            if result.keyword == self.expected_keyword {
                self.runtime.spotter.reset(&self.runtime.stream);
                return Some(WakeWordDetection {
                    confidence: 1.0,
                    detected_at_ms: frame.timestamp_ms,
                });
            }
        }
        None
    }

    /// 用用户声音样本对唤醒词模型做个性化微调（Future Work，占位接口，见 plan.md §10）。
    pub fn fine_tune_with_voice_samples(
        &mut self,
        _samples: Vec<AudioFrame>,
    ) -> Result<(), ModelError> {
        Err(ModelError(
            "wake-word voice fine-tuning is not supported in this version".to_owned(),
        ))
    }
}

struct KwsRuntime {
    stream: OnlineStream,
    spotter: KeywordSpotter,
}

impl KwsRuntime {
    fn create(
        files: &KwsModelFiles,
        keyword_syntax: &str,
        max_active_paths: i32,
    ) -> Result<Self, ModelError> {
        let mut config = KeywordSpotterConfig::default();
        config.model_config.transducer.encoder = Some(path_string(&files.encoder)?);
        config.model_config.transducer.decoder = Some(path_string(&files.decoder)?);
        config.model_config.transducer.joiner = Some(path_string(&files.joiner)?);
        config.model_config.tokens = Some(path_string(&files.tokens)?);
        config.model_config.num_threads = 1;
        config.max_active_paths = max_active_paths;
        config.keywords_buf = Some(keyword_syntax.to_owned());
        config.keywords_threshold = sensitivity_to_threshold(0.65);

        let spotter = KeywordSpotter::create(&config)
            .ok_or_else(|| ModelError("failed to create sherpa-onnx keyword spotter".to_owned()))?;
        let stream = spotter.create_stream_with_keywords(keyword_syntax);
        Ok(Self { stream, spotter })
    }
}

struct KwsModelFiles {
    encoder: std::path::PathBuf,
    decoder: std::path::PathBuf,
    joiner: std::path::PathBuf,
    tokens: std::path::PathBuf,
}

impl KwsModelFiles {
    fn resolve(directory: &std::path::Path) -> Result<Self, ModelError> {
        Ok(Self {
            encoder: resolve_model_file(
                directory,
                "encoder",
                &[
                    "encoder-epoch-12-avg-2-chunk-16-left-64.onnx",
                    "encoder.onnx",
                ],
            )?,
            decoder: resolve_model_file(
                directory,
                "decoder",
                &[
                    "decoder-epoch-12-avg-2-chunk-16-left-64.onnx",
                    "decoder.onnx",
                ],
            )?,
            joiner: resolve_model_file(
                directory,
                "joiner",
                &["joiner-epoch-12-avg-2-chunk-16-left-64.onnx", "joiner.onnx"],
            )?,
            tokens: require_file(directory, "tokens.txt")?,
        })
    }
}

#[derive(Clone)]
struct NormalizedKeyword {
    tokens: String,
    alias: String,
    score: Option<f32>,
}

impl NormalizedKeyword {
    fn with_threshold(&self, threshold: f32) -> String {
        let score = self
            .score
            .map(|score| format!(" :{score}"))
            .unwrap_or_default();
        format!("{} @{}{} #{threshold:.3}", self.tokens, self.alias, score)
    }

    fn from_syntax(syntax: &str, alias: &str) -> Self {
        let tokens = syntax
            .split_whitespace()
            .take_while(|part| !part.starts_with('@'))
            .collect::<Vec<_>>()
            .join(" ");
        let score = syntax
            .split_whitespace()
            .find_map(|part| part.strip_prefix(':'))
            .and_then(|value| value.parse().ok());
        Self {
            tokens,
            alias: alias.to_owned(),
            score,
        }
    }
}

fn normalize_keyword(
    word: &str,
    token_set: &HashSet<String>,
) -> Result<NormalizedKeyword, ModelError> {
    let trimmed = word.trim();
    if trimmed.is_empty() {
        return Err(ModelError("wake word must not be empty".to_owned()));
    }
    if trimmed.contains(['\0', '\r', '\n', '/']) {
        return Err(ModelError(
            "wake word must contain exactly one sherpa-onnx keyword".to_owned(),
        ));
    }

    let normalized = if trimmed.contains('@') {
        parse_advanced_keyword(trimmed)?
    } else {
        let tokens = match trimmed {
            "你好" => "n ǐ h ǎo",
            "小助手" => "x iǎo zh ù sh ǒu",
            "小爱同学" => "x iǎo ài t óng x ué",
            "你好小智" => "n ǐ h ǎo x iǎo zh ì",
            "贾维斯" => "j iǎ w éi s ī",
            "开始工作" => "k āi sh ǐ g ōng z uò",
            _ => return normalize_plain_keyword(trimmed, token_set),
        };
        NormalizedKeyword {
            tokens: tokens.to_owned(),
            alias: trimmed.to_owned(),
            score: None,
        }
    };

    for token in normalized.tokens.split_whitespace() {
        if !token_set.contains(token) {
            return Err(ModelError(format!(
                "wake-word token '{token}' is not present in the model tokens file"
            )));
        }
    }
    Ok(normalized)
}

fn normalize_plain_keyword(
    value: &str,
    token_set: &HashSet<String>,
) -> Result<NormalizedKeyword, ModelError> {
    let mut tokens = Vec::new();
    for character in value.chars() {
        if character.is_whitespace() {
            continue;
        }
        if character.is_ascii_alphabetic() {
            let token = character.to_string();
            if token_set.contains(&token) {
                tokens.push(token);
                continue;
            }
        }
        let pinyin = character.to_pinyin().ok_or_else(|| {
            ModelError(format!(
                "cannot derive local pinyin tokens for wake-word character '{character}'"
            ))
        })?;
        let syllable = pinyin.with_tone();
        let (initial, final_part) = split_pinyin_syllable(syllable);
        if !initial.is_empty() {
            tokens.push(initial.to_owned());
        }
        tokens.push(final_part.to_owned());
    }
    if tokens.is_empty() {
        return Err(ModelError("wake word produced no model tokens".to_owned()));
    }
    for token in &tokens {
        if !token_set.contains(token) {
            return Err(ModelError(format!(
                "derived wake-word token '{token}' is not present in the model vocabulary"
            )));
        }
    }
    Ok(NormalizedKeyword {
        tokens: tokens.join(" "),
        alias: value.to_owned(),
        score: None,
    })
}

fn split_pinyin_syllable(value: &str) -> (&str, &str) {
    const INITIALS: &[&str] = &[
        "zh", "ch", "sh", "b", "p", "m", "f", "d", "t", "n", "l", "g", "k", "h", "j", "q", "x",
        "r", "z", "c", "s", "y", "w",
    ];
    INITIALS
        .iter()
        .find_map(|initial| value.strip_prefix(initial).map(|rest| (*initial, rest)))
        .filter(|(_, rest)| !rest.is_empty())
        .unwrap_or(("", value))
}

fn parse_advanced_keyword(value: &str) -> Result<NormalizedKeyword, ModelError> {
    let mut tokens = Vec::new();
    let mut alias = None;
    let mut score = None;
    let mut has_threshold = false;
    let mut metadata_started = false;

    for part in value.split_whitespace() {
        if let Some(value) = part.strip_prefix('@') {
            metadata_started = true;
            if alias.replace(value.to_owned()).is_some() || value.is_empty() {
                return Err(ModelError(
                    "advanced wake-word syntax must contain one non-empty @alias".to_owned(),
                ));
            }
        } else if let Some(value) = part.strip_prefix(':') {
            metadata_started = true;
            let parsed = value
                .parse::<f32>()
                .map_err(|_| ModelError("advanced wake-word :score must be a number".to_owned()))?;
            if !parsed.is_finite() || parsed <= 0.0 || score.replace(parsed).is_some() {
                return Err(ModelError(
                    "advanced wake-word :score must be one positive finite number".to_owned(),
                ));
            }
        } else if let Some(value) = part.strip_prefix('#') {
            metadata_started = true;
            let parsed = value.parse::<f32>().map_err(|_| {
                ModelError("advanced wake-word #threshold must be a number".to_owned())
            })?;
            if !parsed.is_finite()
                || !(0.0..=1.0).contains(&parsed)
                || std::mem::replace(&mut has_threshold, true)
            {
                return Err(ModelError(
                    "advanced wake-word #threshold must be one number between 0 and 1".to_owned(),
                ));
            }
        } else if metadata_started {
            return Err(ModelError(
                "keyword tokens must appear before keyword metadata".to_owned(),
            ));
        } else {
            tokens.push(part);
        }
    }

    if tokens.is_empty() {
        return Err(ModelError(
            "advanced wake-word syntax has no keyword tokens".to_owned(),
        ));
    }
    Ok(NormalizedKeyword {
        tokens: tokens.join(" "),
        alias: alias
            .ok_or_else(|| ModelError("advanced wake-word syntax requires an @alias".to_owned()))?,
        score,
    })
}

fn load_token_set(path: &std::path::Path) -> Result<HashSet<String>, ModelError> {
    let content = fs::read_to_string(path).map_err(|error| {
        ModelError(format!(
            "failed to read tokens file '{}': {error}",
            path.display()
        ))
    })?;
    let tokens = content
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .collect::<HashSet<_>>();
    if tokens.is_empty() {
        return Err(ModelError(format!(
            "tokens file '{}' is empty",
            path.display()
        )));
    }
    Ok(tokens)
}

fn sanitize_sensitivity(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.65
    }
}

fn sensitivity_to_threshold(sensitivity: f32) -> f32 {
    0.60 - 0.45 * sanitize_sensitivity(sensitivity)
}

pub(crate) fn keyword_syntax_for_model(
    model_path: &Path,
    word: &str,
    sensitivity: f32,
) -> Result<String, ModelError> {
    let directory = validate_model_directory(
        model_path
            .to_str()
            .ok_or_else(|| ModelError("evoke model path is not valid UTF-8".to_owned()))?,
    )?;
    let files = KwsModelFiles::resolve(&directory)?;
    let tokens = load_token_set(&files.tokens)?;
    Ok(normalize_keyword(word, &tokens)?.with_threshold(sensitivity_to_threshold(sensitivity)))
}

#[derive(Default)]
struct AudioConverter {
    input_rate: Option<u32>,
    resampler: Option<LinearResampler>,
}

impl AudioConverter {
    fn convert(&mut self, frame: &AudioFrame) -> Option<Vec<f32>> {
        if frame.sample_rate == 0 || frame.sample_rate > i32::MAX as u32 {
            return None;
        }
        let samples = normalize_i16(&frame.samples);
        if frame.sample_rate == 16_000 {
            self.input_rate = Some(16_000);
            self.resampler = None;
            return Some(samples);
        }
        if self.input_rate != Some(frame.sample_rate) {
            self.resampler = LinearResampler::create(frame.sample_rate as i32, 16_000);
            self.input_rate = Some(frame.sample_rate);
        }
        self.resampler
            .as_ref()
            .map(|resampler| resampler.resample(&samples, false))
    }

    fn reset(&mut self) {
        if let Some(resampler) = &self.resampler {
            resampler.reset();
        }
    }
}

fn normalize_i16(samples: &[i16]) -> Vec<f32> {
    samples
        .iter()
        .map(|sample| f32::from(*sample) / 32768.0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_i16, normalize_keyword, sanitize_sensitivity, sensitivity_to_threshold,
        AudioConverter,
    };
    use crate::audio::AudioFrame;
    use std::collections::HashSet;

    #[test]
    fn sensitivity_maps_inversely_to_threshold() {
        assert!((sensitivity_to_threshold(0.0) - 0.60).abs() < f32::EPSILON);
        assert!((sensitivity_to_threshold(1.0) - 0.15).abs() < f32::EPSILON);
        assert!(sensitivity_to_threshold(0.75) < sensitivity_to_threshold(0.25));
        assert_eq!(sanitize_sensitivity(-2.0), 0.0);
        assert_eq!(sanitize_sensitivity(2.0), 1.0);
        assert_eq!(sanitize_sensitivity(f32::NAN), 0.65);
    }

    #[test]
    fn normalizes_and_statefully_resamples_audio() {
        assert_eq!(normalize_i16(&[i16::MIN, 0]), vec![-1.0, 0.0]);
        let mut converter = AudioConverter::default();
        let frame = AudioFrame {
            samples: vec![0; 480],
            sample_rate: 48_000,
            timestamp_ms: 0,
        };
        let first = converter.convert(&frame).expect("valid sample rate");
        let second = converter.convert(&frame).expect("valid sample rate");
        assert!(!first.is_empty());
        assert!(!second.is_empty());
        assert_eq!(converter.input_rate, Some(48_000));
    }

    #[test]
    fn normalizes_common_and_advanced_keywords() {
        let tokens = ["n", "ǐ", "h", "ǎo", "x", "iǎo", "zh", "ù", "sh", "ǒu"]
            .into_iter()
            .map(str::to_owned)
            .collect::<HashSet<_>>();
        let default = normalize_keyword("你好", &tokens).expect("supported default keyword");
        assert_eq!(default.alias, "你好");
        assert_eq!(default.tokens, "n ǐ h ǎo");

        let common = normalize_keyword("小助手", &tokens).expect("supported common keyword");
        assert_eq!(common.alias, "小助手");
        assert_eq!(common.tokens, "x iǎo zh ù sh ǒu");

        let advanced = normalize_keyword("x iǎo zh ù sh ǒu @私人助手 :1.5 #0.25", &tokens)
            .expect("valid advanced keyword");
        assert_eq!(advanced.alias, "私人助手");
        assert_eq!(advanced.score, Some(1.5));
        assert!(normalize_keyword("任意未转换中文", &tokens).is_err());

        let dynamic_tokens = ["t", "iān", "q", "ì"]
            .into_iter()
            .map(str::to_owned)
            .collect::<HashSet<_>>();
        let dynamic = normalize_keyword("天气", &dynamic_tokens)
            .expect("arbitrary Chinese text should derive local pinyin tokens");
        assert_eq!(dynamic.tokens, "t iān q ì");
    }
}
