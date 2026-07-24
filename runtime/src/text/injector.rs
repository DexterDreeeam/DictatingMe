//! Text Injector：模拟键盘输入接口（见 brainstrom/plan.md §8.1）。
//!
//! 不使用剪贴板 copy-paste；平台相关的具体实现见 `crate::platform::windows::send_input`。

/// 注入过程中可能发生的错误。
#[derive(Debug, Clone, PartialEq)]
pub struct InjectorError(pub String);

/// 模拟键盘输入的注入器接口（跨平台 trait，Windows 优先实现 SendInput）。
pub trait TextInjector {
    /// 把新增文字模拟打字到当前操作系统焦点控件。
    /// 无条件发送到当前焦点，不做敏感控件（如密码框）识别与保护（v1 明确排除，见 plan.md §10）。
    fn type_text(&self, text: &str) -> Result<(), InjectorError>;
}
