//! Tray Manager（见 brainstrom/plan.md §3.3、§5、§7）。
//!
//! 创建/持有系统托盘图标（固定不变，不随状态改变）；
//! 左键点击 = 向 State Machine 发出"打开 MainWindow 请求"；右键菜单提供"退出"。
//! Tray、Runtime、SystemTray 三者共生共灭（见 plan.md §3.3 关键生命周期事实）。

use std::sync::{Arc, Mutex};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::AppHandle;

/// 托盘操作错误。
#[derive(Debug, Clone, PartialEq)]
pub struct TrayError(pub String);

/// 左键点击回调：等价于向 State Machine 发出 `StateEvent::OpenMainWindowRequested`。
pub type OpenMainWindowCallback = Box<dyn Fn() + Send>;
/// 右键菜单"退出"回调：终止整个 Runtime 进程。
pub type ExitCallback = Box<dyn Fn() + Send>;

/// Tray Manager 接口。
pub trait TrayManager {
    /// 创建托盘图标（图标固定，不随状态变化，见 plan.md §7）。
    fn create(
        &mut self,
        on_open_main_window: OpenMainWindowCallback,
        on_exit: ExitCallback,
    ) -> Result<(), TrayError>;

    fn destroy(&mut self);
}

struct TauriTrayManager {
    app: AppHandle,
    tray: Option<TrayIcon>,
}

impl TauriTrayManager {
    fn new(app: AppHandle) -> Self {
        Self { app, tray: None }
    }
}

impl TrayManager for TauriTrayManager {
    fn create(
        &mut self,
        on_open_main_window: OpenMainWindowCallback,
        on_exit: ExitCallback,
    ) -> Result<(), TrayError> {
        tracing::info!("tray creation begin");
        if self.tray.is_some() {
            tracing::error!("tray creation rejected: already created");
            return Err(TrayError("tray icon has already been created".to_owned()));
        }

        let quit = MenuItem::with_id(&self.app, "dictatingme-quit", "Quit", true, None::<&str>)
            .map_err(|error| TrayError(format!("failed to create tray quit item: {error}")))?;
        let menu = Menu::with_items(&self.app, &[&quit])
            .map_err(|error| TrayError(format!("failed to create tray menu: {error}")))?;
        let icon = self
            .app
            .default_window_icon()
            .cloned()
            .ok_or_else(|| TrayError("application has no default tray icon".to_owned()))?;

        let open_callback = Arc::new(Mutex::new(on_open_main_window));
        let exit_callback = Arc::new(Mutex::new(on_exit));
        let quit_id = quit.id().clone();
        let tray = TrayIconBuilder::with_id("dictatingme-runtime")
            .icon(icon)
            .tooltip("DictatingMe")
            .menu(&menu)
            .show_menu_on_left_click(false)
            .on_tray_icon_event(move |_tray, event| {
                if matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }
                ) {
                    tracing::info!("tray open requested");
                    match open_callback.lock() {
                        Ok(callback) => callback(),
                        Err(_) => tracing::error!(
                            "tray open callback failed: callback mutex was poisoned"
                        ),
                    }
                }
            })
            .on_menu_event(move |_app, event| {
                if event.id() == &quit_id {
                    tracing::info!("tray quit requested");
                    match exit_callback.lock() {
                        Ok(callback) => callback(),
                        Err(_) => tracing::error!(
                            "tray exit callback failed: callback mutex was poisoned"
                        ),
                    }
                }
            })
            .build(&self.app)
            .map_err(|error| TrayError(format!("failed to build tray icon: {error}")))?;
        self.tray = Some(tray);
        tracing::info!("tray creation success");
        Ok(())
    }

    fn destroy(&mut self) {
        if let Some(tray) = self.tray.take() {
            tracing::info!("tray destruction begin");
            let id = tray.id().clone();
            drop(self.app.remove_tray_by_id(&id));
            drop(tray);
            tracing::info!("tray destruction success");
        } else {
            tracing::debug!("tray destruction skipped: tray not present");
        }
    }
}

pub(crate) fn create_tauri_tray_manager(app: AppHandle) -> Box<dyn TrayManager + Send> {
    Box::new(TauriTrayManager::new(app))
}
