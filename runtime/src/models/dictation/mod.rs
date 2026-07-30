//! Manifest-driven dictation recognition.

mod offline_generative;
mod online_transducer;
mod utterance_segmenter;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::Duration;

use sherpa_onnx::LinearResampler;

use crate::audio::AudioFrame;
use crate::storage::{AssetDescriptor, AssetKind, OutputMode, RecognizerDescriptor};

use super::onnx_session::ModelError;

use offline_generative::OfflineGenerativeRecognizer;
use online_transducer::OnlineTransducerRecognizer;

static RECOGNIZER_ACTIVITY_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
pub struct DictationModelSpec {
    pub id: String,
    pub root: PathBuf,
    pub recognizer: RecognizerDescriptor,
}

impl DictationModelSpec {
    pub fn from_descriptor(
        descriptor: &AssetDescriptor,
        root: PathBuf,
    ) -> Result<Self, ModelError> {
        if descriptor.kind != AssetKind::DictationModel {
            return Err(ModelError(format!(
                "asset '{}' is not a dictation model",
                descriptor.id
            )));
        }
        let recognizer = descriptor.recognizer.clone().ok_or_else(|| {
            ModelError(format!(
                "dictation model '{}' has no recognizer configuration",
                descriptor.id
            ))
        })?;
        Ok(Self {
            id: descriptor.id.clone(),
            root,
            recognizer,
        })
    }

    pub fn output_mode(&self) -> OutputMode {
        self.recognizer.output_mode()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionUpdate {
    pub full_text: String,
    pub is_final: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadState {
    Unloaded,
    Loading,
    Loaded,
}

trait DictationRecognizer: Send {
    fn output_mode(&self) -> OutputMode;

    fn accept_samples(&mut self, samples: &[f32]) -> Result<Vec<TranscriptionUpdate>, ModelError>;

    fn poll_updates(&mut self) -> Result<Vec<TranscriptionUpdate>, ModelError>;

    fn discard_pending(&mut self);
}

pub struct DictationModelEngine {
    spec: Option<DictationModelSpec>,
    load_state: LoadState,
    recognizer: Option<Box<dyn DictationRecognizer>>,
    audio: AudioConverter,
}

impl DictationModelEngine {
    pub fn new(spec: Option<DictationModelSpec>) -> Self {
        Self {
            spec,
            load_state: LoadState::Unloaded,
            recognizer: None,
            audio: AudioConverter::default(),
        }
    }

    pub fn load_state(&self) -> LoadState {
        self.load_state
    }

    pub fn spec(&self) -> Option<&DictationModelSpec> {
        self.spec.as_ref()
    }

    pub fn set_spec(&mut self, spec: Option<DictationModelSpec>) -> Result<(), ModelError> {
        if self.load_state != LoadState::Unloaded {
            return Err(ModelError(
                "cannot change dictation model while it is loaded".to_owned(),
            ));
        }
        self.spec = spec;
        Ok(())
    }

    pub async fn load(&mut self) -> Result<(), ModelError> {
        self.load_with_cancellation(Arc::new(AtomicBool::new(false)))
            .await
    }

    pub async fn load_with_cancellation(
        &mut self,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(), ModelError> {
        if self.load_state != LoadState::Unloaded {
            return Err(ModelError(format!(
                "dictation model cannot load while in {:?} state",
                self.load_state
            )));
        }
        let spec = self
            .spec
            .clone()
            .ok_or_else(|| ModelError("no dictation model is configured".to_owned()))?;
        self.load_state = LoadState::Loading;
        let loaded = tokio::task::spawn_blocking(move || {
            if cancelled.load(Ordering::Acquire) {
                return Err(ModelError(
                    "dictation model loading was cancelled".to_owned(),
                ));
            }
            let recognizer = create_recognizer(&spec, Arc::clone(&cancelled))?;
            if cancelled.load(Ordering::Acquire) {
                drop(recognizer);
                return Err(ModelError(
                    "dictation model loading was cancelled".to_owned(),
                ));
            }
            Ok(recognizer)
        })
        .await
        .map_err(|error| ModelError(format!("dictation model loading task failed: {error}")));

        match loaded {
            Ok(Ok(recognizer)) => {
                tracing::info!(
                    model_id = %self.spec.as_ref().map(|value| value.id.as_str()).unwrap_or(""),
                    output_mode = ?recognizer.output_mode(),
                    "dictation recognizer loaded"
                );
                self.recognizer = Some(recognizer);
                self.audio = AudioConverter::default();
                self.load_state = LoadState::Loaded;
                Ok(())
            }
            Ok(Err(error)) | Err(error) => {
                self.recognizer = None;
                self.load_state = LoadState::Unloaded;
                Err(error)
            }
        }
    }

    pub fn unload(&mut self) {
        if let Some(recognizer) = self.recognizer.as_mut() {
            recognizer.discard_pending();
        }
        self.recognizer = None;
        self.audio = AudioConverter::default();
        self.load_state = LoadState::Unloaded;
    }

    pub fn discard_pending(&mut self) {
        if let Some(recognizer) = self.recognizer.as_mut() {
            recognizer.discard_pending();
        }
    }

    pub fn process_frame(
        &mut self,
        frame: &AudioFrame,
    ) -> Result<Vec<TranscriptionUpdate>, ModelError> {
        if self.load_state != LoadState::Loaded {
            return Ok(Vec::new());
        }
        let recognizer = self
            .recognizer
            .as_mut()
            .ok_or_else(|| ModelError("loaded dictation model has no recognizer".to_owned()))?;
        let samples = self.audio.convert(frame).unwrap_or_default();
        let mut updates = if samples.is_empty() {
            Vec::new()
        } else {
            recognizer.accept_samples(&samples)?
        };
        updates.extend(recognizer.poll_updates()?);
        Ok(updates)
    }

    pub fn poll_updates(&mut self) -> Result<Vec<TranscriptionUpdate>, ModelError> {
        if self.load_state != LoadState::Loaded {
            return Ok(Vec::new());
        }
        self.recognizer
            .as_mut()
            .ok_or_else(|| ModelError("loaded dictation model has no recognizer".to_owned()))?
            .poll_updates()
    }
}

fn create_recognizer(
    spec: &DictationModelSpec,
    cancelled: Arc<AtomicBool>,
) -> Result<Box<dyn DictationRecognizer>, ModelError> {
    match &spec.recognizer {
        RecognizerDescriptor::OnlineTransducer { .. } => {
            let _activity = acquire_recognizer_activity(&cancelled)?;
            let recognizer = OnlineTransducerRecognizer::create(spec)?;
            if cancelled.load(Ordering::Acquire) {
                return Err(ModelError(
                    "dictation model loading was cancelled".to_owned(),
                ));
            }
            Ok(Box::new(recognizer))
        }
        RecognizerDescriptor::OfflineGenerative { .. } => Ok(Box::new(
            OfflineGenerativeRecognizer::create(spec, cancelled)?,
        )),
    }
}

fn acquire_recognizer_activity(
    cancelled: &AtomicBool,
) -> Result<MutexGuard<'static, ()>, ModelError> {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(ModelError(
                "dictation model loading was cancelled".to_owned(),
            ));
        }
        match RECOGNIZER_ACTIVITY_LOCK.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err(ModelError(
                    "dictation recognizer activity lock was poisoned".to_owned(),
                ));
            }
        }
    }
}

pub(super) fn assemble_text(committed: &str, partial: &str) -> String {
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
