//! EvokeModel Engine：ONNX 唤醒词模型（见 plan.md §3.1、§5）。
//!
//! 目标 <20MB，常驻内存，长期运行；自训练小型关键词检测模型（类 openWakeWord 思路）。
//! 仅在 `State::Listening` 态激活；检测到唤醒词是触发 `DictationModelEngine` 加载的唯一入口。

use crate::audio::AudioFrame;
use super::onnx_session::{ModelError, OnnxSession};

/// 唤醒词检测结果。
#[derive(Debug, Clone, PartialEq)]
pub struct WakeWordDetection {
    /// 置信度 0.0-1.0，与 `sensitivity` 阈值比较后决定是否上报
    pub confidence: f32,
    pub detected_at_ms: u64,
}

/// EvokeModel Engine。
pub struct EvokeModelEngine {
    session: OnnxSession,
    /// 敏感度（对应 EvokeWord 设置页滑杆，0.0=不易误触发 ~ 1.0=更容易唤醒）
    sensitivity: f32,
    /// 当前生效的唤醒词（同一时间只能有 1 个生效，见 plan.md §6.3）
    active_word: String,
}

impl EvokeModelEngine {
    /// 常驻加载：Runtime 启动时调用一次，此后长期驻留内存。
    pub fn new(model_path: &str, active_word: String) -> Result<Self, ModelError> {
        todo!()
    }

    pub fn active_word(&self) -> &str {
        todo!()
    }

    /// 切换生效唤醒词（EvokeWord 设置页），可能需要重新加载/微调模型权重。
    pub fn set_active_word(&mut self, word: String) -> Result<(), ModelError> {
        todo!()
    }

    pub fn sensitivity(&self) -> f32 {
        todo!()
    }

    /// 设置敏感度（EvokeWord 设置页滑杆）。
    pub fn set_sensitivity(&mut self, value: f32) {
        todo!()
    }

    /// 送入一帧音频，返回是否检测到唤醒词（仅在 `State::Listening` 态被调用）。
    pub fn process_frame(&mut self, frame: &AudioFrame) -> Option<WakeWordDetection> {
        todo!()
    }

    /// 用用户声音样本对唤醒词模型做个性化微调（Future Work，占位接口，见 plan.md §10）。
    pub fn fine_tune_with_voice_samples(&mut self, samples: Vec<AudioFrame>) -> Result<(), ModelError> {
        todo!("Future Work：预留接口，v1 不实现")
    }
}
