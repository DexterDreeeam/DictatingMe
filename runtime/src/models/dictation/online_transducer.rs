use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};

use crate::storage::{OutputMode, RecognizerDescriptor};

use super::super::onnx_session::{path_string, require_file, validate_model_directory, ModelError};
use super::{assemble_text, DictationModelSpec, DictationRecognizer, TranscriptionUpdate};

pub(super) struct OnlineTransducerRecognizer {
    stream: OnlineStream,
    recognizer: OnlineRecognizer,
    committed_segments: String,
    last_emitted: String,
}

impl OnlineTransducerRecognizer {
    pub(super) fn create(spec: &DictationModelSpec) -> Result<Self, ModelError> {
        let RecognizerDescriptor::OnlineTransducer {
            output_mode,
            artifacts,
            options,
            ..
        } = &spec.recognizer
        else {
            return Err(ModelError(format!(
                "model '{}' is not an online transducer",
                spec.id
            )));
        };
        if *output_mode != OutputMode::Streaming {
            return Err(ModelError(format!(
                "online transducer '{}' must use streaming output",
                spec.id
            )));
        }
        let directory = validate_model_directory(
            spec.root
                .to_str()
                .ok_or_else(|| ModelError("dictation model path is not UTF-8".to_owned()))?,
        )?;
        let encoder = require_file(&directory, &artifacts.encoder)?;
        let decoder = require_file(&directory, &artifacts.decoder)?;
        let joiner = require_file(&directory, &artifacts.joiner)?;
        let tokens = require_file(&directory, &artifacts.tokens)?;

        let mut config = OnlineRecognizerConfig::default();
        config.model_config.transducer.encoder = Some(path_string(&encoder)?);
        config.model_config.transducer.decoder = Some(path_string(&decoder)?);
        config.model_config.transducer.joiner = Some(path_string(&joiner)?);
        config.model_config.tokens = Some(path_string(&tokens)?);
        config.model_config.num_threads = options.num_threads;
        config.decoding_method = Some(options.decoding_method.clone());
        config.enable_endpoint = true;
        config.rule1_min_trailing_silence = options.endpoint.rule1_silence_ms as f32 / 1000.0;
        config.rule2_min_trailing_silence = options.endpoint.rule2_silence_ms as f32 / 1000.0;
        config.rule3_min_utterance_length = options.endpoint.max_utterance_ms as f32 / 1000.0;

        let recognizer = OnlineRecognizer::create(&config).ok_or_else(|| {
            ModelError(format!(
                "failed to create online transducer recognizer for '{}'",
                spec.id
            ))
        })?;
        let stream = recognizer.create_stream();
        Ok(Self {
            stream,
            recognizer,
            committed_segments: String::new(),
            last_emitted: String::new(),
        })
    }
}

impl DictationRecognizer for OnlineTransducerRecognizer {
    fn output_mode(&self) -> OutputMode {
        OutputMode::Streaming
    }

    fn accept_samples(&mut self, samples: &[f32]) -> Result<Vec<TranscriptionUpdate>, ModelError> {
        self.stream.accept_waveform(16_000, samples);
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }

        let Some(result) = self.recognizer.get_result(&self.stream) else {
            return Ok(Vec::new());
        };
        let endpoint = self.recognizer.is_endpoint(&self.stream);
        let full_text = assemble_text(&self.committed_segments, result.text.trim());
        let is_final = endpoint || result.is_final;

        if endpoint {
            self.committed_segments.clone_from(&full_text);
            self.recognizer.reset(&self.stream);
        }
        if full_text.is_empty() || (full_text == self.last_emitted && !is_final) {
            return Ok(Vec::new());
        }
        self.last_emitted.clone_from(&full_text);
        Ok(vec![TranscriptionUpdate {
            full_text,
            is_final,
        }])
    }

    fn poll_updates(&mut self) -> Result<Vec<TranscriptionUpdate>, ModelError> {
        Ok(Vec::new())
    }

    fn discard_pending(&mut self) {
        self.recognizer.reset(&self.stream);
        self.committed_segments.clear();
        self.last_emitted.clear();
    }
}
