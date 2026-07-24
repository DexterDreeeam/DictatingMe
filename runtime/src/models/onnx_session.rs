//! 通用 ONNX Runtime 会话封装，EvokeModel / DictationModel 共用（见 plan.md §2 技术栈）。

/// 模型加载/推理相关错误。
#[derive(Debug, Clone, PartialEq)]
pub struct ModelError(pub String);

/// 对 `ort` (ONNX Runtime) 会话的薄封装。
pub struct OnnxSession {
    model_path: String,
}

impl OnnxSession {
    /// 同步加载（EvokeModel 常驻加载路径）。
    pub fn load(model_path: &str) -> Result<Self, ModelError> {
        todo!()
    }

    /// 异步加载（DictationModel 在 Loading 态走此路径，避免阻塞主循环）。
    pub async fn load_async(model_path: &str) -> Result<Self, ModelError> {
        todo!()
    }

    pub fn model_path(&self) -> &str {
        todo!()
    }

    /// 显式卸载，释放底层会话资源（DictationModel 在 Unloading 态调用）。
    pub fn unload(self) {
        todo!()
    }
}
