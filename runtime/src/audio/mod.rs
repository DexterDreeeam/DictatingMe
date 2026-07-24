//! Audio 模块：麦克风采集、设备管理、Loading 阶段录音缓冲（见 brainstrom/plan.md §5）。

pub mod device;
pub mod capture;
pub mod ring_buffer;

pub use device::{AudioDeviceInfo, AudioDeviceProvider, AudioError};
pub use capture::{AudioCapture, AudioFrame, AudioFrameCallback, CpalAudioCapture};
pub use ring_buffer::AudioRingBuffer;
