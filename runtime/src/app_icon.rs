use tauri::{image::Image, AppHandle, Manager, Theme};

#[cfg(windows)]
use windows::{
    core::w,
    Win32::{
        Foundation::{ERROR_SUCCESS, LPARAM, WPARAM},
        System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
        UI::WindowsAndMessaging::{
            SendMessageW, ICON_BIG, ICON_SMALL, ICON_SMALL2, WM_GETICON, WM_SETICON,
        },
    },
};

const WINDOW_DARK: &[u8] = include_bytes!("../icons/logo-dark.png");
const WINDOW_LIGHT: &[u8] = include_bytes!("../icons/logo-light.png");
const TRAY_DARK: &[u8] = include_bytes!("../icons/tray-dark.png");
const TRAY_LIGHT: &[u8] = include_bytes!("../icons/tray-light.png");
pub(crate) const TRAY_ID: &str = "dictatingme-runtime";

fn image_for_theme(
    theme: Theme,
    dark: &'static [u8],
    light: &'static [u8],
) -> Result<Image<'static>, String> {
    let bytes = if matches!(theme, Theme::Light) {
        light
    } else {
        dark
    };
    Image::from_bytes(bytes).map_err(|error| format!("failed to decode application icon: {error}"))
}

#[cfg(windows)]
pub(crate) fn current_theme(app: &AppHandle) -> Result<Theme, String> {
    let _ = app;
    let mut apps_use_light_theme = 1u32;
    let mut value_size = std::mem::size_of_val(&apps_use_light_theme) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut apps_use_light_theme as *mut u32).cast()),
            Some(&mut value_size),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "failed to read Windows application theme: Win32 error {}",
            status.0
        ));
    }
    Ok(if apps_use_light_theme == 0 {
        Theme::Dark
    } else {
        Theme::Light
    })
}

#[cfg(not(windows))]
pub(crate) fn current_theme(app: &AppHandle) -> Result<Theme, String> {
    app.get_webview_window("main")
        .ok_or_else(|| "main window is unavailable while resolving the system theme".to_owned())?
        .theme()
        .map_err(|error| format!("failed to resolve the system theme: {error}"))
}

pub(crate) fn tray_icon(theme: Theme) -> Result<Image<'static>, String> {
    image_for_theme(theme, TRAY_DARK, TRAY_LIGHT)
}

pub(crate) fn apply_theme(app: &AppHandle, theme: Theme) -> Result<(), String> {
    tracing::info!(?theme, "applying system-themed application icons");
    let main_window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window is unavailable while applying the themed icon".to_owned())?;
    let window_icon = image_for_theme(theme, WINDOW_DARK, WINDOW_LIGHT)?;
    main_window
        .set_icon(window_icon)
        .map_err(|error| format!("failed to update the main window icon: {error}"))?;
    #[cfg(windows)]
    apply_taskbar_icon(&main_window)?;

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_icon(Some(tray_icon(theme)?))
            .map_err(|error| format!("failed to update the tray icon: {error}"))?;
    }
    Ok(())
}

#[cfg(windows)]
fn apply_taskbar_icon(main_window: &tauri::WebviewWindow) -> Result<(), String> {
    let hwnd = main_window
        .hwnd()
        .map_err(|error| format!("failed to resolve the main window handle: {error}"))?;
    let mut icon = unsafe {
        SendMessageW(
            hwnd,
            WM_GETICON,
            Some(WPARAM(ICON_SMALL as usize)),
            Some(LPARAM(0)),
        )
    };
    if icon.0 == 0 {
        icon = unsafe {
            SendMessageW(
                hwnd,
                WM_GETICON,
                Some(WPARAM(ICON_SMALL2 as usize)),
                Some(LPARAM(0)),
            )
        };
    }
    if icon.0 == 0 {
        return Err("main window did not expose a themed small icon".to_owned());
    }

    unsafe {
        SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_BIG as usize)),
            Some(LPARAM(icon.0)),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themed_icons_decode_at_expected_sizes() {
        for (theme, window_size, tray_size) in [(Theme::Dark, 256, 32), (Theme::Light, 256, 32)] {
            let window = image_for_theme(theme, WINDOW_DARK, WINDOW_LIGHT).unwrap();
            assert_eq!(
                (window.width(), window.height()),
                (window_size, window_size)
            );

            let tray = tray_icon(theme).unwrap();
            assert_eq!((tray.width(), tray.height()), (tray_size, tray_size));
        }
    }
}
