//! 麦克风采集（见 brainstrom/plan.md §5 Audio Capture）。
//! 基于 cpal（跨平台），不需要 platform/windows 下的专属实现。

use super::device::{AudioDeviceInfo, AudioError};

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
}

impl CpalAudioCapture {
    pub fn new() -> Self {
        todo!()
    }
}

impl Default for CpalAudioCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioCapture for CpalAudioCapture {
    fn start(&mut self, device: &AudioDeviceInfo) -> Result<(), AudioError> {
        todo!()
    }

    fn stop(&mut self) {
        todo!()
    }

    fn set_frame_callback(&mut self, callback: AudioFrameCallback) {
        todo!()
    }

    fn is_capturing(&self) -> bool {
        todo!()
    }
}
