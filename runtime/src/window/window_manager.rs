//! Window Manager（见 brainstrom/plan.md §3.2、§5）。
//!
//! 根据 State Machine 广播的状态互斥显示 MainWindow 或 HudWindow：
//! 同一时刻有且只有一个可见。`Configure` 显示 MainWindow，其余四态显示 HudWindow。

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, Position, Size, WebviewWindow};

use crate::state_machine::{HudLight, WindowKind};

use super::compute_hud_position;

/// 窗口操作错误。
#[derive(Debug, Clone, PartialEq)]
pub struct WindowError(pub String);

/// Window Manager 接口。
pub trait WindowManager {
    /// 依据 State Machine 给出的目标窗口做互斥切换（显示其一、隐藏另一个）。
    fn apply_visible_window(&mut self, visible: WindowKind) -> Result<(), WindowError>;

    fn show_main_window(&mut self) -> Result<(), WindowError>;
    fn hide_main_window(&mut self) -> Result<(), WindowError>;

    fn show_hud_window(&mut self) -> Result<(), WindowError>;
    fn hide_hud_window(&mut self) -> Result<(), WindowError>;

    /// 更新 HUD 灯光颜色（Listening=黄，Loading/Dictating=绿，Unloading=灭）。
    fn set_hud_light(&mut self, light: HudLight) -> Result<(), WindowError>;
}

struct TauriWindowManager {
    app: AppHandle,
    main: WebviewWindow,
    hud: WebviewWindow,
}

impl TauriWindowManager {
    fn new(app: AppHandle) -> Result<Self, WindowError> {
        tracing::info!("window manager initialization begin");
        let main = app
            .get_webview_window("main")
            .ok_or_else(|| WindowError("Tauri main window is missing".to_owned()))?;
        let hud = app
            .get_webview_window("hud")
            .ok_or_else(|| WindowError("Tauri HUD window is missing".to_owned()))?;
        hud.set_ignore_cursor_events(true).map_err(|error| {
            WindowError(format!("failed to make HUD window click-through: {error}"))
        })?;
        let manager = Self { app, main, hud };
        manager.position_hud()?;
        tracing::info!("window manager initialization success");
        Ok(manager)
    }

    fn position_hud(&self) -> Result<(), WindowError> {
        let monitor = self
            .app
            .primary_monitor()
            .map_err(|error| WindowError(format!("failed to query primary monitor: {error}")))?
            .ok_or_else(|| WindowError("no primary monitor is available".to_owned()))?;
        self.hud
            .set_size(Size::Physical(PhysicalSize::new(monitor.size().width, 64)))
            .map_err(|error| WindowError(format!("failed to size HUD window: {error}")))?;
        let hud_size = self
            .hud
            .outer_size()
            .map_err(|error| WindowError(format!("failed to query HUD window size: {error}")))?;
        let monitor_width = i32::try_from(monitor.size().width).unwrap_or(i32::MAX);
        let hud_width = i32::try_from(hud_size.width).unwrap_or(i32::MAX);
        let relative = compute_hud_position(monitor_width, hud_width);
        let origin = monitor.position();
        let absolute_x = origin.x.saturating_add(relative.x);
        let absolute_y = origin.y.saturating_add(relative.y);
        tracing::info!(
            monitor_width,
            hud_width,
            x = absolute_x,
            y = absolute_y,
            "positioning HUD window"
        );
        self.hud
            .set_position(Position::Physical(PhysicalPosition::new(
                absolute_x, absolute_y,
            )))
            .map_err(|error| WindowError(format!("failed to position HUD window: {error}")))?;
        tracing::info!(x = absolute_x, y = absolute_y, "HUD window positioned");
        Ok(())
    }
}

impl WindowManager for TauriWindowManager {
    fn apply_visible_window(&mut self, visible: WindowKind) -> Result<(), WindowError> {
        tracing::info!(?visible, "applying visible window");
        match visible {
            WindowKind::MainWindow => self.show_main_window(),
            WindowKind::HudWindow => self.show_hud_window(),
        }
    }

    fn show_main_window(&mut self) -> Result<(), WindowError> {
        tracing::info!("show main window begin");
        self.hud
            .hide()
            .map_err(|error| WindowError(format!("failed to hide HUD window: {error}")))?;
        tracing::info!("HUD window hidden");
        self.main
            .show()
            .map_err(|error| WindowError(format!("failed to show main window: {error}")))?;
        tracing::info!("main window shown");
        self.main
            .set_focus()
            .map_err(|error| WindowError(format!("failed to focus main window: {error}")))?;
        tracing::info!("main window focused");
        Ok(())
    }

    fn hide_main_window(&mut self) -> Result<(), WindowError> {
        tracing::info!("hide main window begin");
        self.main
            .hide()
            .map_err(|error| WindowError(format!("failed to hide main window: {error}")))?;
        tracing::info!("main window hidden");
        Ok(())
    }

    fn show_hud_window(&mut self) -> Result<(), WindowError> {
        tracing::info!("show HUD window begin");
        self.main
            .hide()
            .map_err(|error| WindowError(format!("failed to hide main window: {error}")))?;
        tracing::info!("main window hidden");
        self.position_hud()?;
        self.hud.set_ignore_cursor_events(true).map_err(|error| {
            WindowError(format!("failed to make HUD window click-through: {error}"))
        })?;
        self.hud
            .show()
            .map_err(|error| WindowError(format!("failed to show HUD window: {error}")))?;
        tracing::info!("HUD window shown");
        Ok(())
    }

    fn hide_hud_window(&mut self) -> Result<(), WindowError> {
        tracing::info!("hide HUD window begin");
        self.hud
            .hide()
            .map_err(|error| WindowError(format!("failed to hide HUD window: {error}")))?;
        tracing::info!("HUD window hidden");
        Ok(())
    }

    fn set_hud_light(&mut self, light: HudLight) -> Result<(), WindowError> {
        tracing::debug!(?light, "HUD light state updated");
        // State events are authoritative and the HUD maps State to its light itself.
        match light {
            HudLight::Yellow | HudLight::Green | HudLight::Off => Ok(()),
        }
    }
}

pub(crate) fn create_tauri_window_manager(
    app: AppHandle,
) -> Result<Box<dyn WindowManager + Send>, WindowError> {
    Ok(Box::new(TauriWindowManager::new(app)?))
}
