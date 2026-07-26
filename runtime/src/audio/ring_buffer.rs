//! Audio Ring Buffer：Loading 阶段的录音缓冲区（见 plan.md §8.2 无缝衔接）。
//!
//! 仅在 `State::Loading` 起启用（由 State Machine 的 `TransitionEffect::StartAudioBuffering`
//! 触发），保证从"唤醒词被识别的那一刻"开始的音频都不丢失，进入 `Dictating` 时无缝喂入。

use super::capture::AudioFrame;
use std::collections::VecDeque;

/// Loading 阶段的录音缓冲区。
pub struct AudioRingBuffer {
    /// 缓冲区最大时长（毫秒），超过后应丢弃最旧数据（具体策略待实现）
    capacity_ms: u64,
    frames: VecDeque<(AudioFrame, u128)>,
    buffered_duration_ns: u128,
}

impl AudioRingBuffer {
    pub fn new(capacity_ms: u64) -> Self {
        Self {
            capacity_ms,
            frames: VecDeque::new(),
            buffered_duration_ns: 0,
        }
    }

    /// 写入一帧音频（仅 Loading 态调用，对应架构图连线 "③唤醒后才缓冲"）。
    pub fn push(&mut self, frame: AudioFrame) {
        if self.capacity_ms == 0 || frame.sample_rate == 0 || frame.samples.is_empty() {
            return;
        }

        let duration_ns = frame_duration_ns(&frame);
        self.buffered_duration_ns = self.buffered_duration_ns.saturating_add(duration_ns);
        self.frames.push_back((frame, duration_ns));

        let capacity_ns = u128::from(self.capacity_ms) * 1_000_000;
        while self.buffered_duration_ns > capacity_ns {
            let Some((_, removed_duration_ns)) = self.frames.pop_front() else {
                self.buffered_duration_ns = 0;
                break;
            };
            self.buffered_duration_ns = self
                .buffered_duration_ns
                .saturating_sub(removed_duration_ns);
        }
    }

    /// 进入 Dictating 时调用：取出全部缓冲内容作为第一批数据，随后清空。
    pub fn drain(&mut self) -> Vec<AudioFrame> {
        self.buffered_duration_ns = 0;
        self.frames.drain(..).map(|(frame, _)| frame).collect()
    }

    /// Unloading 时调用：丢弃所有未处理的缓冲内容。
    pub fn clear(&mut self) {
        self.frames.clear();
        self.buffered_duration_ns = 0;
    }

    pub fn buffered_duration_ms(&self) -> u64 {
        let rounded_up = self.buffered_duration_ns.saturating_add(999_999) / 1_000_000;
        u64::try_from(rounded_up).unwrap_or(u64::MAX)
    }
}

fn frame_duration_ns(frame: &AudioFrame) -> u128 {
    if frame.samples.is_empty() {
        return 0;
    }
    let numerator = (frame.samples.len() as u128).saturating_mul(1_000_000_000);
    numerator.saturating_add(u128::from(frame.sample_rate) - 1) / u128::from(frame.sample_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(value: i16, samples: usize, sample_rate: u32, timestamp_ms: u64) -> AudioFrame {
        AudioFrame {
            samples: vec![value; samples],
            sample_rate,
            timestamp_ms,
        }
    }

    #[test]
    fn evicts_oldest_frames_by_audio_duration_and_drains_in_order() {
        let mut buffer = AudioRingBuffer::new(200);
        buffer.push(frame(1, 100, 1_000, 100));
        buffer.push(frame(2, 100, 1_000, 0));
        buffer.push(frame(3, 100, 1_000, 50));

        assert_eq!(buffer.buffered_duration_ms(), 200);
        let drained = buffer.drain();
        assert_eq!(
            drained
                .iter()
                .map(|audio| audio.samples[0])
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(buffer.buffered_duration_ms(), 0);
    }

    #[test]
    fn handles_mixed_rates_sub_millisecond_frames_and_invalid_rates() {
        let mut buffer = AudioRingBuffer::new(2);
        buffer.push(frame(1, 1, 1_000, u64::MAX));
        buffer.push(frame(2, 24, 48_000, 0));
        buffer.push(frame(3, 50, 0, 1));

        assert_eq!(buffer.buffered_duration_ms(), 2);
        assert_eq!(buffer.drain().len(), 2);
    }

    #[test]
    fn zero_capacity_and_clear_leave_the_buffer_empty() {
        let mut zero = AudioRingBuffer::new(0);
        zero.push(frame(1, 100, 1_000, 0));
        assert!(zero.drain().is_empty());

        let mut buffer = AudioRingBuffer::new(100);
        buffer.push(frame(1, 100, 1_000, 0));
        buffer.clear();
        assert!(buffer.drain().is_empty());
    }
}
