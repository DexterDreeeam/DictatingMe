use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::thread;

use sherpa_onnx::{OfflineQwen3ASRModelConfig, OfflineRecognizer, OfflineRecognizerConfig};

use crate::storage::{OutputMode, RecognizerDescriptor};

use super::super::onnx_session::{path_string, require_file, validate_model_directory, ModelError};
use super::utterance_segmenter::UtteranceSegmenter;
use super::{
    acquire_recognizer_activity, assemble_text, DictationModelSpec, DictationRecognizer,
    TranscriptionUpdate,
};

struct RecognitionJob {
    generation: u64,
    sequence: u64,
    samples: Vec<f32>,
}

struct RecognitionResult {
    generation: u64,
    sequence: u64,
    result: Result<String, ModelError>,
}

pub(super) struct OfflineGenerativeRecognizer {
    segmenter: UtteranceSegmenter,
    jobs: Sender<RecognitionJob>,
    results: Receiver<RecognitionResult>,
    active_generation: Arc<AtomicU64>,
    generation: u64,
    next_sequence: u64,
    committed: String,
}

impl OfflineGenerativeRecognizer {
    pub(super) fn create(
        spec: &DictationModelSpec,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<Self, ModelError> {
        let RecognizerDescriptor::OfflineGenerative {
            output_mode,
            artifacts,
            options,
            ..
        } = &spec.recognizer
        else {
            return Err(ModelError(format!(
                "model '{}' is not an offline generative recognizer",
                spec.id
            )));
        };
        if *output_mode != OutputMode::Utterance {
            return Err(ModelError(format!(
                "offline generative recognizer '{}' must use utterance output",
                spec.id
            )));
        }
        let directory = validate_model_directory(
            spec.root
                .to_str()
                .ok_or_else(|| ModelError("dictation model path is not UTF-8".to_owned()))?,
        )?;
        let frontend = require_file(&directory, &artifacts.frontend)?;
        let encoder = require_file(&directory, &artifacts.encoder)?;
        let decoder = require_file(&directory, &artifacts.decoder)?;
        let tokenizer = directory.join(&artifacts.tokenizer);
        if !tokenizer.is_dir() {
            return Err(ModelError(format!(
                "required tokenizer directory '{}' is missing",
                tokenizer.display()
            )));
        }

        let mut config = OfflineRecognizerConfig::default();
        config.model_config.qwen3_asr = OfflineQwen3ASRModelConfig {
            conv_frontend: Some(path_string(&frontend)?),
            encoder: Some(path_string(&encoder)?),
            decoder: Some(path_string(&decoder)?),
            tokenizer: Some(path_string(&tokenizer)?),
            max_total_len: options.max_total_length,
            max_new_tokens: options.max_new_tokens,
            ..Default::default()
        };
        config.model_config.tokens = Some(String::new());
        config.model_config.provider = Some("cpu".to_owned());
        config.model_config.num_threads = options.num_threads;
        let (job_sender, job_receiver) = channel();
        let (result_sender, result_receiver) = channel();
        let (ready_sender, ready_receiver) = channel();
        let active_generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&active_generation);
        let model_id = spec.id.clone();
        thread::Builder::new()
            .name("dictatingme-offline-asr".to_owned())
            .spawn(move || {
                worker_loop(
                    config,
                    model_id,
                    job_receiver,
                    result_sender,
                    worker_generation,
                    cancelled,
                    ready_sender,
                )
            })
            .map_err(|error| {
                ModelError(format!(
                    "failed to start offline recognition worker: {error}"
                ))
            })?;
        ready_receiver.recv().map_err(|_| {
            ModelError("offline recognition worker stopped during startup".to_owned())
        })??;

        Ok(Self {
            segmenter: UtteranceSegmenter::new(options.segmentation.clone()),
            jobs: job_sender,
            results: result_receiver,
            active_generation,
            generation: 0,
            next_sequence: 0,
            committed: String::new(),
        })
    }

    fn queue_utterance(&mut self, samples: Vec<f32>) -> Result<(), ModelError> {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        let job = RecognitionJob {
            generation: self.generation,
            sequence,
            samples,
        };
        self.jobs
            .send(job)
            .map_err(|_| ModelError("offline recognition worker has stopped".to_owned()))
    }
}

impl DictationRecognizer for OfflineGenerativeRecognizer {
    fn output_mode(&self) -> OutputMode {
        OutputMode::Utterance
    }

    fn accept_samples(&mut self, samples: &[f32]) -> Result<Vec<TranscriptionUpdate>, ModelError> {
        for utterance in self.segmenter.push(samples) {
            self.queue_utterance(utterance)?;
        }
        Ok(Vec::new())
    }

    fn poll_updates(&mut self) -> Result<Vec<TranscriptionUpdate>, ModelError> {
        let mut updates = Vec::new();
        loop {
            match self.results.try_recv() {
                Ok(result) if result.generation != self.generation => continue,
                Ok(result) => {
                    let text = result.result?;
                    let text = text.trim();
                    if text.is_empty() {
                        continue;
                    }
                    tracing::debug!(
                        sequence = result.sequence,
                        text_length = text.chars().count(),
                        "offline utterance recognition completed"
                    );
                    self.committed = assemble_text(&self.committed, text);
                    updates.push(TranscriptionUpdate {
                        full_text: self.committed.clone(),
                        is_final: true,
                    });
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    return Err(ModelError(
                        "offline recognition worker result channel closed".to_owned(),
                    ));
                }
            }
        }
        Ok(updates)
    }

    fn discard_pending(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.active_generation
            .store(self.generation, Ordering::Release);
        self.next_sequence = 0;
        self.committed.clear();
        self.segmenter.reset();
        while self.results.try_recv().is_ok() {}
    }
}

impl Drop for OfflineGenerativeRecognizer {
    fn drop(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.active_generation
            .store(self.generation, Ordering::Release);
    }
}

fn worker_loop(
    config: OfflineRecognizerConfig,
    model_id: String,
    jobs: Receiver<RecognitionJob>,
    results: Sender<RecognitionResult>,
    active_generation: Arc<AtomicU64>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
    ready: Sender<Result<(), ModelError>>,
) {
    let _activity = match acquire_recognizer_activity(&cancelled) {
        Ok(guard) => guard,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let recognizer = match OfflineRecognizer::create(&config) {
        Some(recognizer) => recognizer,
        None => {
            let _ = ready.send(Err(ModelError(format!(
                "failed to create offline generative recognizer for '{model_id}'"
            ))));
            return;
        }
    };
    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        let _ = ready.send(Err(ModelError(
            "dictation model loading was cancelled".to_owned(),
        )));
        return;
    }
    if ready.send(Ok(())).is_err() {
        return;
    }
    while let Ok(job) = jobs.recv() {
        if job.generation != active_generation.load(Ordering::Acquire) {
            continue;
        }
        let stream = recognizer.create_stream();
        stream.accept_waveform(16_000, &job.samples);
        recognizer.decode(&stream);
        if job.generation != active_generation.load(Ordering::Acquire) {
            continue;
        }
        let result = stream
            .get_result()
            .map(|value| value.text)
            .ok_or_else(|| ModelError("offline recognizer returned no result".to_owned()));
        if results
            .send(RecognitionResult {
                generation: job.generation,
                sequence: job.sequence,
                result,
            })
            .is_err()
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use crate::storage::{
        OfflineGenerativeArtifacts, OfflineGenerativeOptions, OutputMode, RecognizerDescriptor,
        RecognizerEngine, SegmentationOptions,
    };
    use sherpa_onnx::LinearResampler;

    use super::*;

    #[test]
    #[ignore = "requires DICTATINGME_QWEN_MODEL_DIR and DICTATINGME_QWEN_TEST_WAV"]
    fn recognizes_configured_qwen_fixture() {
        let _evoke = std::env::var("DICTATINGME_EVOKE_MODEL_DIR")
            .ok()
            .map(|path| crate::models::EvokeModelEngine::new(&path, "你好".to_owned()).unwrap());
        let root = PathBuf::from(
            std::env::var("DICTATINGME_QWEN_MODEL_DIR")
                .expect("DICTATINGME_QWEN_MODEL_DIR is required"),
        );
        let wav = std::env::var("DICTATINGME_QWEN_TEST_WAV")
            .expect("DICTATINGME_QWEN_TEST_WAV is required");
        let spec = DictationModelSpec {
            id: "qwen-test".to_owned(),
            root,
            recognizer: RecognizerDescriptor::OfflineGenerative {
                engine: RecognizerEngine::SherpaOnnx,
                output_mode: OutputMode::Utterance,
                artifacts: OfflineGenerativeArtifacts {
                    frontend: "conv_frontend.onnx".to_owned(),
                    encoder: "encoder.int8.onnx".to_owned(),
                    decoder: "decoder.int8.onnx".to_owned(),
                    tokenizer: "tokenizer".to_owned(),
                },
                options: OfflineGenerativeOptions {
                    num_threads: 2,
                    max_total_length: 512,
                    max_new_tokens: 128,
                    segmentation: SegmentationOptions {
                        pre_roll_ms: 0,
                        minimum_speech_ms: 50,
                        trailing_silence_ms: 1000,
                        maximum_utterance_ms: 30_000,
                        speech_threshold: 0.0001,
                    },
                },
            },
        };
        let mut recognizer = OfflineGenerativeRecognizer::create(
            &spec,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
        .unwrap();
        let mut reader = hound::WavReader::open(wav).unwrap();
        assert_eq!(reader.spec().channels, 1);
        let sample_rate = reader.spec().sample_rate;
        let mut samples = reader
            .samples::<i16>()
            .map(|sample| f32::from(sample.unwrap()) / 32768.0)
            .collect::<Vec<_>>();
        if sample_rate != 16_000 {
            samples = LinearResampler::create(sample_rate as i32, 16_000)
                .unwrap()
                .resample(&samples, true);
        }
        for chunk in samples.chunks(1600) {
            recognizer.accept_samples(chunk).unwrap();
        }
        recognizer.accept_samples(&vec![0.0; 20_000]).unwrap();

        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            let updates = recognizer.poll_updates().unwrap();
            if let Some(update) = updates.last() {
                assert!(!update.full_text.trim().is_empty());
                assert!(
                    update
                        .full_text
                        .chars()
                        .any(|character| matches!(character, ',' | '.' | '，' | '。' | '?' | '？')),
                    "Qwen fixture transcript did not contain punctuation: {}",
                    update.full_text
                );
                println!("Qwen fixture transcript: {}", update.full_text);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Qwen fixture recognition timed out"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
