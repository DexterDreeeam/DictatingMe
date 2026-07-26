use std::sync::OnceLock;

use tracing_subscriber::util::SubscriberInitExt;

use std::path::PathBuf;

static INITIALIZATION: OnceLock<Result<(), String>> = OnceLock::new();

static FILE_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

pub(crate) fn init() -> Result<(), String> {
    INITIALIZATION.get_or_init(initialize).clone()
}

fn initialize() -> Result<(), String> {
    #[cfg(debug_assertions)]
    let log_dir = development_log_dir()?;

    #[cfg(debug_assertions)]
    {
        std::fs::create_dir_all(&log_dir).map_err(|error| {
            format!(
                "failed to create development log directory '{}': {error}",
                log_dir.display()
            )
        })?;
        let file = tracing_appender::rolling::RollingFileAppender::builder()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix("dictatingme-dev")
            .filename_suffix("log")
            .max_log_files(7)
            .build(&log_dir)
            .map_err(|error| {
                format!(
                    "failed to create development log appender in '{}': {error}",
                    log_dir.display()
                )
            })?;
        let (file_writer, guard) = tracing_appender::non_blocking(file);
        FILE_GUARD
            .set(guard)
            .map_err(|_| "development log guard was already initialized".to_owned())?;

        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_target(true)
            .with_thread_ids(true)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(file_writer)
            .finish()
            .try_init()
            .map_err(|error| format!("failed to install tracing subscriber: {error}"))?;
    }

    #[cfg(not(debug_assertions))]
    {
        let log_dir = release_log_dir()?;
        std::fs::create_dir_all(&log_dir).map_err(|error| {
            format!(
                "failed to create release log directory '{}': {error}",
                log_dir.display()
            )
        })?;
        let file = tracing_appender::rolling::RollingFileAppender::builder()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix("dictatingme")
            .filename_suffix("log")
            .max_log_files(7)
            .build(&log_dir)
            .map_err(|error| {
                format!(
                    "failed to create release log appender in '{}': {error}",
                    log_dir.display()
                )
            })?;
        let (file_writer, guard) = tracing_appender::non_blocking(file);
        FILE_GUARD
            .set(guard)
            .map_err(|_| "release log guard was already initialized".to_owned())?;
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_target(true)
            .with_thread_ids(true)
            .with_max_level(tracing::Level::INFO)
            .with_writer(file_writer)
            .finish()
            .try_init()
            .map_err(|error| format!("failed to install tracing subscriber: {error}"))?;
    }

    install_panic_hook();

    #[cfg(debug_assertions)]
    tracing::info!(
        log_directory = %log_dir.display(),
        log_pattern = "dictatingme-dev.YYYY-MM-DD.log",
        retained_files = 7,
        "development logging initialized"
    );
    #[cfg(not(debug_assertions))]
    tracing::info!("release file logging initialized");

    Ok(())
}

#[cfg(debug_assertions)]
fn development_log_dir() -> Result<PathBuf, String> {
    match std::env::var_os("DICTATINGME_DEV_LOG_DIR") {
        Some(path) if !path.is_empty() => Ok(PathBuf::from(path)),
        _ => std::env::current_dir()
            .map(|cwd| cwd.join("logs"))
            .map_err(|error| format!("failed to resolve current directory for logging: {error}")),
    }
}

#[cfg(not(debug_assertions))]
fn release_log_dir() -> Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|path| path.join("DictatingMe").join("logs"))
        .ok_or_else(|| "LOCALAPPDATA is unavailable for release logging".to_owned())
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let payload = panic_info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| {
                panic_info
                    .payload()
                    .downcast_ref::<String>()
                    .map(String::as_str)
            })
            .unwrap_or("<non-string panic payload>");
        if let Some(location) = panic_info.location() {
            tracing::error!(
                panic.file = location.file(),
                panic.line = location.line(),
                panic.column = location.column(),
                panic.payload = payload,
                "application panic"
            );
        } else {
            tracing::error!(panic.payload = payload, "application panic");
        }
        previous(panic_info);
    }));
}
