//! DictationModel Engine：本地 ONNX 千问 ASR（见 plan.md §3.1、§5）。
//!
//! 体积较大，仅由 State Machine 在收到 EvokeModel 的唤醒词检测事件后触发加载（唯一入口），
//! 运行于 Loading/Dictating 态，其余时间不存在；用完即卸载，不长期占用资源。

use crate::audio::AudioFrame;
use super::onnx_session::{ModelError, OnnxSession};

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
}

impl DictationModelEngine {
    /// 构造时不加载模型（`load_state` 为 `Unloaded`），仅记录模型路径。
    pub fn new(model_path: &str) -> Self {
        todo!()
    }

    pub fn load_state(&self) -> LoadState {
        todo!()
    }

    /// 异步加载模型（`State::Loading` 态触发，对应架构图连线 "②仅Evoke触发后加载"）。
    pub async fn load(&mut self) -> Result<(), ModelError> {
        todo!()
    }

    /// 卸载模型，释放资源（`State::Unloading` 态触发）。
    pub fn unload(&mut self) {
        todo!()
    }

    /// 送入一帧音频（含 Ring Buffer 补喂的历史帧，以及后续实时流），返回增量识别结果。
    /// 仅在 `load_state() == Loaded` 时应被调用。
    pub fn process_frame(&mut self, frame: &AudioFrame) -> Option<TranscriptionUpdate> {
        todo!()
    }
}
