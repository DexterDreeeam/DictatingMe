//! Windows 全局键鼠监听实现（见 brainstrom/plan.md §8.3、§11 风险#3）。
//!
//! 候选方案：`SetWindowsHookEx`（低级钩子）或 Raw Input；
//! 需评估杀毒软件/安全软件的误报风险（见 plan.md §11 已知风险）。

use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
    UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_QUIT,
    WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_XBUTTONDOWN,
};

use crate::input_monitor::{DismissCallback, GlobalInputMonitor, InputEventKind, MonitorError};

use super::send_input::INPUT_MARKER;

thread_local! {
    static CALLBACK: RefCell<Option<DismissCallback>> = RefCell::new(None);
}

/// `GlobalInputMonitor` 的 Windows 实现。
pub struct WindowsInputMonitor {
    monitoring: bool,
    thread_id: Option<u32>,
    worker: Option<JoinHandle<()>>,
}

impl WindowsInputMonitor {
    pub fn new() -> Self {
        Self {
            monitoring: false,
            thread_id: None,
            worker: None,
        }
    }
}

impl Default for WindowsInputMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl GlobalInputMonitor for WindowsInputMonitor {
    fn start(&mut self, callback: DismissCallback) -> Result<(), MonitorError> {
        self.stop();

        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("dictatingme-input-monitor".into())
            .spawn(move || hook_thread(callback, ready_sender))
            .map_err(|error| MonitorError(format!("failed to start input monitor: {error}")))?;

        match ready_receiver.recv() {
            Ok(Ok(thread_id)) => {
                self.monitoring = true;
                self.thread_id = Some(thread_id);
                self.worker = Some(worker);
                Ok(())
            }
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                let _ = worker.join();
                Err(MonitorError(
                    "input monitor thread stopped during startup".into(),
                ))
            }
        }
    }

    fn stop(&mut self) {
        if let Some(thread_id) = self.thread_id.take() {
            let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.monitoring = false;
    }

    fn is_monitoring(&self) -> bool {
        self.monitoring
    }
}

impl Drop for WindowsInputMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

fn hook_thread(callback: DismissCallback, ready: mpsc::SyncSender<Result<u32, MonitorError>>) {
    CALLBACK.with(|slot| *slot.borrow_mut() = Some(callback));

    let mut message = MSG::default();
    unsafe {
        // Force creation of the thread message queue before start() is allowed
        // to post WM_QUIT to it.
        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
    }

    let keyboard_hook =
        match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_proc), None, 0) } {
            Ok(hook) => hook,
            Err(error) => {
                CALLBACK.with(|slot| slot.borrow_mut().take());
                let _ = ready.send(Err(MonitorError(format!(
                    "failed to install keyboard hook: {error}"
                ))));
                return;
            }
        };

    let mouse_hook = match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), None, 0) }
    {
        Ok(hook) => hook,
        Err(error) => {
            let _ = unsafe { UnhookWindowsHookEx(keyboard_hook) };
            CALLBACK.with(|slot| slot.borrow_mut().take());
            let _ = ready.send(Err(MonitorError(format!(
                "failed to install mouse hook: {error}"
            ))));
            return;
        }
    };

    let thread_id = unsafe { GetCurrentThreadId() };
    if ready.send(Ok(thread_id)).is_ok() {
        loop {
            let result = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
            if result <= 0 {
                break;
            }
        }
    }

    let _ = unsafe { UnhookWindowsHookEx(mouse_hook) };
    let _ = unsafe { UnhookWindowsHookEx(keyboard_hook) };
    CALLBACK.with(|slot| slot.borrow_mut().take());
}

unsafe extern "system" fn keyboard_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN) {
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        if event.dwExtraInfo != INPUT_MARKER {
            notify_once(InputEventKind::Keyboard);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        if let Some(kind) = mouse_event_kind(wparam.0 as u32) {
            let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            if event.dwExtraInfo != INPUT_MARKER {
                notify_once(kind);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn mouse_event_kind(message: u32) -> Option<InputEventKind> {
    match message {
        WM_LBUTTONDOWN => Some(InputEventKind::MouseLeft),
        WM_RBUTTONDOWN => Some(InputEventKind::MouseRight),
        WM_MBUTTONDOWN => Some(InputEventKind::MouseMiddle),
        WM_XBUTTONDOWN => Some(InputEventKind::MouseSide),
        _ => None,
    }
}

fn notify_once(kind: InputEventKind) {
    let callback = CALLBACK.with(|slot| slot.borrow_mut().take());
    if let Some(callback) = callback {
        let _ = catch_unwind(AssertUnwindSafe(|| callback(kind)));
    }
}

#[cfg(test)]
mod tests {
    use super::mouse_event_kind;
    use crate::input_monitor::InputEventKind;
    use windows::Win32::UI::WindowsAndMessaging::{
        WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_XBUTTONDOWN,
    };

    #[test]
    fn classifies_only_mouse_button_down_events() {
        assert_eq!(
            mouse_event_kind(WM_LBUTTONDOWN),
            Some(InputEventKind::MouseLeft)
        );
        assert_eq!(
            mouse_event_kind(WM_RBUTTONDOWN),
            Some(InputEventKind::MouseRight)
        );
        assert_eq!(
            mouse_event_kind(WM_XBUTTONDOWN),
            Some(InputEventKind::MouseSide)
        );
        assert_eq!(mouse_event_kind(WM_MOUSEMOVE), None);
    }
}
