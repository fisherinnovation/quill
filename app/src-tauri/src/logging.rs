//! Logs go to stderr (visible under `npm run app`) and to daily files in
//! `<data dir>/logs`, which is the only place they go when Quill runs as an
//! app. Files older than two weeks are deleted.
//!
//! What's logged: events, timings and errors. Never transcript, summary or
//! dictated text, and never API keys.

use std::path::{Path, PathBuf};

use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

const KEEP_DAYS: usize = 14;

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// Sets up logging. If the log folder can't be used, logs only to stderr.
pub fn init(data_dir: &Path) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,whisper_rs=error,flacenc=warn"));
    let file = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("quill")
        .filename_suffix("log")
        .max_log_files(KEEP_DAYS)
        .build(dir(data_dir));
    // Written synchronously (not through a background thread), so nothing
    // is lost if the app crashes right after logging: that's when the log
    // matters most.
    let (file_layer, file_error) = match file {
        Ok(file) => (Some(fmt::layer().with_ansi(false).with_writer(file)), None),
        Err(e) => (None, Some(e)),
    };
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(file_layer)
        .init();
    if let Some(e) = file_error {
        tracing::warn!("logging to stderr only: {e}");
    }

    // Record panics in the log too (the default hook only prints to stderr).
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        default_hook(info);
    }));
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Quill starting");
}
