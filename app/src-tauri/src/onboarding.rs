//! First-run setup: checking that Quill can hear the microphone and the
//! system audio, and pointing the user at the right privacy settings if not.

use std::time::Duration;

use quill_audio::capture::{Source, TrackSummary};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::commands::{CommandError, CommandResult, blocking, lock};

/// Long enough to say a few words; the chime plays near the start.
const CHECK_DURATION: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, Serialize)]
pub struct AccessReport {
    pub microphone: TrackAccess,
    pub system_audio: TrackAccess,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackAccess {
    pub device: String,
    /// Anything other than exact silence arrived. Exact silence means the
    /// OS withheld the audio (macOS doesn't report a denied permission any
    /// other way).
    pub heard: bool,
    /// Loudest moment in dBFS (0 = full scale), if anything was heard.
    pub peak_db: Option<f32>,
}

/// Records a few seconds from both sources (playing a chime so the system
/// track has something to hear) and reports what came through. The first
/// run is also what makes macOS ask for both permissions.
#[tauri::command]
pub async fn check_audio_access(app: AppHandle) -> CommandResult<AccessReport> {
    blocking(move || {
        let state = app.state::<AppState>();
        if lock(&state.recording).is_some() {
            return Err(CommandError::Other(
                "can't check while a meeting is recording".into(),
            ));
        }
        let dir = state.data_dir.join("access-check");
        let result = quill_audio::check::check_access(&dir, CHECK_DURATION);
        // The test audio isn't needed afterwards; don't keep a recording of
        // the room lying around.
        let _ = std::fs::remove_dir_all(&dir);
        let tracks = result.map_err(|e| CommandError::Other(e.to_string()))?;
        let find = |source| {
            tracks
                .iter()
                .find(|t| t.source == source)
                .map(access)
                .ok_or_else(|| CommandError::Other(format!("no {source:?} track")))
        };
        Ok(AccessReport {
            microphone: find(Source::Microphone)?,
            system_audio: find(Source::SystemAudio)?,
        })
    })
    .await
}

fn access(track: &TrackSummary) -> TrackAccess {
    let heard = track.peak > 0.0;
    TrackAccess {
        device: track.device.clone(),
        heard,
        peak_db: heard.then(|| 20.0 * track.peak.log10()),
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyPane {
    Microphone,
    SystemAudio,
}

/// Opens the OS privacy settings where the user can grant access.
#[tauri::command]
pub fn open_privacy_settings(pane: PrivacyPane) -> CommandResult<()> {
    let url = if cfg!(target_os = "macos") {
        match pane {
            PrivacyPane::Microphone => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            }
            // "Screen & System Audio Recording", which lists the
            // "System Audio Recording Only" apps.
            PrivacyPane::SystemAudio => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
            }
        }
    } else {
        // Windows needs no permission for loopback capture.
        "ms-settings:privacy-microphone"
    };
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| CommandError::Other(e.to_string()))
}

/// Marks the first-run screen as done.
#[tauri::command]
pub fn finish_onboarding(state: tauri::State<'_, AppState>) -> CommandResult<()> {
    let mut settings = lock(&state.settings).clone();
    settings.onboarded = true;
    settings.save(&state.data_dir)?;
    *lock(&state.settings) = settings;
    Ok(())
}
