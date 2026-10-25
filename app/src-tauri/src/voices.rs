//! Known voices: the people the user named on calls, recognised by voice in
//! later meetings. The logic and storage live in `quill_core::voices`; these
//! are the commands the transcript and Settings call.

use quill_core::voices::{KnownVoiceInfo, MeetingVoice};
use tauri::State;

use crate::AppState;
use crate::commands::{CommandResult, lock};

/// The people on a meeting's call that Quill has voices for: which were
/// named automatically, and which it remembers.
#[tauri::command]
pub fn meeting_voices(state: State<'_, AppState>, id: String) -> CommandResult<Vec<MeetingVoice>> {
    Ok(state.library.meeting_voices(&id)?)
}

/// The user says an automatically given name is right; Quill learns this
/// meeting's voice for them (if recognising voices is on).
#[tauri::command]
pub fn confirm_speaker(state: State<'_, AppState>, id: String, label: String) -> CommandResult<()> {
    let remember = lock(&state.settings).recognize_voices;
    Ok(state.library.confirm_speaker(&id, &label, remember)?)
}

/// Every known voice, for Settings.
#[tauri::command]
pub fn known_voices(state: State<'_, AppState>) -> CommandResult<Vec<KnownVoiceInfo>> {
    Ok(state.library.known_voices()?)
}

/// Forgets one known voice.
#[tauri::command]
pub fn delete_known_voice(state: State<'_, AppState>, id: i64) -> CommandResult<()> {
    state.library.delete_known_voice(id)?;
    tracing::info!("forgot a known voice");
    Ok(())
}

/// Forgets every known voice; returns how many there were.
#[tauri::command]
pub fn forget_known_voices(state: State<'_, AppState>) -> CommandResult<usize> {
    let n = state.library.forget_known_voices()?;
    tracing::info!(voices = n, "forgot all known voices");
    Ok(n)
}
