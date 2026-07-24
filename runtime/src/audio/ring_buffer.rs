//! Audio Ring Buffer：Loading 阶段的录音缓冲区（见 plan.md §8.2 无缝衔接）。
//!
//! 仅在 `State::Loading` 起启用（由 State Machine 的 `TransitionEffect::StartAudioBuffering`
//! 触发），保证从"唤醒词被识别的那一刻"开始的音频都不丢失，进入 `Dictating` 时无缝喂入。

use super::capture::AudioFrame;

/// Loading 阶段的录音缓冲区。
pub struct AudioRingBuffer {
    /// 缓冲区最大时长（毫秒），超过后应丢弃最旧数据（具体策略待实现）
    capacity_ms: u64,
}

impl AudioRingBuffer {
    pub fn new(capacity_ms: u64) -> Self {
        todo!()
    }

    /// 写入一帧音频（仅 Loading 态调用，对应架构图连线 "③唤醒后才缓冲"）。
    pub fn push(&mut self, frame: AudioFrame) {
        todo!()
    }

    /// 进入 Dictating 时调用：取出全部缓冲内容作为第一批数据，随后清空。
    pub fn drain(&mut self) -> Vec<AudioFrame> {
        todo!()
    }

    /// Unloading 时调用：丢弃所有未处理的缓冲内容。
    pub fn clear(&mut self) {
        todo!()
    }

    pub fn buffered_duration_ms(&self) -> u64 {
        todo!()
    }
}
