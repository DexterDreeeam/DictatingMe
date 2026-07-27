//! Text Injector：把增量文本粘贴到当前焦点（见 brainstrom/plan.md §8.1）。
//!
//! Windows 实现通过 `SendInput` 直接发送 Unicode 键盘事件，不读写系统剪贴板；
//! 平台相关细节见 `crate::platform::windows::send_input`。

/// 注入过程中可能发生的错误。
#[derive(Debug, Clone, PartialEq)]
pub struct InjectorError(pub String);

/// 文本注入器接口。
pub trait TextInjector {
    /// 把新增文字粘贴到当前操作系统焦点控件。
    /// 无条件发送到当前焦点，不做敏感控件（如密码框）识别与保护（v1 明确排除，见 plan.md §10）。
    fn type_text(&self, text: &str) -> Result<(), InjectorError>;
}
