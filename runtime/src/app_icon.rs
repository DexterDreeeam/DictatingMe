use tauri::{image::Image, AppHandle, Manager, Theme};

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
    let main_window = app
        .get_webview_window("main")
        .ok_or_else(|| "main window is unavailable while applying the themed icon".to_owned())?;
    let window_icon = image_for_theme(theme, WINDOW_DARK, WINDOW_LIGHT)?;
    main_window
        .set_icon(window_icon)
        .map_err(|error| format!("failed to update the main window icon: {error}"))?;

    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        tray.set_icon(Some(tray_icon(theme)?))
            .map_err(|error| format!("failed to update the tray icon: {error}"))?;
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
