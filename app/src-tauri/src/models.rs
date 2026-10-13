//! Downloaded models: what's needed, what's on disk, downloading and
//! deleting them.
//!
//! The installers carry no models; they download on first use into the
//! models folder (shared with `quill-cli`). Every download goes through
//! [`ensure`], which runs one download at a time, so a meeting and a
//! dictation needing the same model never write the same file at once, and
//! reports progress to the UI as `model-download` events.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use quill_core::settings::Settings;
use quill_core::speakers;
use quill_stt::models::{self, ModelInfo};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;
use crate::commands::{CommandError, CommandResult, lock};

pub const MODEL_EVENT: &str = "model-download";

/// Download state shared by everything that needs a model.
#[derive(Default)]
pub struct Downloads {
    /// Held for the whole of each download.
    lock: Mutex<()>,
    /// Models downloading or waiting their turn.
    active: Mutex<HashSet<&'static str>>,
}

impl Downloads {
    /// A model is downloading or waiting its turn.
    pub fn any(&self) -> bool {
        !lock(&self.active).is_empty()
    }
}

/// One `model-download` event.
#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub name: &'static str,
    pub done: u64,
    pub total: u64,
    /// Set on the last event for this download.
    pub finished: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Whisper.
    Speech,
    /// Silence detection.
    Vad,
    /// Telling speakers apart.
    Speakers,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelEntry {
    pub name: &'static str,
    pub kind: Kind,
    pub description: &'static str,
    pub size_bytes: u64,
    pub downloaded: bool,
    /// Needed by the current settings.
    pub in_use: bool,
    pub downloading: bool,
    /// One of the models *Tell speakers apart* uses.
    pub for_speakers: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelsInfo {
    pub dir: String,
    pub models: Vec<ModelEntry>,
    /// Space all downloaded models take.
    pub bytes_on_disk: u64,
    /// Still to download for the current settings.
    pub missing_bytes: u64,
}

fn models_dir() -> Result<PathBuf, String> {
    quill_core::paths::models_dir().ok_or_else(|| "no home directory".to_string())
}

/// The models the current settings need: the Whisper model, the VAD model
/// if silence skipping is on, and the speaker models if telling speakers
/// apart is on.
pub fn required(settings: &Settings) -> Vec<&'static ModelInfo> {
    let mut list: Vec<&'static ModelInfo> = models::find(&settings.stt_model).into_iter().collect();
    if settings.vad {
        list.push(&models::VAD_MODEL);
    }
    if settings.identify_speakers {
        list.extend(speakers::required_models());
    }
    list
}

/// The models dictation needs (Whisper, and VAD if on) that aren't
/// downloaded yet.
pub fn missing_for_dictation(settings: &Settings) -> Vec<&'static ModelInfo> {
    let Ok(dir) = models_dir() else {
        return Vec::new();
    };
    let mut list: Vec<&'static ModelInfo> = models::find(&settings.stt_model).into_iter().collect();
    if settings.vad {
        list.push(&models::VAD_MODEL);
    }
    list.retain(|m| !m.is_downloaded(&dir));
    list
}

/// Downloads `model` if it isn't already there and returns its path.
/// Blocking. Waits for any other download to finish first. `on_progress`
/// gets (done, total) bytes, as do `model-download` listeners.
pub fn ensure(
    app: &AppHandle,
    model: &'static ModelInfo,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<PathBuf, String> {
    let dir = models_dir()?;
    let downloads = &app.state::<AppState>().downloads;
    if model.is_downloaded(&dir) {
        // It may have been queued by `download_in_background`.
        lock(&downloads.active).remove(model.name);
        return Ok(model.path_in(&dir));
    }
    lock(&downloads.active).insert(model.name);
    let result = {
        let _turn = lock(&downloads.lock);
        // Someone else may have downloaded it while we waited.
        if model.is_downloaded(&dir) {
            Ok(model.path_in(&dir))
        } else {
            tracing::info!(
                model = model.name,
                mb = model.size_bytes / 1_000_000,
                "downloading"
            );
            let mut last = 0_u64;
            models::download(model, &dir, |done, total| {
                on_progress(done, total);
                // About one event per percent.
                if done - last >= total / 100 || done == total {
                    last = done;
                    emit(app, model.name, done, total, false, None);
                }
            })
            .map_err(|e| e.to_string())
        }
    };
    lock(&downloads.active).remove(model.name);
    match &result {
        Ok(_) => emit(
            app,
            model.name,
            model.size_bytes,
            model.size_bytes,
            true,
            None,
        ),
        Err(e) => {
            tracing::warn!(model = model.name, "download failed: {e}");
            emit(app, model.name, 0, model.size_bytes, true, Some(e.clone()));
        }
    }
    result
}

/// Starts downloading `list` one after another in the background. All of
/// them count as downloading from now, so the UI doesn't flicker back to a
/// Download button between two models.
pub fn download_in_background(app: &AppHandle, list: Vec<&'static ModelInfo>) {
    let downloads = &app.state::<AppState>().downloads;
    lock(&downloads.active).extend(list.iter().map(|m| m.name));
    let app = app.clone();
    std::thread::spawn(move || {
        let mut rest = list.into_iter();
        for model in rest.by_ref() {
            if ensure(&app, model, |_, _| {}).is_err() {
                // The event carries the error; don't start the rest.
                break;
            }
        }
        let downloads = &app.state::<AppState>().downloads;
        for model in rest {
            lock(&downloads.active).remove(model.name);
        }
    });
}

fn emit(
    app: &AppHandle,
    name: &'static str,
    done: u64,
    total: u64,
    finished: bool,
    error: Option<String>,
) {
    let progress = DownloadProgress {
        name,
        done,
        total,
        finished,
        error,
    };
    if let Err(e) = app.emit(MODEL_EVENT, progress) {
        tracing::warn!("could not emit model progress: {e}");
    }
}

/// Every model the app might use: all Whisper sizes, VAD, and the speaker
/// models. Other voice models (for `quill-cli diarize` comparisons) are
/// listed only if downloaded, so their space can be reclaimed.
fn catalog(dir: &std::path::Path) -> Vec<(&'static ModelInfo, Kind)> {
    let mut list: Vec<_> = models::MODELS.iter().map(|m| (m, Kind::Speech)).collect();
    list.push((&models::VAD_MODEL, Kind::Vad));
    list.push((&models::SEGMENTATION_MODEL, Kind::Speakers));
    for m in models::VOICE_MODELS {
        if m.name == speakers::VOICE_MODEL || m.is_downloaded(dir) {
            list.push((m, Kind::Speakers));
        }
    }
    list
}

fn info(app: &AppHandle) -> Result<ModelsInfo, String> {
    let state = app.state::<AppState>();
    let dir = models_dir()?;
    let needed: Vec<&str> = required(&lock(&state.settings))
        .iter()
        .map(|m| m.name)
        .collect();
    let active = lock(&state.downloads.active).clone();
    let entries: Vec<ModelEntry> = catalog(&dir)
        .into_iter()
        .map(|(m, kind)| ModelEntry {
            name: m.name,
            kind,
            description: m.description,
            size_bytes: m.size_bytes,
            downloaded: m.is_downloaded(&dir),
            in_use: needed.contains(&m.name),
            downloading: active.contains(m.name),
            for_speakers: speakers::required_models().contains(&m),
        })
        .collect();
    Ok(ModelsInfo {
        dir: dir.display().to_string(),
        bytes_on_disk: entries
            .iter()
            .filter(|m| m.downloaded)
            .map(|m| m.size_bytes)
            .sum(),
        missing_bytes: entries
            .iter()
            .filter(|m| m.in_use && !m.downloaded)
            .map(|m| m.size_bytes)
            .sum(),
        models: entries,
    })
}

#[tauri::command]
pub fn models_info(app: AppHandle) -> CommandResult<ModelsInfo> {
    info(&app).map_err(CommandError::Other)
}

/// Downloads whatever the current settings need, in the background.
/// Progress arrives as `model-download` events.
#[tauri::command]
pub fn download_needed_models(app: AppHandle) -> CommandResult<()> {
    let dir = models_dir().map_err(CommandError::Other)?;
    let settings = lock(&app.state::<AppState>().settings).clone();
    let missing: Vec<_> = required(&settings)
        .into_iter()
        .filter(|m| !m.is_downloaded(&dir))
        .collect();
    download_in_background(&app, missing);
    Ok(())
}

#[tauri::command]
pub fn download_model(app: AppHandle, name: String) -> CommandResult<()> {
    let model = models::find(&name)
        .ok_or_else(|| CommandError::Other(format!("unknown model `{name}`")))?;
    download_in_background(&app, vec![model]);
    Ok(())
}

/// Deletes a downloaded model. One the settings use downloads again the
/// next time it's needed (or from Settings → Models).
#[tauri::command]
pub fn delete_model(app: AppHandle, name: String) -> CommandResult<ModelsInfo> {
    let model = find(&name)?;
    remove(&app, model)?;
    info(&app).map_err(CommandError::Other)
}

/// Deletes a model and downloads it again, checking it against its hash:
/// for a file that seems damaged. Progress arrives as `model-download`
/// events.
#[tauri::command]
pub fn redownload_model(app: AppHandle, name: String) -> CommandResult<ModelsInfo> {
    let model = find(&name)?;
    remove(&app, model)?;
    download_in_background(&app, vec![model]);
    info(&app).map_err(CommandError::Other)
}

/// Deletes every downloaded model (and any half-finished download). Those
/// the settings use download again when next needed.
#[tauri::command]
pub fn delete_all_models(app: AppHandle) -> CommandResult<ModelsInfo> {
    let dir = models_dir().map_err(CommandError::Other)?;
    let all = models::MODELS.iter().chain(models::support_models());
    let mut skipped = Vec::new();
    for model in all {
        if !model.path_in(&dir).is_file() && !model.part_path_in(&dir).is_file() {
            continue;
        }
        if let Err(e) = remove(&app, model) {
            tracing::warn!(model = model.name, "not deleted: {e}");
            skipped.push(model.name);
        }
    }
    if !skipped.is_empty() {
        return Err(CommandError::Other(format!(
            "deleted the others, but not {} (downloading)",
            skipped.join(", ")
        )));
    }
    info(&app).map_err(CommandError::Other)
}

fn find(name: &str) -> CommandResult<&'static ModelInfo> {
    models::find(name).ok_or_else(|| CommandError::Other(format!("unknown model `{name}`")))
}

/// Deletes a model's file and any leftover partial download, and unloads
/// the Whisper model in memory if this is it (or its VAD model), so the
/// next use loads a fresh copy. Refuses while the model is downloading.
fn remove(app: &AppHandle, model: &'static ModelInfo) -> CommandResult<()> {
    let state = app.state::<AppState>();
    if lock(&state.downloads.active).contains(model.name) {
        return Err(CommandError::Other(format!(
            "{} is downloading; try again when it's done",
            model.name
        )));
    }
    let dir = models_dir().map_err(CommandError::Other)?;
    for path in [model.path_in(&dir), model.part_path_in(&dir)] {
        if path.is_file() {
            std::fs::remove_file(&path).map_err(|e| CommandError::Other(e.to_string()))?;
        }
    }
    tracing::info!(model = model.name, "deleted model");
    // A meeting already transcribing keeps its own reference to the loaded
    // model and finishes normally.
    let mut loaded = lock(&state.transcriber);
    if loaded
        .as_ref()
        .is_some_and(|t| t.model_name() == model.name || model.name == models::VAD_MODEL.name)
    {
        *loaded = None;
    }
    Ok(())
}

/// Opens the models folder in Finder / Explorer.
#[tauri::command]
pub fn show_models_dir() -> CommandResult<()> {
    let dir = models_dir().map_err(CommandError::Other)?;
    std::fs::create_dir_all(&dir).map_err(|e| CommandError::Other(e.to_string()))?;
    tauri_plugin_opener::open_path(dir, None::<&str>)
        .map_err(|e| CommandError::Other(e.to_string()))
}
