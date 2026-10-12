//! After a recording: transcribe → (optionally) summarize → save, reporting
//! progress to the UI as `meeting-progress` events.

use std::sync::Arc;

use quill_core::meeting;
use quill_core::settings::Settings;
use quill_core::speakers;
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
    Identifying,
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

impl LoadedTranscriber {
    /// The Whisper model's short name, e.g. `turbo`.
    pub fn model_name(&self) -> &str {
        &self.model
    }
}

/// What to do after transcribing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Steps {
    TranscribeOnly,
    TranscribeAndSummarize,
    SummarizeOnly,
    /// Split the call's speakers again, into this many if given.
    Speakers(Option<usize>),
}

/// Processes a meeting in the background. Returns immediately; progress
/// arrives as events. A meeting already being processed is skipped.
pub fn spawn(app: AppHandle, meeting_id: String, steps: Steps) {
    let state = app.state::<AppState>();
    if !lock(&state.processing).insert(meeting_id.clone()) {
        tracing::info!(meeting_id, "already processing");
        return;
    }
    // Checked after registering, so a move starting now sees us (see
    // `storage::relocate`).
    if state.moving.load(std::sync::atomic::Ordering::SeqCst) {
        lock(&state.processing).remove(&meeting_id);
        let message = "recordings are being moved; try again in a moment".to_string();
        emit(&app, &meeting_id, Stage::Failed, None, Some(message));
        return;
    }
    tauri::async_runtime::spawn(async move {
        let result = run(&app, &meeting_id, steps).await;
        match result {
            Ok(()) => emit(&app, &meeting_id, Stage::Done, None, None),
            Err(e) => {
                tracing::error!(meeting_id, "processing failed: {e}");
                emit(&app, &meeting_id, Stage::Failed, None, Some(e));
            }
        }
        // Shrink the audio once it's been transcribed. Still registered as
        // processing meanwhile, so the recordings can't be moved under it.
        if matches!(steps, Steps::TranscribeOnly | Steps::TranscribeAndSummarize) {
            let app2 = app.clone();
            let id = meeting_id.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || compact(&app2, &id)).await;
        }
        lock(&app.state::<AppState>().processing).remove(&meeting_id);
    });
}

/// Replaces a meeting's WAVs with FLAC (see `meeting::compact_recording`).
/// Failures are logged: the WAVs are kept and the next startup retries.
fn compact(app: &AppHandle, id: &str) {
    let dir = app.state::<AppState>().library.meeting_dir(id);
    let started = std::time::Instant::now();
    match meeting::compact_recording(&dir) {
        Ok(done) if done.tracks > 0 => tracing::info!(
            meeting_id = id,
            tracks = done.tracks,
            mb_before = done.bytes_before / 1_000_000,
            mb_after = done.bytes_after / 1_000_000,
            ms = started.elapsed().as_millis(),
            "compacted recording"
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!(meeting_id = id, "couldn't compact recording: {e}"),
    }
}

/// Compacts recordings made before compaction existed, or whose compaction
/// failed. Runs once at startup in the background; skips meetings that are
/// recording or being processed.
pub fn compact_existing(app: &AppHandle) {
    let state = app.state::<AppState>();
    let meetings = match state.library.list() {
        Ok(meetings) => meetings,
        Err(e) => {
            tracing::warn!("couldn't list meetings to compact: {e}");
            return;
        }
    };
    for m in meetings {
        if !meeting::has_wav(&state.library.meeting_dir(&m.id)) {
            continue;
        }
        let recording = lock(&state.recording)
            .as_ref()
            .is_some_and(|r| r.meeting_id == m.id);
        // Registering as processing keeps the pipeline and a recordings
        // move away from this folder meanwhile.
        if recording || !lock(&state.processing).insert(m.id.clone()) {
            continue;
        }
        if !state.moving.load(std::sync::atomic::Ordering::SeqCst) {
            compact(app, &m.id);
        }
        lock(&state.processing).remove(&m.id);
    }
}

async fn run(app: &AppHandle, id: &str, steps: Steps) -> Result<(), String> {
    let state = app.state::<AppState>();
    let settings = lock(&state.settings).clone();

    if let Steps::Speakers(count) = steps {
        let app2 = app.clone();
        let id2 = id.to_string();
        let segments = tauri::async_runtime::spawn_blocking(move || {
            let state = app2.state::<AppState>();
            let dir = state.library.meeting_dir(&id2);
            let track = speakers::load_track_transcript(&dir)
                .or_else(|| two_track_from_library(&app2, &id2))
                .ok_or("this meeting's original transcript is missing; transcribe it again")?;
            identify_speakers(&app2, &id2, &track, count)
        })
        .await
        .map_err(|e| e.to_string())??;
        state
            .library
            .save_transcript(id, &segments)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    if steps != Steps::SummarizeOnly {
        emit(app, id, Stage::Transcribing, None, None);
        let app2 = app.clone();
        let id2 = id.to_string();
        let settings2 = settings.clone();
        // Whisper is CPU/GPU-bound and blocking: keep it off the async
        // runtime's worker threads.
        let segments = tauri::async_runtime::spawn_blocking(move || {
            let transcriber = transcriber(&app2, Some(&id2), &settings2)?;
            let state = app2.state::<AppState>();
            let dir = state.library.meeting_dir(&id2);
            let opts = TranscribeOptions {
                language: settings2.language.clone(),
                ..Default::default()
            };
            let segments = meeting::transcribe_recording(&dir, transcriber.as_ref(), &opts, true)
                .map_err(|e| e.to_string())?;
            // Kept as a file, like `quill-cli transcribe <dir>` does; speakers
            // are split again from it.
            if let Err(e) = speakers::save_track_transcript(&dir, &segments) {
                tracing::warn!(meeting_id = id2, "couldn't save transcript.json: {e}");
            }
            if !settings2.identify_speakers {
                return Ok::<_, String>(segments);
            }
            // Telling speakers apart is a bonus: if it fails, keep "Others".
            match identify_speakers(&app2, &id2, &segments, None) {
                Ok(labelled) => Ok(labelled),
                Err(e) => {
                    tracing::warn!(meeting_id = id2, "couldn't tell speakers apart: {e}");
                    Ok(segments)
                }
            }
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
    let (provider, model) =
        llm_provider(app, &settings.summary_model, settings.zdr).map_err(|e| e.to_string())?;
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

/// Relabels the "Others" lines of a two-track transcript by voice. The
/// models run only the first time for a meeting; later calls (a different
/// speaker count) reuse the cached analysis. Blocking.
fn identify_speakers(
    app: &AppHandle,
    id: &str,
    track: &[quill_stt::Segment],
    count: Option<usize>,
) -> Result<Vec<quill_stt::Segment>, String> {
    if !track
        .iter()
        .any(|s| s.speaker.as_deref() == Some(meeting::OTHERS))
    {
        return Ok(track.to_vec());
    }
    let dir = app.state::<AppState>().library.meeting_dir(id);
    let started = std::time::Instant::now();
    let analysis = match speakers::cached_analysis(&dir, speakers::VOICE_MODEL) {
        Some(analysis) => analysis,
        None => {
            emit(app, id, Stage::Identifying, None, None);
            for model in speakers::required_models() {
                ensure_downloaded(app, Some(id), model)?;
            }
            emit(app, id, Stage::Identifying, None, None);
            let models_dir = quill_core::paths::models_dir().ok_or("no home directory")?;
            let diarizer = speakers::diarizer(&models_dir).map_err(|e| e.to_string())?;
            match speakers::analyze(&dir, speakers::VOICE_MODEL, &diarizer)
                .map_err(|e| e.to_string())?
            {
                Some(analysis) => analysis,
                None => return Ok(track.to_vec()),
            }
        }
    };
    let (labelled, found) = speakers::label(track, &analysis, count);
    tracing::info!(
        meeting_id = id,
        speakers = found,
        requested = count,
        ms = started.elapsed().as_millis(),
        "told speakers apart"
    );
    Ok(labelled)
}

/// For meetings transcribed before `transcript.json` was kept as the
/// two-track original: the stored transcript, if it's still only "Me" and
/// "Others".
fn two_track_from_library(app: &AppHandle, id: &str) -> Option<Vec<quill_stt::Segment>> {
    let segments = app.state::<AppState>().library.get(id).ok()?.segments;
    segments
        .iter()
        .all(|s| {
            matches!(
                s.speaker.as_deref(),
                Some(meeting::ME | meeting::OTHERS) | None
            )
        })
        .then_some(segments)
}

/// Loads the Whisper model in the background at startup, if it's already
/// downloaded, so the first dictation doesn't wait for it.
pub fn preload(app: &AppHandle) {
    let settings = lock(&app.state::<AppState>().settings).clone();
    let Some(dir) = quill_core::paths::models_dir() else {
        return;
    };
    let downloaded = models::find(&settings.stt_model).is_some_and(|m| m.is_downloaded(&dir))
        && (!settings.vad || models::VAD_MODEL.is_downloaded(&dir));
    if downloaded {
        let started = std::time::Instant::now();
        match transcriber(app, None, &settings) {
            Ok(_) => tracing::info!(ms = started.elapsed().as_millis(), "Whisper model loaded"),
            Err(e) => tracing::warn!("couldn't preload the Whisper model: {e}"),
        }
    }
}

/// An LLM provider for `spec` (OpenRouter id or `ollama:<model>`), with
/// the OpenRouter key read from the keychain at most once per launch.
pub fn llm_provider(
    app: &AppHandle,
    spec: &str,
    zdr: bool,
) -> Result<(Box<dyn quill_llm::LlmProvider>, String), quill_llm::LlmError> {
    let privacy = Privacy {
        deny_data_collection: true,
        zero_data_retention: zdr,
    };
    quill_llm::provider_for_with_key(spec, privacy, || {
        let state = app.state::<AppState>();
        let mut cached = lock(&state.openrouter_key);
        if let Some(key) = cached.as_ref() {
            return Ok(key.clone());
        }
        let key = quill_llm::keys::openrouter_key()?;
        *cached = Some(key.clone());
        Ok(key)
    })
}

/// Returns the Whisper model from settings, loading (and first downloading)
/// it if it isn't the one already in memory. Download progress goes to
/// meeting `id`'s progress events, if given. Meetings and dictation share
/// the one loaded model; it's safe to use from several threads at once.
pub fn transcriber(
    app: &AppHandle,
    id: Option<&str>,
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
    if let Some(id) = id {
        emit(app, id, Stage::Transcribing, None, None);
    }
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

/// Downloads `model` if needed (see `models::ensure`), showing progress on
/// meeting `id` too, if given.
fn ensure_downloaded(
    app: &AppHandle,
    id: Option<&str>,
    model: &'static ModelInfo,
) -> Result<std::path::PathBuf, String> {
    let message = Some(if speakers::required_models().contains(&model) {
        "Downloading the voice models".to_string()
    } else {
        format!("Downloading the {} speech model", model.name)
    });
    let mut last = -1.0;
    crate::models::ensure(app, model, |done, total| {
        let fraction = done as f64 / total.max(1) as f64;
        // Throttle: one event per percent.
        if fraction - last >= 0.01 {
            last = fraction;
            if let Some(id) = id {
                emit(app, id, Stage::Downloading, Some(fraction), message.clone());
            }
        }
    })
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
