//! Windows 剪贴板粘贴实现（见 brainstrom/plan.md §8.1、§11 风险#2）。
//!
//! 先写入系统剪贴板，再通过 `SendInput` 发送 Ctrl+V。跨完整性级别的输入仍可能
//! 被 Windows UIPI 拦截，例如普通权限的 DM 无法可靠控制管理员权限窗口。

use crate::text::{InjectorError, TextInjector};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL,
};

pub(crate) const INPUT_MARKER: usize = 0x444D_494E;
const VK_V: VIRTUAL_KEY = VIRTUAL_KEY(0x56);

/// `TextInjector` 的 Windows 实现。
pub struct WindowsTextInjector;

impl WindowsTextInjector {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WindowsTextInjector {
    fn default() -> Self {
        Self::new()
    }
}

impl TextInjector for WindowsTextInjector {
    fn type_text(&self, text: &str) -> Result<(), InjectorError> {
        if text.is_empty() {
            return Ok(());
        }

        let mut clipboard = arboard::Clipboard::new()
            .map_err(|error| InjectorError(format!("failed to open system clipboard: {error}")))?;
        clipboard.set_text(text.to_owned()).map_err(|error| {
            InjectorError(format!(
                "failed to write dictated text to clipboard: {error}"
            ))
        })?;
        drop(clipboard);
        tracing::debug!(
            text_length = text.chars().count(),
            "clipboard text prepared for paste"
        );

        let inputs = paste_inputs();
        let inserted = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) } as usize;
        if inserted != inputs.len() {
            return Err(InjectorError(format!(
                "SendInput inserted {inserted} of {} Ctrl+V events; paste may be blocked by UIPI: {}",
                inputs.len(),
                std::io::Error::last_os_error()
            )));
        }
        tracing::debug!(events = inserted, "Ctrl+V input injected");

        Ok(())
    }
}

fn paste_inputs() -> [INPUT; 4] {
    [
        virtual_key_input(VK_CONTROL, false),
        virtual_key_input(VK_V, false),
        virtual_key_input(VK_V, true),
        virtual_key_input(VK_CONTROL, true),
    ]
}

fn virtual_key_input(key: VIRTUAL_KEY, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: if key_up {
                    KEYEVENTF_KEYUP
                } else {
                    Default::default()
                },
                time: 0,
                dwExtraInfo: INPUT_MARKER,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{paste_inputs, INPUT_MARKER, VK_V};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT_KEYBOARD, KEYEVENTF_KEYUP, VK_CONTROL,
    };

    #[test]
    fn constructs_ctrl_v_key_pairs_with_marker() {
        let inputs = paste_inputs();
        assert_eq!(inputs.len(), 4);
        assert!(inputs.iter().all(|input| input.r#type == INPUT_KEYBOARD));

        let control_down = unsafe { inputs[0].Anonymous.ki };
        let v_down = unsafe { inputs[1].Anonymous.ki };
        let v_up = unsafe { inputs[2].Anonymous.ki };
        let control_up = unsafe { inputs[3].Anonymous.ki };
        assert_eq!(control_down.wVk, VK_CONTROL);
        assert_eq!(v_down.wVk, VK_V);
        assert_eq!(v_up.wVk, VK_V);
        assert_eq!(control_up.wVk, VK_CONTROL);
        assert_eq!(control_down.dwFlags, Default::default());
        assert_eq!(v_down.dwFlags, Default::default());
        assert_eq!(v_up.dwFlags, KEYEVENTF_KEYUP);
        assert_eq!(control_up.dwFlags, KEYEVENTF_KEYUP);
        assert!(inputs
            .iter()
            .all(|input| unsafe { input.Anonymous.ki }.dwExtraInfo == INPUT_MARKER));
    }
}
