//! Windows 模拟键盘输入实现（见 brainstrom/plan.md §8.1、§11 风险#2）。
//!
//! 基于 Windows `SendInput` API；已知风险：以管理员权限运行的窗口（UAC 提权）、
//! 部分反作弊/反外挂机制的游戏，模拟输入可能失效或被拦截。

use crate::text::{InjectorError, TextInjector};

/// `TextInjector` 的 Windows 实现。
pub struct WindowsTextInjector;

impl WindowsTextInjector {
    pub fn new() -> Self {
        todo!()
    }
}

impl Default for WindowsTextInjector {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInjector for WindowsTextInjector {
    fn type_text(&self, text: &str) -> Result<(), InjectorError> {
        todo!()
    }
}
