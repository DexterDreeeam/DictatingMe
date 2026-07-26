//! 麦克风采集（见 brainstrom/plan.md §5 Audio Capture）。
//! 基于 cpal（跨平台），不需要 platform/windows 下的专属实现。

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};

use super::device::{AudioDeviceInfo, AudioDeviceProvider, AudioError};

/// 单帧音频数据（PCM，单声道，采样率见 `sample_rate`）。
#[derive(Debug, Clone, PartialEq)]
pub struct AudioFrame {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
    /// 相对于本次 Loading/Dictating 会话开始的毫秒偏移
    pub timestamp_ms: u64,
}

/// 音频帧回调类型：EvokeModel / Audio Ring Buffer 均通过此回调消费音频。
pub type AudioFrameCallback = Box<dyn FnMut(AudioFrame) + Send>;

/// 麦克风采集能力的抽象（供未来更换后端或做单元测试 mock）。
pub trait AudioCapture {
    /// 使用指定设备开始持续采集（Listening 态启动，覆盖 Loading/Dictating 全程）。
    fn start(&mut self, device: &AudioDeviceInfo) -> Result<(), AudioError>;
    fn stop(&mut self);
    /// 注册音频帧回调；`Runtime` 会同时把帧路由给 EvokeModel 或 Ring Buffer，取决于当前状态。
    fn set_frame_callback(&mut self, callback: AudioFrameCallback);
    fn is_capturing(&self) -> bool;
}

/// 基于 cpal 的跨平台默认实现。
pub struct CpalAudioCapture {
    current_device: Option<AudioDeviceInfo>,
    host: cpal::Host,
    stream: Option<Stream>,
    callback: Arc<Mutex<Option<AudioFrameCallback>>>,
    worker: Option<JoinHandle<()>>,
}

impl CpalAudioCapture {
    pub fn new() -> Self {
        Self {
            current_device: None,
            host: cpal::default_host(),
            stream: None,
            callback: Arc::new(Mutex::new(None)),
            worker: None,
        }
    }

    fn resolve_device(&self, id: &str) -> Result<cpal::Device, AudioError> {
        let devices = self
            .host
            .input_devices()
            .map_err(|error| AudioError(format!("failed to enumerate input devices: {error}")))?;

        for device in devices {
            let device_id = device
                .id()
                .map_err(|error| AudioError(format!("failed to read input device id: {error}")))?;
            if device_id.to_string() == id {
                return Ok(device);
            }
        }

        Err(AudioError(format!(
            "audio input device is unavailable: {id}"
        )))
    }
}

impl Default for CpalAudioCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioCapture for CpalAudioCapture {
    fn start(&mut self, device: &AudioDeviceInfo) -> Result<(), AudioError> {
        self.stop();

        let cpal_device = self.resolve_device(&device.id)?;
        let supported = cpal_device.default_input_config().map_err(|error| {
            AudioError(format!(
                "input device '{}' has no usable default configuration: {error}",
                device.name
            ))
        })?;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        if config.channels == 0 {
            return Err(AudioError("input configuration has no channels".into()));
        }

        let (sender, receiver) = mpsc::sync_channel(8);
        let start = Instant::now();
        let stream = match sample_format {
            SampleFormat::I8 => build_input_stream::<i8>(&cpal_device, &config, sender, start),
            SampleFormat::I16 => build_input_stream::<i16>(&cpal_device, &config, sender, start),
            SampleFormat::I24 => {
                build_input_stream::<cpal::I24>(&cpal_device, &config, sender, start)
            }
            SampleFormat::I32 => build_input_stream::<i32>(&cpal_device, &config, sender, start),
            SampleFormat::I64 => build_input_stream::<i64>(&cpal_device, &config, sender, start),
            SampleFormat::U8 => build_input_stream::<u8>(&cpal_device, &config, sender, start),
            SampleFormat::U16 => build_input_stream::<u16>(&cpal_device, &config, sender, start),
            SampleFormat::U24 => {
                build_input_stream::<cpal::U24>(&cpal_device, &config, sender, start)
            }
            SampleFormat::U32 => build_input_stream::<u32>(&cpal_device, &config, sender, start),
            SampleFormat::U64 => build_input_stream::<u64>(&cpal_device, &config, sender, start),
            SampleFormat::F32 => build_input_stream::<f32>(&cpal_device, &config, sender, start),
            SampleFormat::F64 => build_input_stream::<f64>(&cpal_device, &config, sender, start),
            unsupported => Err(AudioError(format!(
                "unsupported input sample format: {unsupported}"
            ))),
        }?;

        let callback = Arc::clone(&self.callback);
        let worker = thread::Builder::new()
            .name("dictatingme-audio-frames".into())
            .spawn(move || {
                while let Ok(frame) = receiver.recv() {
                    let mut callback = callback.lock().unwrap_or_else(|lock| lock.into_inner());
                    if let Some(callback) = callback.as_mut() {
                        let _ = catch_unwind(AssertUnwindSafe(|| callback(frame)));
                    }
                }
            })
            .map_err(|error| AudioError(format!("failed to start audio worker: {error}")))?;

        if let Err(error) = stream.play() {
            drop(stream);
            let _ = worker.join();
            return Err(AudioError(format!("failed to start input stream: {error}")));
        }

        self.current_device = Some(device.clone());
        self.stream = Some(stream);
        self.worker = Some(worker);
        Ok(())
    }

    fn stop(&mut self) {
        // The stream owns the last sender used by its real-time callback. Dropping it
        // closes the channel after queued frames are drained by the worker.
        self.stream.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    fn set_frame_callback(&mut self, callback: AudioFrameCallback) {
        *self
            .callback
            .lock()
            .unwrap_or_else(|lock| lock.into_inner()) = Some(callback);
    }

    fn is_capturing(&self) -> bool {
        self.stream.is_some()
    }
}

impl AudioDeviceProvider for CpalAudioCapture {
    fn list_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioError> {
        let default_id = self
            .host
            .default_input_device()
            .and_then(|device| device.id().ok())
            .map(|id| id.to_string());
        let devices = self
            .host
            .input_devices()
            .map_err(|error| AudioError(format!("failed to enumerate input devices: {error}")))?;

        devices
            .map(|device| {
                let id = device
                    .id()
                    .map_err(|error| {
                        AudioError(format!("failed to read input device id: {error}"))
                    })?
                    .to_string();
                Ok(AudioDeviceInfo {
                    is_default: default_id.as_deref() == Some(id.as_str()),
                    name: device.to_string(),
                    id,
                })
            })
            .collect()
    }

    fn select_device(&mut self, device_id: &str) -> Result<(), AudioError> {
        let device = self.resolve_device(device_id)?;
        device.default_input_config().map_err(|error| {
            AudioError(format!(
                "selected device does not support audio input: {error}"
            ))
        })?;
        let default_id = self
            .host
            .default_input_device()
            .and_then(|device| device.id().ok())
            .map(|id| id.to_string());
        self.current_device = Some(AudioDeviceInfo {
            id: device_id.to_owned(),
            name: device.to_string(),
            is_default: default_id.as_deref() == Some(device_id),
        });
        Ok(())
    }

    fn current_device(&self) -> Option<AudioDeviceInfo> {
        self.current_device.clone()
    }
}

impl Drop for CpalAudioCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

fn build_input_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    sender: mpsc::SyncSender<AudioFrame>,
    start: Instant,
) -> Result<Stream, AudioError>
where
    T: SizedSample + Sample,
    i16: FromSample<T>,
{
    let channels = usize::from(config.channels);
    let sample_rate = config.sample_rate;
    device
        .build_input_stream::<T, _, _>(
            config.clone(),
            move |data, _| {
                let frame = AudioFrame {
                    samples: convert_to_mono(data, channels),
                    sample_rate,
                    timestamp_ms: start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                };
                let _ = sender.try_send(frame);
            },
            |error| {
                tracing::error!(%error, "CPAL input stream error");
            },
            None,
        )
        .map_err(|error| AudioError(format!("failed to create input stream: {error}")))
}

fn convert_to_mono<T>(samples: &[T], channels: usize) -> Vec<i16>
where
    T: Sample,
    i16: FromSample<T>,
{
    if channels == 0 {
        return Vec::new();
    }

    samples
        .chunks_exact(channels)
        .map(|frame| {
            let sum: i64 = frame
                .iter()
                .map(|sample| i64::from((*sample).to_sample::<i16>()))
                .sum();
            (sum / channels as i64) as i16
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::convert_to_mono;

    #[test]
    fn converts_interleaved_stereo_to_mono() {
        assert_eq!(
            convert_to_mono(&[i16::MAX, i16::MIN, 1_000, 3_000], 2),
            vec![0, 2_000]
        );
    }

    #[test]
    fn converts_float_samples_and_ignores_incomplete_frames() {
        assert_eq!(
            convert_to_mono(&[-1.0_f32, 1.0, 0.5, 0.5, 1.0], 2),
            vec![0, 16_384]
        );
    }

    #[test]
    fn zero_channels_produces_no_samples() {
        assert!(convert_to_mono(&[1_i16, 2], 0).is_empty());
    }
}
