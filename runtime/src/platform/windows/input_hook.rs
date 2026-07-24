//! Windows 全局键鼠监听实现（见 brainstrom/plan.md §8.3、§11 风险#3）。
//!
//! 候选方案：`SetWindowsHookEx`（低级钩子）或 Raw Input；
//! 需评估杀毒软件/安全软件的误报风险（见 plan.md §11 已知风险）。

use crate::input_monitor::{DismissCallback, GlobalInputMonitor, MonitorError};

/// `GlobalInputMonitor` 的 Windows 实现。
pub struct WindowsInputMonitor {
    monitoring: bool,
}

impl WindowsInputMonitor {
    pub fn new() -> Self {
        todo!()
    }
}

impl Default for WindowsInputMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalInputMonitor for WindowsInputMonitor {
    fn start(&mut self, callback: DismissCallback) -> Result<(), MonitorError> {
        todo!()
    }

    fn stop(&mut self) {
        todo!()
    }

    fn is_monitoring(&self) -> bool {
        todo!()
    }
}
