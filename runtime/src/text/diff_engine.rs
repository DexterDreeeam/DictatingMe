//! Text Diff Engine（见 brainstrom/plan.md §8.1）。
//!
//! 对比"上次完整识别文本"与"当前完整识别文本"，只取新增后缀，不做修正回退——
//! 如果模型后续修正了之前的识别结果，DM 不会退格重打，只会继续追加新内容。

/// 文本增量对比引擎，每次听写会话（一次 Loading→Unloading 周期）持有一个实例。
pub struct TextDiffEngine {
    last_full_text: String,
}

impl TextDiffEngine {
    pub fn new() -> Self {
        todo!()
    }

    /// 输入当前完整识别文本，返回需要新增打字的后缀（可能为空字符串）。
    /// 不做"选中删除重打"，只做纯后缀追加。
    pub fn compute_suffix(&mut self, current_full_text: &str) -> String {
        todo!()
    }

    /// 一次听写结束后重置内部状态（`State::Unloading` 阶段调用）。
    pub fn reset(&mut self) {
        todo!()
    }
}

impl Default for TextDiffEngine {
    fn default() -> Self {
        Self::new()
    }
}
