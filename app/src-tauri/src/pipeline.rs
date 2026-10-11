//! After a recording: transcribe → (optionally) summarize → save, reporting
//! progress to the UI as `meeting-progress` events.

use std::sync::Arc;

use quill_core::meeting;
use quill_core::settings::Settings;
use quill_core::summary;
use quill_llm::Privacy;
use quill_stt::models::{self, ModelInfo};
use quill_stt::{TranscribeOptions, WhisperConfig, WhisperTranscriber};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;
use crate::commands::lock;

pub const PROGRESS_EVENT: &str = "meeting-progress";

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Downloading,
    Transcribing,
    Summarizing,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub meeting_id: String,
    pub stage: Stage,
    /// Download progress, 0.0–1.0.
    pub fraction: Option<f64>,
    pub message: Option<String>,
}

/// The loaded Whisper model, kept between meetings: loading takes seconds.
pub struct LoadedTranscriber {
    model: String,
    vad: bool,
    transcriber: Arc<WhisperTranscriber>,
}

/// What to do after transcribing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Steps {
    TranscribeOnly,
    TranscribeAndSummarize,
    SummarizeOnly,
}

/// Processes a meeting in the background. Returns immediately; progress
/// arrives as events. A meeting already being processed is skipped.
pub fn spawn(app: AppHandle, meeting_id: String, steps: Steps) {
    let state = app.state::<AppState>();
    if !lock(&state.processing).insert(meeting_id.clone()) {
        tracing::info!(meeting_id, "already processing");
        return;
    }
    tauri::async_runtime::spawn(async move {
        let result = run(&app, &meeting_id, steps).await;
        let state = app.state::<AppState>();
        lock(&state.processing).remove(&meeting_id);
        match result {
            Ok(()) => emit(&app, &meeting_id, Stage::Done, None, None),
            Err(e) => {
                tracing::error!(meeting_id, "processing failed: {e}");
                emit(&app, &meeting_id, Stage::Failed, None, Some(e));
            }
        }
    });
}

async fn run(app: &AppHandle, id: &str, steps: Steps) -> Result<(), String> {
    let state = app.state::<AppState>();
    let settings = lock(&state.settings).clone();

    if steps != Steps::SummarizeOnly {
        emit(app, id, Stage::Transcribing, None, None);
        let app2 = app.clone();
        let id2 = id.to_string();
        let settings2 = settings.clone();
        // Whisper is CPU/GPU-bound and blocking: keep it off the async
        // runtime's worker threads.
        let segments = tauri::async_runtime::spawn_blocking(move || {
            let transcriber = transcriber(&app2, &id2, &settings2)?;
            let state = app2.state::<AppState>();
            let dir = state.library.meeting_dir(&id2);
            let opts = TranscribeOptions {
                language: settings2.language.clone(),
                initial_prompt: None,
            };
            let segments = meeting::transcribe_recording(&dir, transcriber.as_ref(), &opts)
                .map_err(|e| e.to_string())?;
            // Also save as a file, like `quill-cli transcribe <dir>` does.
            if let Ok(json) = serde_json::to_string_pretty(&segments) {
                let _ = std::fs::write(dir.join("transcript.json"), json);
            }
            Ok::<_, String>(segments)
        })
        .await
        .map_err(|e| e.to_string())??;
        state
            .library
            .save_transcript(id, &segments)
            .map_err(|e| e.to_string())?;
    }

    if steps == Steps::TranscribeOnly {
        return Ok(());
    }
    let detail = state.library.get(id).map_err(|e| e.to_string())?;
    if detail.segments.is_empty() {
        // Nothing was said; nothing to summarize.
        return Ok(());
    }
    emit(app, id, Stage::Summarizing, None, None);
    let privacy = Privacy {
        deny_data_collection: true,
        zero_data_retention: settings.zdr,
    };
    let (provider, model) =
        quill_llm::provider_for_with_key(&settings.summary_model, privacy, || {
            let mut cached = lock(&state.openrouter_key);
            if let Some(key) = cached.as_ref() {
                return Ok(key.clone());
            }
            let key = quill_llm::keys::openrouter_key()?;
            *cached = Some(key.clone());
            Ok(key)
        })
        .map_err(|e| e.to_string())?;
    let date = meeting_date(&detail.meeting.started_at);
    let summary = summary::summarize(
        provider.as_ref(),
        &model,
        &quill_llm::prompts::SUMMARY,
        &detail.segments,
        &date,
    )
    .await
    .map_err(|e| e.to_string())?;
    state
        .library
        .save_summary(id, &summary)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Returns the Whisper model from settings, loading (and first downloading)
/// it if it isn't the one already in memory.
fn transcriber(
    app: &AppHandle,
    id: &str,
    settings: &Settings,
) -> Result<Arc<WhisperTranscriber>, String> {
    let state = app.state::<AppState>();
    let mut cached = lock(&state.transcriber);
    if let Some(t) = cached.as_ref()
        && t.model == settings.stt_model
        && t.vad == settings.vad
    {
        return Ok(Arc::clone(&t.transcriber));
    }

    let model = models::find(&settings.stt_model)
        .ok_or_else(|| format!("unknown Whisper model `{}`", settings.stt_model))?;
    let model_path = ensure_downloaded(app, id, model)?;
    let vad_model = if settings.vad {
        Some(ensure_downloaded(app, id, &models::VAD_MODEL)?)
    } else {
        None
    };
    // Back to "transcribing" after any download.
    emit(app, id, Stage::Transcribing, None, None);
    let config = WhisperConfig {
        vad_model,
        ..WhisperConfig::default()
    };
    let transcriber =
        Arc::new(WhisperTranscriber::load(&model_path, &config).map_err(|e| e.to_string())?);
    *cached = Some(LoadedTranscriber {
        model: settings.stt_model.clone(),
        vad: settings.vad,
        transcriber: Arc::clone(&transcriber),
    });
    Ok(transcriber)
}

fn ensure_downloaded(
    app: &AppHandle,
    id: &str,
    model: &ModelInfo,
) -> Result<std::path::PathBuf, String> {
    let dir = quill_core::paths::models_dir().ok_or("no home directory")?;
    if model.is_downloaded(&dir) {
        return Ok(model.path_in(&dir));
    }
    let message = Some(format!("Downloading {} model", model.name));
    let mut last = -1.0;
    models::download(model, &dir, |done, total| {
        let fraction = done as f64 / total.max(1) as f64;
        // Throttle: one event per percent.
        if fraction - last >= 0.01 {
            last = fraction;
            emit(app, id, Stage::Downloading, Some(fraction), message.clone());
        }
    })
    .map_err(|e| e.to_string())
}

fn emit(app: &AppHandle, id: &str, stage: Stage, fraction: Option<f64>, message: Option<String>) {
    let progress = Progress {
        meeting_id: id.to_string(),
        stage,
        fraction,
        message,
    };
    if let Err(e) = app.emit(PROGRESS_EVENT, progress) {
        tracing::warn!("could not emit progress: {e}");
    }
}

/// The meeting's local date, e.g. `2026-10-11`, for the summary title.
fn meeting_date(started_at: &str) -> String {
    started_at
        .parse::<jiff::Timestamp>()
        .map(|t| t.to_zoned(jiff::tz::TimeZone::system()).date().to_string())
        .unwrap_or_else(|_| jiff::Zoned::now().date().to_string())
}
