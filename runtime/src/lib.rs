//! DictatingMe Runtime crate 根：见 brainstrom/plan.md 全文，
//! 尤其 §5 模块设计表 与 §2 技术栈选型。

use std::path::Path;

use tauri::{path::BaseDirectory, Manager};

mod app_icon;
pub mod audio;
pub mod commands;
pub mod events;
pub mod evoke_setup;
pub mod input_monitor;
mod logging;
pub mod models;
pub mod platform;
pub mod runtime;
pub mod scoring;
pub mod settings;
pub mod state_machine;
pub mod storage;
pub mod text;
pub mod tray;
pub mod window;

use audio::{AudioBus, CpalAudioCapture};
use models::{DictationModelEngine, EvokeModelEngine};
use platform::windows::{WindowsInputMonitor, WindowsTextInjector};
use runtime::{RuntimeCore, RuntimeError, RuntimeHandle};
use settings::SettingsHandle;
use storage::{AppPaths, AssetManager, ConfigStore, Database, SettingsStore};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(error) = logging::init() {
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "DictatingMe logging initialization failed: {error}"
        );
    }
    let cwd = std::env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|error| format!("<unavailable: {error}>"));
    let executable = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|error| format!("<unavailable: {error}>"));
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        %cwd,
        %executable,
        model_dir = ?std::env::var_os("DICTATINGME_MODEL_DIR"),
        "DictatingMe startup"
    );

    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::frontend_ready,
            commands::list_devices,
            commands::set_input_device,
            commands::get_config,
            commands::get_settings_snapshot,
            commands::inspect_asset,
            commands::install_asset,
            commands::select_dictation_model,
            commands::get_operation,
            commands::begin_evoke_setup,
            commands::capture_evoke_sample,
            commands::finish_evoke_setup,
            commands::get_evoke_setup,
            commands::cancel_evoke_setup,
            commands::set_sensitivity,
            commands::list_history,
            commands::copy_history_text,
            commands::play_history_audio,
            commands::request_background,
            commands::quit_app,
        ])
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::ThemeChanged(_) = event {
                let result = app_icon::current_theme(window.app_handle())
                    .and_then(|theme| app_icon::apply_theme(window.app_handle(), theme));
                if let Err(error) = result {
                    tracing::error!(%error, "failed to apply system-themed application icon");
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Some(runtime) = window.app_handle().try_state::<RuntimeHandle>() {
                    tracing::info!("main window close requested");
                    runtime.notify_event(
                        state_machine::StateEvent::MainWindowClosed,
                        "main-window close",
                    );
                } else {
                    tracing::error!(
                        "main-window close callback failed: runtime handle is unavailable"
                    );
                }
            }
        })
        .setup(|app| {
            tracing::info!("runtime setup begin");
            match setup_runtime(app) {
                Ok(()) => {
                    tracing::info!("runtime setup success");
                    Ok(())
                }
                Err(error) => {
                    tracing::error!(error = %error, "runtime setup error");
                    Err(Box::new(error) as Box<dyn std::error::Error>)
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to initialize DictatingMe Tauri application");

    app.run(|_app, event| {
        if let tauri::RunEvent::ExitRequested {
            code: None, api, ..
        } = event
        {
            tracing::debug!("preventing implicit Tauri exit request");
            api.prevent_exit();
        }
    });
    tracing::info!("Tauri event loop stopped");
}

fn setup_runtime(app: &mut tauri::App) -> Result<(), RuntimeError> {
    tracing::debug!("setup phase: remove configured tray");
    drop(app.remove_tray_by_id("main"));
    let initial_theme = app_icon::current_theme(app.handle()).map_err(RuntimeError)?;
    app_icon::apply_theme(app.handle(), initial_theme).map_err(RuntimeError)?;

    tracing::debug!("setup phase: resolve application paths");
    let legacy_app_data_dir = app.path().app_local_data_dir().map_err(|error| {
        RuntimeError(format!(
            "failed to resolve local app data directory: {error}"
        ))
    })?;
    let local_data_dir = legacy_app_data_dir.parent().ok_or_else(|| {
        RuntimeError(format!(
            "application data directory has no parent: {}",
            legacy_app_data_dir.display()
        ))
    })?;
    let app_data_dir = local_data_dir.join("DictatingMe");
    let paths = AppPaths::new(app_data_dir.clone());
    paths
        .migrate_from(&legacy_app_data_dir)
        .map_err(|error| RuntimeError(error.0))?;
    paths.ensure().map_err(|error| RuntimeError(error.0))?;
    let packaged_manifest = app
        .path()
        .resolve("manifest-cn.json", BaseDirectory::Resource)
        .map_err(|error| RuntimeError(format!("failed to resolve packaged manifest: {error}")))?;
    sync_packaged_manifest(&packaged_manifest, &paths.manifest)?;
    let recordings_dir = paths.history.clone();
    let database_path = paths.database.clone();
    tracing::info!(
        app_data_directory = %app_data_dir.display(),
        legacy_app_data_directory = %legacy_app_data_dir.display(),
        assets_directory = %paths.assets.display(),
        manifest_path = %paths.manifest.display(),
        recordings_directory = %recordings_dir.display(),
        database_path = %database_path.display(),
        "application paths resolved"
    );
    let database_path = path_text(&database_path, "database")?;
    tracing::debug!("setup phase: open database and load configuration");
    let database = Database::open(database_path).map_err(|error| RuntimeError(error.0))?;
    let config_store = ConfigStore::new(database.clone());
    let asset_manager = AssetManager::load_manifest(paths.clone(), &paths.manifest)
        .map_err(|error| RuntimeError(error.0))?;
    asset_manager
        .bootstrap_embedded()
        .map_err(|error| RuntimeError(error.0))?;
    asset_manager
        .cleanup_transient()
        .map_err(|error| RuntimeError(error.0))?;
    let settings_store = SettingsStore::new(database.clone());
    let audio_bus = AudioBus::new(128);
    let settings = SettingsHandle::new(
        settings_store,
        config_store,
        asset_manager,
        paths,
        audio_bus.clone(),
        app.handle().clone(),
    )
    .map_err(|error| RuntimeError(error.0))?;
    let bundle = settings
        .initial_bundle()
        .map_err(|error| RuntimeError(error.0))?;

    tracing::debug!("setup phase: resolve model directories");
    let evoke_path = bundle.preset_model_path.clone();
    tracing::info!(
        evoke_model_directory = %evoke_path.display(),
        dictation_model_id = bundle
            .dictation_model
            .as_ref()
            .map(|model| model.id.as_str())
            .unwrap_or("unconfigured"),
        dictation_model_directory = bundle
            .dictation_model
            .as_ref()
            .map(|model| model.root.display().to_string())
            .unwrap_or_else(|| "unconfigured".to_owned()),
        "model directories resolved"
    );
    tracing::debug!("setup phase: initialize model engines");
    let mut evoke_model = EvokeModelEngine::new(
        path_text(&evoke_path, "evoke model")?,
        bundle.profile.phrase.clone(),
        bundle.profile.mode.kws_max_active_paths(),
    )
    .map_err(|error| RuntimeError(format!("failed to initialize evoke model: {}", error.0)))?;
    evoke_model.set_sensitivity(bundle.config.sensitivity);
    let dictation_model = DictationModelEngine::new(bundle.dictation_model.clone());

    tracing::debug!("setup phase: initialize tray and windows");
    let tray_manager = tray::create_tauri_tray_manager(app.handle().clone());
    let window_manager = window::create_tauri_window_manager(app.handle().clone())
        .map_err(|error| RuntimeError(error.0))?;
    if !app.manage(settings.clone()) {
        return Err(RuntimeError(
            "a settings handle was already registered with Tauri".to_owned(),
        ));
    }

    let core = RuntimeCore::new(
        Box::new(CpalAudioCapture::new()),
        evoke_model,
        dictation_model,
        Box::new(WindowsTextInjector::new()),
        Box::new(WindowsInputMonitor::new()),
        database,
        tray_manager,
        window_manager,
        settings.clone(),
        bundle,
    )?;
    tracing::debug!("setup phase: spawn runtime actor");
    let handle = RuntimeHandle::spawn(core, app.handle().clone(), recordings_dir, audio_bus)?;
    if !app.manage(handle.clone()) {
        return Err(RuntimeError(
            "a runtime handle was already registered with Tauri".to_owned(),
        ));
    }
    tracing::info!("runtime and settings handles registered; waiting for Tauri Ready");
    Ok(())
}

fn sync_packaged_manifest(source: &Path, destination: &Path) -> Result<(), RuntimeError> {
    let bytes = std::fs::read(source).map_err(|error| {
        RuntimeError(format!(
            "failed to read packaged manifest '{}': {error}",
            source.display()
        ))
    })?;
    if std::fs::read(destination).ok().as_deref() == Some(bytes.as_slice()) {
        return Ok(());
    }

    let temporary = destination.with_extension("json.tmp");
    let backup = destination.with_extension("json.backup");
    std::fs::write(&temporary, &bytes).map_err(|error| {
        RuntimeError(format!(
            "failed to stage AppData manifest '{}': {error}",
            temporary.display()
        ))
    })?;
    if backup.exists() {
        std::fs::remove_file(&backup).map_err(|error| {
            RuntimeError(format!(
                "failed to remove stale manifest backup '{}': {error}",
                backup.display()
            ))
        })?;
    }
    if destination.exists() {
        std::fs::rename(destination, &backup).map_err(|error| {
            RuntimeError(format!(
                "failed to back up AppData manifest '{}': {error}",
                destination.display()
            ))
        })?;
    }
    if let Err(error) = std::fs::rename(&temporary, destination) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, destination);
        }
        return Err(RuntimeError(format!(
            "failed to activate AppData manifest '{}': {error}",
            destination.display()
        )));
    }
    if backup.exists() {
        std::fs::remove_file(&backup).map_err(|error| {
            RuntimeError(format!(
                "failed to remove manifest backup '{}': {error}",
                backup.display()
            ))
        })?;
    }
    tracing::info!(
        source = %source.display(),
        destination = %destination.display(),
        "plaintext asset manifest synchronized"
    );
    Ok(())
}

fn path_text<'a>(path: &'a Path, purpose: &str) -> Result<&'a str, RuntimeError> {
    path.to_str()
        .ok_or_else(|| RuntimeError(format!("{purpose} path is not valid UTF-8")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_manifest_overwrites_appdata_copy() {
        let root =
            std::env::temp_dir().join(format!("dictatingme-manifest-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("packaged.json");
        let destination = root.join("manifest-cn.json");
        std::fs::write(&source, br#"{"schemaVersion":1,"locale":"zh-CN"}"#).unwrap();
        std::fs::write(&destination, br#"{"schemaVersion":0}"#).unwrap();

        sync_packaged_manifest(&source, &destination).unwrap();

        assert_eq!(
            std::fs::read(&destination).unwrap(),
            std::fs::read(&source).unwrap()
        );
        assert!(!destination.with_extension("json.tmp").exists());
        assert!(!destination.with_extension("json.backup").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
