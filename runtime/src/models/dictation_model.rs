//! DictationModel Engine：本地 ONNX 千问 ASR（见 plan.md §3.1、§5）。
//!
//! 体积较大，仅由 State Machine 在收到 EvokeModel 的唤醒词检测事件后触发加载（唯一入口），
//! 运行于 Loading/Dictating 态，其余时间不存在；用完即卸载，不长期占用资源。

use super::onnx_session::{
    path_string, require_file, resolve_model_file, validate_model_directory, ModelError,
    OnnxSession,
};
use crate::audio::AudioFrame;
use sherpa_onnx::{LinearResampler, OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};
use std::sync::Mutex;

static MODEL_LOAD_LOCK: Mutex<()> = Mutex::new(());

/// 一次流式识别的增量结果：`full_text` 是"当前完整识别文本"（非单纯 delta），
/// 由 `TextDiffEngine` 负责与上一次结果比较、计算新增后缀（见 plan.md §8.1）。
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionUpdate {
    pub full_text: String,
    /// 是否为本段语音的最终结果（Qwen ASR 流式输出常见 partial/final 区分）
    pub is_final: bool,
}

/// DictationModel Engine 的加载状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadState {
    Unloaded,
    Loading,
    Loaded,
}

/// DictationModel Engine。
pub struct DictationModelEngine {
    model_path: String,
    session: Option<OnnxSession>,
    load_state: LoadState,
    runtime: Option<DictationRuntime>,
    audio: AudioConverter,
    committed_segments: String,
    last_emitted: String,
}

impl DictationModelEngine {
    /// 构造时不加载模型（`load_state` 为 `Unloaded`），仅记录模型路径。
    pub fn new(model_path: &str) -> Self {
        Self {
            model_path: model_path.to_owned(),
            session: None,
            load_state: LoadState::Unloaded,
            runtime: None,
            audio: AudioConverter::default(),
            committed_segments: String::new(),
            last_emitted: String::new(),
        }
    }

    pub fn load_state(&self) -> LoadState {
        self.load_state
    }

    pub(crate) fn model_path(&self) -> &str {
        &self.model_path
    }

    pub(crate) fn set_model_path(&mut self, model_path: &str) -> Result<(), ModelError> {
        if self.load_state != LoadState::Unloaded {
            return Err(ModelError(
                "cannot change dictation model path while the model is loaded".to_owned(),
            ));
        }
        self.model_path = model_path.to_owned();
        Ok(())
    }

    /// 异步加载模型（`State::Loading` 态触发，对应架构图连线 "②仅Evoke触发后加载"）。
    pub async fn load(&mut self) -> Result<(), ModelError> {
        if self.load_state != LoadState::Unloaded {
            return Err(ModelError(format!(
                "dictation model cannot load while in {:?} state",
                self.load_state
            )));
        }
        self.load_state = LoadState::Loading;
        let model_path = self.model_path.clone();
        let loaded = tokio::task::spawn_blocking(move || {
            let _load_guard = MODEL_LOAD_LOCK
                .lock()
                .map_err(|_| ModelError("dictation model load lock was poisoned".to_owned()))?;
            let session = OnnxSession::load(&model_path)?;
            let runtime = DictationRuntime::create(&model_path)?;
            Ok::<_, ModelError>((session, runtime))
        })
        .await
        .map_err(|error| ModelError(format!("dictation model loading task failed: {error}")));

        match loaded {
            Ok(Ok((session, runtime))) => {
                self.session = Some(session);
                self.runtime = Some(runtime);
                self.audio = AudioConverter::default();
                self.committed_segments.clear();
                self.last_emitted.clear();
                self.load_state = LoadState::Loaded;
                Ok(())
            }
            Ok(Err(error)) | Err(error) => {
                self.session = None;
                self.runtime = None;
                self.load_state = LoadState::Unloaded;
                Err(error)
            }
        }
    }

    /// 卸载模型，释放资源（`State::Unloading` 态触发）。
    pub fn unload(&mut self) {
        self.runtime = None;
        if let Some(session) = self.session.take() {
            session.unload();
        }
        self.audio = AudioConverter::default();
        self.committed_segments.clear();
        self.last_emitted.clear();
        self.load_state = LoadState::Unloaded;
    }

    /// 送入一帧音频（含 Ring Buffer 补喂的历史帧，以及后续实时流），返回增量识别结果。
    /// 仅在 `load_state() == Loaded` 时应被调用。
    pub fn process_frame(&mut self, frame: &AudioFrame) -> Option<TranscriptionUpdate> {
        if self.load_state != LoadState::Loaded {
            return None;
        }
        let samples = self.audio.convert(frame)?;
        if samples.is_empty() {
            return None;
        }

        let runtime = self.runtime.as_ref()?;
        runtime.stream.accept_waveform(16_000, &samples);
        while runtime.recognizer.is_ready(&runtime.stream) {
            runtime.recognizer.decode(&runtime.stream);
        }

        let result = runtime.recognizer.get_result(&runtime.stream)?;
        let endpoint = runtime.recognizer.is_endpoint(&runtime.stream);
        let mut full_text = assemble_text(&self.committed_segments, result.text.trim());
        let is_final = endpoint || result.is_final;

        if endpoint {
            self.committed_segments = full_text.clone();
            runtime.recognizer.reset(&runtime.stream);
        }

        if full_text == self.last_emitted && !is_final {
            return None;
        }
        if full_text.is_empty() {
            return None;
        }
        self.last_emitted = std::mem::take(&mut full_text);
        Some(TranscriptionUpdate {
            full_text: self.last_emitted.clone(),
            is_final,
        })
    }
}

struct DictationRuntime {
    stream: OnlineStream,
    recognizer: OnlineRecognizer,
}

impl DictationRuntime {
    fn create(model_path: &str) -> Result<Self, ModelError> {
        let directory = validate_model_directory(model_path)?;
        let encoder = resolve_model_file(
            &directory,
            "encoder",
            &[
                "encoder-epoch-99-avg-1.int8.onnx",
                "encoder-epoch-99-avg-1.onnx",
                "encoder.onnx",
            ],
        )?;
        let decoder = resolve_model_file(
            &directory,
            "decoder",
            &["decoder-epoch-99-avg-1.onnx", "decoder.onnx"],
        )?;
        let joiner = resolve_model_file(
            &directory,
            "joiner",
            &[
                "joiner-epoch-99-avg-1.int8.onnx",
                "joiner-epoch-99-avg-1.onnx",
                "joiner.onnx",
            ],
        )?;
        let tokens = require_file(&directory, "tokens.txt")?;

        let mut config = OnlineRecognizerConfig::default();
        config.model_config.transducer.encoder = Some(path_string(&encoder)?);
        config.model_config.transducer.decoder = Some(path_string(&decoder)?);
        config.model_config.transducer.joiner = Some(path_string(&joiner)?);
        config.model_config.tokens = Some(path_string(&tokens)?);
        config.model_config.num_threads = 2;
        config.decoding_method = Some("greedy_search".to_owned());
        config.enable_endpoint = true;
        config.rule1_min_trailing_silence = 2.4;
        config.rule2_min_trailing_silence = 1.2;
        config.rule3_min_utterance_length = 20.0;

        let recognizer = OnlineRecognizer::create(&config).ok_or_else(|| {
            ModelError("failed to create sherpa-onnx online recognizer".to_owned())
        })?;
        let stream = recognizer.create_stream();
        Ok(Self { stream, recognizer })
    }
}

fn assemble_text(committed: &str, partial: &str) -> String {
    if committed.is_empty() {
        return partial.to_owned();
    }
    if partial.is_empty() {
        return committed.to_owned();
    }

    let needs_space = committed
        .chars()
        .next_back()
        .zip(partial.chars().next())
        .is_some_and(|(left, right)| left.is_ascii_alphanumeric() && right.is_ascii_alphanumeric());
    format!("{committed}{}{partial}", if needs_space { " " } else { "" })
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
        let samples = frame
            .samples
            .iter()
            .map(|sample| f32::from(*sample) / 32768.0)
            .collect::<Vec<_>>();
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
}

#[cfg(test)]
mod tests {
    use super::assemble_text;

    #[test]
    fn assembles_committed_and_partial_text() {
        assert_eq!(assemble_text("", "你好"), "你好");
        assert_eq!(assemble_text("你好", "世界"), "你好世界");
        assert_eq!(assemble_text("hello", "world"), "hello world");
        assert_eq!(assemble_text("hello ", "世界"), "hello 世界");
        assert_eq!(assemble_text("完成。", ""), "完成。");
    }
}
