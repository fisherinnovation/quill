//! Functions the UI can call with `invoke("name", { args })`.
//!
//! Each `#[tauri::command]` function's arguments are deserialized from the
//! JS call (camelCase in JS → snake_case here) and its `Ok` value is
//! serialized back as JSON. Errors must be serializable too; we send their
//! message as a string.

use quill_core::library::{LibraryError, Meeting, MeetingDetail, SearchHit};
use quill_core::settings::{Settings, SettingsError};
use std::sync::atomic::Ordering;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::pipeline::{self, Steps};
use crate::recorder::ActiveRecording;
use crate::{AppState, tray};

/// Sent to the UI whenever recording starts or stops.
pub const RECORDING_EVENT: &str = "recording-state";

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error(transparent)]
    Library(#[from] LibraryError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Llm(#[from] quill_llm::LlmError),
    #[error("{0}")]
    Other(String),
}

// Tauri requires command errors to be `Serialize`; the UI only needs the
// message.
impl serde::Serialize for CommandError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type CommandResult<T> = Result<T, CommandError>;

#[tauri::command]
pub fn list_meetings(state: State<'_, AppState>) -> CommandResult<Vec<Meeting>> {
    Ok(state.library.list()?)
}

/// What the Meetings page shows about each meeting beyond its title: the
/// conversation's shape, who was there, the gist and its action items.
#[tauri::command]
pub async fn meeting_overviews(
    app: AppHandle,
) -> CommandResult<Vec<quill_core::overview::Overview>> {
    blocking(move || Ok(app.state::<AppState>().library.overviews()?)).await
}

/// Every meeting's action items, for the Action items page, the sidebar's
/// count and the checkboxes in a meeting's notes.
#[tauri::command]
pub async fn action_items(app: AppHandle) -> CommandResult<Vec<quill_core::library::ActionItem>> {
    blocking(move || Ok(app.state::<AppState>().library.action_items()?)).await
}

/// Ticks an action item off, or back on.
#[tauri::command]
pub fn set_action_item_done(
    state: State<'_, AppState>,
    id: i64,
    done: bool,
) -> CommandResult<quill_core::library::ActionItem> {
    Ok(state.library.set_action_item_done(id, done)?)
}

#[tauri::command]
pub fn get_meeting(state: State<'_, AppState>, id: String) -> CommandResult<MeetingDetail> {
    Ok(state.library.get(&id)?)
}

#[tauri::command]
pub fn search_transcripts(
    state: State<'_, AppState>,
    query: String,
) -> CommandResult<Vec<SearchHit>> {
    Ok(state.library.search(&query, 50)?)
}

#[tauri::command]
pub fn rename_meeting(state: State<'_, AppState>, id: String, title: String) -> CommandResult<()> {
    Ok(state.library.rename(&id, title.trim())?)
}

#[tauri::command]
pub fn delete_meeting(state: State<'_, AppState>, id: String) -> CommandResult<()> {
    Ok(state.library.delete(&id)?)
}

/// Talk time, pace, moments and most used words for one meeting's
/// transcript (empty stats if it isn't transcribed).
#[tauri::command]
pub async fn meeting_stats(
    app: AppHandle,
    id: String,
) -> CommandResult<quill_core::stats::MeetingStats> {
    blocking(move || {
        let detail = app.state::<AppState>().library.get(&id)?;
        Ok(quill_core::stats::meeting(&detail.segments))
    })
    .await
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    lock(&state.settings).clone()
}

/// Saves the settings form. The recordings folder, onboarding flag and
/// dictation settings have their own commands, so the stored values win over
/// whatever the form sent.
#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    mut settings: Settings,
) -> CommandResult<()> {
    let theme_was = {
        let current = lock(&state.settings);
        settings.recordings_dir.clone_from(&current.recordings_dir);
        settings.onboarded = current.onboarded;
        settings.dictation.clone_from(&current.dictation);
        current.theme
    };
    settings.save(&state.data_dir)?;
    let theme = settings.theme;
    *lock(&state.settings) = settings;
    if theme != theme_was {
        crate::theme::apply(&app, theme);
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordingState {
    pub recording: bool,
    pub paused: bool,
    pub meeting_id: Option<String>,
    /// Time recorded, pauses left out.
    pub elapsed_ms: u64,
}

#[tauri::command]
pub fn recording_state(state: State<'_, AppState>) -> RecordingState {
    current(&state)
}

// Async commands run off the main thread. Starting can block for as long as
// a macOS permission prompt is open, and stopping waits for the WAV files to
// be finalized; neither should freeze the window, so both run on a
// blocking-friendly worker thread.
#[tauri::command]
pub async fn start_recording(app: AppHandle) -> CommandResult<Meeting> {
    tauri::async_runtime::spawn_blocking(move || start(&app))
        .await
        .map_err(|e| CommandError::Other(e.to_string()))?
}

#[tauri::command]
pub async fn stop_recording(app: AppHandle) -> CommandResult<String> {
    tauri::async_runtime::spawn_blocking(move || stop(&app))
        .await
        .map_err(|e| CommandError::Other(e.to_string()))?
}

/// Re-runs processing for a meeting: `transcribe` = transcribe (+ summarize
/// if enabled in settings); otherwise summarize the existing transcript.
#[tauri::command]
pub fn process_meeting(app: AppHandle, id: String, transcribe: bool) {
    let steps = if !transcribe {
        Steps::SummarizeOnly
    } else if lock(&app.state::<AppState>().settings).auto_summarize {
        Steps::TranscribeAndSummarize
    } else {
        Steps::TranscribeOnly
    };
    pipeline::spawn(app, id, steps);
}

/// Splits the people on the call into speakers again, into `speakers` if
/// given (otherwise as many as their voices suggest). Replaces names given
/// to speakers. Progress arrives as `meeting-progress` events.
#[tauri::command]
pub fn split_speakers(app: AppHandle, id: String, speakers: Option<usize>) {
    pipeline::spawn(app, id, Steps::Speakers(speakers.filter(|&n| n > 0)));
}

/// Renames a speaker in a meeting's transcript; a name another speaker
/// already has merges them.
#[tauri::command]
pub fn rename_speaker(
    state: State<'_, AppState>,
    id: String,
    from: String,
    to: String,
) -> CommandResult<()> {
    let to = to.trim();
    if to.is_empty() {
        return Err(CommandError::Other("a speaker needs a name".into()));
    }
    state.library.rename_speaker(&id, &from, to)?;
    Ok(())
}

/// Where the OpenRouter key is: "environment" (QUILL_OPENROUTER_API_KEY),
/// "keychain" (the OS secure store), "none", or "unknown".
///
/// On macOS even looking a keychain item up can show the password prompt,
/// so there it's only "keychain" once the key has been read or saved this
/// session, and "unknown" before. Windows Credential Manager never prompts,
/// so there it's always checked.
#[tauri::command]
pub async fn openrouter_key_status(app: AppHandle) -> CommandResult<&'static str> {
    if std::env::var(quill_llm::keys::OPENROUTER_ENV).is_ok_and(|k| !k.is_empty()) {
        return Ok("environment");
    }
    if lock(&app.state::<AppState>().openrouter_key).is_some() {
        return Ok("keychain");
    }
    if cfg!(target_os = "macos") {
        return Ok("unknown");
    }
    blocking(|| {
        Ok(match quill_llm::keys::openrouter_key_source() {
            Ok(Some(_)) => "keychain",
            Ok(None) => "none",
            Err(e) => {
                tracing::warn!("couldn't check for the OpenRouter key: {e}");
                "unknown"
            }
        })
    })
    .await
}

/// Saves the key to the keychain from the app, which makes the app the
/// item's owner (no access prompts for it afterwards).
#[tauri::command]
pub async fn set_openrouter_key(app: AppHandle, key: String) -> CommandResult<()> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err(CommandError::Other("the key is empty".into()));
    }
    let cached = key.clone();
    blocking(move || Ok(quill_llm::keys::set(quill_llm::keys::OPENROUTER, &key)?)).await?;
    *lock(&app.state::<AppState>().openrouter_key) = Some(cached);
    Ok(())
}

/// Removes the key from the keychain. Returns false if there was none.
#[tauri::command]
pub async fn delete_openrouter_key(app: AppHandle) -> CommandResult<bool> {
    let removed = blocking(|| Ok(quill_llm::keys::delete(quill_llm::keys::OPENROUTER)?)).await?;
    *lock(&app.state::<AppState>().openrouter_key) = None;
    Ok(removed)
}

/// Runs work that can block (keychain prompts, dialogs, file moves) off the
/// main thread.
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> CommandResult<T> + Send + 'static,
) -> CommandResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| CommandError::Other(e.to_string()))?
}

/// Starts recording into a new meeting (shared by the UI and the tray).
pub fn start(app: &AppHandle) -> CommandResult<Meeting> {
    let state = app.state::<AppState>();
    let mut recording = lock(&state.recording);
    if recording.is_some() {
        return Err(CommandError::Other("already recording".into()));
    }
    if state.moving.load(Ordering::SeqCst) {
        return Err(CommandError::Other(
            "recordings are being moved; try again in a moment".into(),
        ));
    }
    let meeting = state.library.create_meeting()?;
    let dir = state.library.meeting_dir(&meeting.id);
    match ActiveRecording::start(meeting.id.clone(), dir.clone()) {
        Ok(active) => *recording = Some(active),
        Err(e) => {
            // Don't leave an empty meeting behind.
            let _ = state.library.delete(&meeting.id);
            return Err(CommandError::Other(format!(
                "could not start recording: {e}"
            )));
        }
    }
    drop(recording);
    notify(app);
    crate::live_notes::start(app, dir);
    Ok(meeting)
}

/// Stops recording and starts processing the meeting in the background.
/// Returns the meeting id.
pub fn stop(app: &AppHandle) -> CommandResult<String> {
    let state = app.state::<AppState>();
    let active = lock(&state.recording)
        .take()
        .ok_or_else(|| CommandError::Other("not recording".into()))?;
    let id = active.meeting_id.clone();
    let title = state
        .library
        .get(&id)
        .map(|d| d.meeting.title)
        .unwrap_or_default();
    crate::live_notes::stop(app, (id.clone(), title));
    let tracks = active.stop().map_err(CommandError::Other)?;
    let dir = state.library.meeting_dir(&id);
    if let Err(e) = quill_core::meeting::write_recording_info(&dir, &tracks) {
        tracing::warn!("could not write recording info: {e}");
    }
    for t in tracks.iter().filter(|t| t.peak == 0.0) {
        // Pure silence usually means a missing permission (macOS delivers
        // zeros rather than an error).
        tracing::warn!(source = ?t.source, device = %t.device, "track is silent");
    }
    let longest = tracks
        .iter()
        .map(quill_audio::capture::TrackSummary::duration_secs)
        .fold(0.0, f64::max);
    state.library.set_duration(&id, (longest * 1000.0) as u64)?;
    notify(app);

    let steps = if lock(&state.settings).auto_summarize {
        Steps::TranscribeAndSummarize
    } else {
        Steps::TranscribeOnly
    };
    pipeline::spawn(app.clone(), id.clone(), steps);
    Ok(id)
}

/// Pauses or resumes the recording in progress.
pub fn set_paused(app: &AppHandle, paused: bool) -> CommandResult<()> {
    let state = app.state::<AppState>();
    lock(&state.recording)
        .as_mut()
        .ok_or_else(|| CommandError::Other("not recording".into()))?
        .set_paused(paused);
    tracing::info!(paused, "recording paused or resumed");
    notify(app);
    Ok(())
}

#[tauri::command]
pub fn pause_recording(app: AppHandle) -> CommandResult<()> {
    set_paused(&app, true)
}

#[tauri::command]
pub fn resume_recording(app: AppHandle) -> CommandResult<()> {
    set_paused(&app, false)
}

fn current(state: &AppState) -> RecordingState {
    let recording = lock(&state.recording);
    RecordingState {
        recording: recording.is_some(),
        paused: recording.as_ref().is_some_and(ActiveRecording::is_paused),
        meeting_id: recording.as_ref().map(|r| r.meeting_id.clone()),
        elapsed_ms: recording
            .as_ref()
            .map_or(0, |r| u64::try_from(r.elapsed().as_millis()).unwrap_or(0)),
    }
}

/// Tells the tray and the UI that recording started or stopped.
fn notify(app: &AppHandle) {
    let state = current(&app.state::<AppState>());
    tray::set_recording(app, state.recording, state.paused);
    crate::calls::recording_changed(app, state.recording);
    let _ = app.emit(RECORDING_EVENT, state);
}

/// Locks a mutex, recovering from poisoning (a panic elsewhere while it was
/// held); our guarded values are plain data that stay valid.
pub fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
