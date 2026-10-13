//! Updates from GitHub Releases.
//!
//! Once a day (unless turned off in Settings, and never in debug builds)
//! Quill fetches the `latest.json` that the release workflow attaches to
//! each published release. It names each platform's update file and that
//! file's signature. Nothing installs until the user asks, and not while
//! Quill is recording, writing up a meeting, dictating or downloading a
//! model, since installing restarts the app.
//!
//! The updater plugin checks every download against the public key in
//! `tauri.conf.json`, and against the version signed into the signature
//! (`requireSignedVersion`), so a tampered `latest.json` can't point at an
//! older release. The private key is a GitHub Actions secret.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::AppState;
use crate::commands::{CommandError, CommandResult, lock};
use crate::dictation::{Dictation, Phase};

pub const EVENT: &str = "update-status";

/// The first automatic check waits for startup (model preload) to settle.
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the background thread looks at the clock. Short, because a
/// long `sleep` stops counting while the computer sleeps.
const TICK: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateStatus {
    /// Not checked yet this session.
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available {
        version: String,
    },
    Downloading {
        version: String,
        done: u64,
        total: Option<u64>,
    },
    /// Downloaded and verified; replacing the app, then restarting.
    Installing {
        version: String,
    },
}

/// What the UI shows (`update-status` events and `update_info`).
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub status: UpdateStatus,
    /// The last finished check, in Unix milliseconds.
    pub checked_at: Option<u64>,
}

#[derive(Default)]
pub struct Updates {
    inner: std::sync::Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    status: UpdateStatus,
    checked_at: Option<SystemTime>,
    /// What the last check found, ready to download.
    update: Option<Update>,
}

impl Updates {
    fn info(&self, app: &AppHandle) -> UpdateInfo {
        let inner = lock(&self.inner);
        UpdateInfo {
            current_version: app.package_info().version.to_string(),
            status: inner.status.clone(),
            checked_at: inner
                .checked_at
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .and_then(|d| u64::try_from(d.as_millis()).ok()),
        }
    }

    fn set(&self, app: &AppHandle, status: UpdateStatus) {
        lock(&self.inner).status = status;
        let _ = app.emit(EVENT, self.info(app));
    }

    fn status(&self) -> UpdateStatus {
        lock(&self.inner).status.clone()
    }
}

/// Starts the daily check in the background.
pub fn spawn_checks(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(FIRST_CHECK_AFTER);
        loop {
            if check_due(&app)
                && let Err(e) = tauri::async_runtime::block_on(check(&app))
            {
                // Usually just offline; the next tick tries again.
                tracing::warn!("couldn't check for updates: {e}");
            }
            std::thread::sleep(TICK);
        }
    });
}

fn check_due(app: &AppHandle) -> bool {
    if cfg!(debug_assertions) || !lock(&app.state::<AppState>().settings).check_for_updates {
        return false;
    }
    let checked_at = lock(&app.state::<Updates>().inner).checked_at;
    checked_at.is_none_or(|t| t.elapsed().is_ok_and(|e| e >= CHECK_EVERY))
}

async fn check(app: &AppHandle) -> CommandResult<()> {
    let updates = app.state::<Updates>();
    let previous = updates.status();
    if matches!(
        previous,
        UpdateStatus::Checking | UpdateStatus::Downloading { .. } | UpdateStatus::Installing { .. }
    ) {
        return Ok(());
    }
    updates.set(app, UpdateStatus::Checking);
    let started = Instant::now();
    let found = match app.updater() {
        Ok(updater) => updater.check().await,
        Err(e) => Err(e),
    };
    let found = match found {
        Ok(found) => found,
        Err(e) => {
            updates.set(app, previous);
            return Err(CommandError::Other(format!(
                "couldn't check for updates: {e}"
            )));
        }
    };
    let status = match &found {
        Some(update) => {
            tracing::info!(version = %update.version, ms = started.elapsed().as_millis(), "update available");
            UpdateStatus::Available {
                version: update.version.clone(),
            }
        }
        None => {
            tracing::info!(
                ms = started.elapsed().as_millis(),
                "checked for updates: up to date"
            );
            UpdateStatus::UpToDate
        }
    };
    {
        let mut inner = lock(&updates.inner);
        inner.update = found;
        inner.checked_at = Some(SystemTime::now());
    }
    updates.set(app, status);
    Ok(())
}

/// Why installing (which restarts Quill) has to wait, if it does.
fn busy(app: &AppHandle) -> Option<&'static str> {
    let state = app.state::<AppState>();
    if lock(&state.recording).is_some() {
        Some("Quill is recording. Update once the recording has stopped.")
    } else if !lock(&state.processing).is_empty() {
        Some("Quill is writing up a meeting. Update once it's done.")
    } else if state.moving.load(Ordering::SeqCst) {
        Some("Quill is moving your recordings. Update once that's done.")
    } else if app.state::<Dictation>().state().phase != Phase::Idle {
        Some("Quill is dictating. Update once it's done.")
    } else if state.downloads.any() {
        Some("A model is downloading. Update once it's done.")
    } else {
        None
    }
}

#[tauri::command]
pub fn update_info(app: AppHandle) -> UpdateInfo {
    app.state::<Updates>().info(&app)
}

/// Checks now, whether or not daily checks are on.
#[tauri::command]
pub async fn check_for_updates(app: AppHandle) -> CommandResult<UpdateInfo> {
    check(&app).await?;
    Ok(app.state::<Updates>().info(&app))
}

/// Downloads the update the last check found, installs it and restarts
/// Quill. Progress arrives as `update-status` events.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> CommandResult<()> {
    let updates = app.state::<Updates>();
    let (update, status) = {
        let inner = lock(&updates.inner);
        (inner.update.clone(), inner.status.clone())
    };
    let Some(update) = update else {
        return Err(CommandError::Other("there's no update to install".into()));
    };
    if !matches!(status, UpdateStatus::Available { .. }) {
        return Err(CommandError::Other(
            "the update is already installing".into(),
        ));
    }
    if let Some(why) = busy(&app) {
        return Err(CommandError::Other(why.into()));
    }
    let version = update.version.clone();
    let available = UpdateStatus::Available {
        version: version.clone(),
    };
    tracing::info!(%version, "downloading update");
    updates.set(
        &app,
        UpdateStatus::Downloading {
            version: version.clone(),
            done: 0,
            total: None,
        },
    );

    let started = Instant::now();
    let mut done = 0u64;
    let mut last_event = Instant::now();
    let downloaded = update
        .download(
            |chunk, total| {
                done += chunk as u64;
                if last_event.elapsed() >= Duration::from_millis(200) {
                    last_event = Instant::now();
                    updates.set(
                        &app,
                        UpdateStatus::Downloading {
                            version: version.clone(),
                            done,
                            total,
                        },
                    );
                }
            },
            || {},
        )
        .await;
    let bytes = match downloaded {
        Ok(bytes) => bytes,
        Err(e) => {
            updates.set(&app, available);
            return Err(CommandError::Other(format!(
                "couldn't download the update: {e}"
            )));
        }
    };
    tracing::info!(%version, bytes = bytes.len(), ms = started.elapsed().as_millis(), "update downloaded and verified");

    // Something may have started during the download.
    if let Some(why) = busy(&app) {
        updates.set(&app, available);
        return Err(CommandError::Other(why.into()));
    }
    updates.set(
        &app,
        UpdateStatus::Installing {
            version: version.clone(),
        },
    );
    // On Windows this starts the installer and exits; the installer opens
    // the new version when it's done.
    let installed = crate::commands::blocking(move || {
        update
            .install(bytes)
            .map_err(|e| CommandError::Other(e.to_string()))
    })
    .await;
    if let Err(e) = installed {
        updates.set(&app, available);
        return Err(CommandError::Other(format!(
            "couldn't install the update: {e}"
        )));
    }
    tracing::info!(%version, "update installed; restarting");
    app.restart()
}
