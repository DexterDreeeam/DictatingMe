//! Windows Unicode 键盘注入实现（见 brainstrom/plan.md §8.1、§11 风险#2）。
//!
//! 通过 `SendInput` 发送 `KEYEVENTF_UNICODE`，不读写系统剪贴板。跨完整性级别的输入
//! 仍可能被 Windows UIPI 拦截，因此 DM 以管理员权限运行以覆盖普通和管理员窗口。

use crate::text::{InjectorError, TextInjector};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    VIRTUAL_KEY,
};

pub(crate) const INPUT_MARKER: usize = 0x444D_494E;

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

        let inputs = unicode_inputs(text);
        let inserted = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) } as usize;
        if inserted != inputs.len() {
            return Err(InjectorError(format!(
                "SendInput inserted {inserted} of {} Unicode keyboard events; input may be blocked by UIPI: {}",
                inputs.len(),
                std::io::Error::last_os_error()
            )));
        }
        tracing::debug!(
            text_length = text.chars().count(),
            events = inserted,
            "Unicode keyboard input injected"
        );

        Ok(())
    }
}

fn unicode_inputs(text: &str) -> Vec<INPUT> {
    text.encode_utf16()
        .flat_map(|code_unit| {
            [
                unicode_input(code_unit, false),
                unicode_input(code_unit, true),
            ]
        })
        .collect()
}

fn unicode_input(code_unit: u16, key_up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: code_unit,
                dwFlags: if key_up {
                    KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
                } else {
                    KEYEVENTF_UNICODE
                },
                time: 0,
                dwExtraInfo: INPUT_MARKER,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{unicode_inputs, INPUT_MARKER};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT_KEYBOARD, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY,
    };

    #[test]
    fn constructs_unicode_key_pairs_with_marker() {
        let code_units = "A你😀".encode_utf16().collect::<Vec<_>>();
        let inputs = unicode_inputs("A你😀");
        assert_eq!(inputs.len(), code_units.len() * 2);
        assert!(inputs.iter().all(|input| input.r#type == INPUT_KEYBOARD));

        for (pair, expected_code_unit) in inputs.chunks_exact(2).zip(code_units) {
            let key_down = unsafe { pair[0].Anonymous.ki };
            let key_up = unsafe { pair[1].Anonymous.ki };
            assert_eq!(key_down.wVk, VIRTUAL_KEY(0));
            assert_eq!(key_up.wVk, VIRTUAL_KEY(0));
            assert_eq!(key_down.wScan, expected_code_unit);
            assert_eq!(key_up.wScan, expected_code_unit);
            assert_eq!(key_down.dwFlags, KEYEVENTF_UNICODE);
            assert_eq!(key_up.dwFlags, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP);
        }
        assert!(inputs
            .iter()
            .all(|input| unsafe { input.Anonymous.ki }.dwExtraInfo == INPUT_MARKER));
    }

    #[test]
    fn empty_text_produces_no_inputs() {
        assert!(unicode_inputs("").is_empty());
    }
}
