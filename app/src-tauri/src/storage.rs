//! Where recordings live, and exporting summaries.
//!
//! File and folder pickers open from Rust, so the UI never hands us a path
//! to write to: it asks for an action and we ask the user where.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use quill_core::library::export_file_name;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::{DialogExt, FileDialogBuilder};

use crate::AppState;
use crate::commands::{CommandError, CommandResult, blocking, lock};

#[derive(Debug, Clone, Serialize)]
pub struct StorageInfo {
    pub path: String,
    /// The built-in location (inside the app's data folder).
    pub is_default: bool,
    /// False if the folder is missing, e.g. on a drive that isn't connected.
    pub available: bool,
    /// Total size of everything in the folder.
    pub bytes: u64,
}

#[tauri::command]
pub async fn storage_info(app: AppHandle) -> CommandResult<StorageInfo> {
    blocking(move || Ok(info(&app.state::<AppState>()))).await
}

/// Recordings with nothing said in them.
#[derive(Debug, Clone, Serialize)]
pub struct EmptyRecordings {
    pub count: usize,
    pub bytes: u64,
}

/// Empty meetings that aren't recording or being processed right now.
fn empty_meeting_ids(state: &AppState) -> CommandResult<Vec<String>> {
    let recording = lock(&state.recording)
        .as_ref()
        .map(|r| r.meeting_id.clone());
    let processing = lock(&state.processing).clone();
    Ok(state
        .library
        .list()?
        .into_iter()
        .filter(|m| m.empty && Some(&m.id) != recording.as_ref() && !processing.contains(&m.id))
        .map(|m| m.id)
        .collect())
}

#[tauri::command]
pub async fn empty_recordings(app: AppHandle) -> CommandResult<EmptyRecordings> {
    blocking(move || {
        let state = app.state::<AppState>();
        let ids = empty_meeting_ids(&state)?;
        Ok(EmptyRecordings {
            count: ids.len(),
            bytes: ids
                .iter()
                .map(|id| dir_size(&state.library.meeting_dir(id)))
                .sum(),
        })
    })
    .await
}

/// Deletes every recording with nothing said in it, audio included.
/// Returns how many were deleted.
#[tauri::command]
pub async fn delete_empty_recordings(app: AppHandle) -> CommandResult<usize> {
    blocking(move || {
        let state = app.state::<AppState>();
        let ids = empty_meeting_ids(&state)?;
        for id in &ids {
            state.library.delete(id)?;
        }
        tracing::info!(count = ids.len(), "deleted empty recordings");
        // The meeting list changed outside the meeting view.
        let _ = app.emit(crate::LIBRARY_EVENT, ());
        Ok(ids.len())
    })
    .await
}

/// Keeps a recording with nothing said in it: it stops being flagged.
#[tauri::command]
pub fn keep_empty_recording(state: tauri::State<'_, AppState>, id: String) -> CommandResult<()> {
    Ok(state.library.keep_empty(&id)?)
}

/// One part of Quill's disk footprint.
#[derive(Debug, Clone, Serialize)]
pub struct FootprintPart {
    /// `recordings`, `models`, `library`, `logs` or `app`.
    pub key: &'static str,
    pub bytes: u64,
}

/// Everything Quill keeps on disk.
#[derive(Debug, Clone, Serialize)]
pub struct Footprint {
    pub total: u64,
    pub parts: Vec<FootprintPart>,
}

/// How much disk space Quill takes, by part. Walks the folders, so it runs
/// off the main thread.
#[tauri::command]
pub async fn footprint(app: AppHandle) -> CommandResult<Footprint> {
    blocking(move || {
        let state = app.state::<AppState>();
        let data = &state.data_dir;
        let recordings = state.library.recordings_dir();
        let models = quill_core::paths::models_dir().unwrap_or_else(|| data.join("models"));
        let logs = crate::logging::dir(data);
        // The library: the database and settings, i.e. whatever else is in
        // the data folder.
        let library: u64 = std::fs::read_dir(data)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| ![&recordings, &models, &logs].contains(&&e.path()))
                    .map(|e| match e.file_type() {
                        Ok(t) if t.is_dir() => dir_size(&e.path()),
                        Ok(_) => e.metadata().map_or(0, |m| m.len()),
                        Err(_) => 0,
                    })
                    .sum()
            })
            .unwrap_or(0);
        let parts = vec![
            FootprintPart {
                key: "recordings",
                bytes: dir_size(&recordings),
            },
            FootprintPart {
                key: "models",
                bytes: dir_size(&models),
            },
            FootprintPart {
                key: "library",
                bytes: library,
            },
            FootprintPart {
                key: "logs",
                bytes: dir_size(&logs),
            },
            FootprintPart {
                key: "app",
                bytes: app_size(),
            },
        ];
        Ok(Footprint {
            total: parts.iter().map(|p| p.bytes).sum(),
            parts,
        })
    })
    .await
}

/// Size of the installed app: the `.app` bundle on macOS, the install
/// folder on Windows (recognised by its uninstaller). Running from a build
/// folder, just the executable.
fn app_size() -> u64 {
    let Ok(exe) = std::env::current_exe() else {
        return 0;
    };
    if let Some(bundle) = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
    {
        return dir_size(bundle);
    }
    if let Some(dir) = exe.parent()
        && dir.join("uninstall.exe").is_file()
    {
        return dir_size(dir);
    }
    std::fs::metadata(&exe).map_or(0, |m| m.len())
}

/// Asks for a new recordings folder and moves every meeting's audio there.
/// Returns `None` if the user cancelled.
#[tauri::command]
pub async fn choose_recordings_dir(app: AppHandle) -> CommandResult<Option<StorageInfo>> {
    blocking(move || {
        let state = app.state::<AppState>();
        let Some(picked) = dialog(&app)
            .set_title("Choose where to keep recordings")
            .set_directory(state.library.recordings_dir())
            .blocking_pick_folder()
        else {
            return Ok(None);
        };
        let to = picked
            .into_path()
            .map_err(|e| CommandError::Other(e.to_string()))?;
        relocate(&state, &to, Some(to.clone()))?;
        Ok(Some(info(&state)))
    })
    .await
}

/// Moves recordings back to the built-in location.
#[tauri::command]
pub async fn reset_recordings_dir(app: AppHandle) -> CommandResult<StorageInfo> {
    blocking(move || {
        let state = app.state::<AppState>();
        let default = quill_core::settings::Settings::default().recordings_dir(&state.data_dir);
        relocate(&state, &default, None)?;
        Ok(info(&state))
    })
    .await
}

#[tauri::command]
pub fn show_recordings_dir(state: tauri::State<'_, AppState>) -> CommandResult<()> {
    tauri_plugin_opener::open_path(state.library.recordings_dir(), None::<&str>)
        .map_err(|e| CommandError::Other(e.to_string()))
}

/// Usage numbers for the Insights page.
#[tauri::command]
pub async fn get_insights(app: AppHandle) -> CommandResult<quill_core::insights::Insights> {
    blocking(move || {
        let state = app.state::<AppState>();
        Ok(quill_core::insights::compute(
            &state.library,
            &jiff::Zoned::now(),
        )?)
    })
    .await
}

/// Forgets dictation statistics (the history is separate).
#[tauri::command]
pub fn reset_dictation_stats(state: tauri::State<'_, AppState>) -> CommandResult<()> {
    Ok(state.library.clear_dictation_stats()?)
}

/// Opens the folder of log files (see logging.rs).
#[tauri::command]
pub fn show_logs(state: tauri::State<'_, AppState>) -> CommandResult<()> {
    let dir = crate::logging::dir(&state.data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| CommandError::Other(e.to_string()))?;
    tauri_plugin_opener::open_path(dir, None::<&str>)
        .map_err(|e| CommandError::Other(e.to_string()))
}

/// Asks where to save the meeting's latest summary and writes it there as
/// Markdown. Returns the saved path, or `None` if the user cancelled.
#[tauri::command]
pub async fn export_summary(app: AppHandle, id: String) -> CommandResult<Option<String>> {
    blocking(move || {
        let detail = app.state::<AppState>().library.get(&id)?;
        let summary = detail
            .summaries
            .first()
            .ok_or_else(|| CommandError::Other("this meeting has no summary yet".into()))?;
        let Some(picked) = dialog(&app)
            .set_title("Export summary")
            .set_file_name(export_file_name(&detail.meeting))
            .add_filter("Markdown", &["md"])
            .blocking_save_file()
        else {
            return Ok(None);
        };
        let mut path = picked
            .into_path()
            .map_err(|e| CommandError::Other(e.to_string()))?;
        if path.extension().is_none() {
            path.set_extension("md");
        }
        let mut text = summary.markdown.trim_end().to_string();
        text.push('\n');
        std::fs::write(&path, text)
            .map_err(|e| CommandError::Other(format!("could not save the summary: {e}")))?;
        Ok(Some(path.display().to_string()))
    })
    .await
}

/// Shows a file in Finder (Explorer on Windows), selected.
#[tauri::command]
pub fn reveal_file(path: String) -> CommandResult<()> {
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(|e| CommandError::Other(e.to_string()))
}

/// A file dialog attached to the main window (a sheet on macOS).
fn dialog(app: &AppHandle) -> FileDialogBuilder<tauri::Wry> {
    let builder = app.dialog().file();
    match app.get_webview_window("main") {
        Some(window) => builder.set_parent(&window),
        None => builder,
    }
}

/// Moves the recordings to `to` and saves `setting` as the new location
/// (`None` = default). Refuses while anything is using the audio files.
fn relocate(state: &AppState, to: &Path, setting: Option<PathBuf>) -> CommandResult<()> {
    // Set the flag before checking, and recording/processing check it after
    // registering themselves: whichever runs second sees the other.
    if state.moving.swap(true, Ordering::SeqCst) {
        return Err(CommandError::Other("already moving recordings".into()));
    }
    let result = (|| {
        if lock(&state.recording).is_some() {
            return Err(CommandError::Other(
                "stop the recording before moving recordings".into(),
            ));
        }
        if !lock(&state.processing).is_empty() {
            return Err(CommandError::Other(
                "wait for transcription to finish before moving recordings".into(),
            ));
        }
        let from = state.library.recordings_dir();
        let moved = state.library.move_recordings(to)?;
        let mut settings = lock(&state.settings).clone();
        settings.recordings_dir = setting;
        if let Err(e) = settings.save(&state.data_dir) {
            // Keep the files where the saved settings say they are.
            if let Err(undo) = state.library.move_recordings(&from) {
                tracing::error!("could not move recordings back: {undo}");
            }
            return Err(e.into());
        }
        *lock(&state.settings) = settings;
        tracing::info!(moved, to = %to.display(), "moved recordings");
        Ok(())
    })();
    state.moving.store(false, Ordering::SeqCst);
    result
}

fn info(state: &AppState) -> StorageInfo {
    let dir = state.library.recordings_dir();
    StorageInfo {
        path: dir.display().to_string(),
        is_default: lock(&state.settings).recordings_dir.is_none(),
        available: dir.is_dir(),
        bytes: dir_size(&dir),
    }
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(t) if t.is_dir() => dir_size(&entry.path()),
            Ok(_) => entry.metadata().map_or(0, |m| m.len()),
            Err(_) => 0,
        })
        .sum()
}
