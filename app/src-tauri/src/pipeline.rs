//! After a recording: transcribe → (optionally) summarize → save, reporting
//! progress to the UI as `meeting-progress` events. Each run is a [`Job`]
//! following its steps with `quill_core::processing`: how far each has got,
//! time left, the lines just heard, and what came out.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use quill_core::meeting::{self, Phase, Track};
use quill_core::processing::{Outcome, Report, StepKind, Tracker};
use quill_core::settings::Settings;
use quill_core::speakers;
use quill_core::summary;
use quill_llm::Privacy;
use quill_stt::models::{self, ModelInfo};
use quill_stt::{GpuChoice, TranscribeOptions, Transcriber, WhisperConfig};
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
    /// How far through the stage, 0.0–1.0, when known.
    pub fraction: Option<f64>,
    pub message: Option<String>,
    /// The whole write-up: steps, time left, lines heard, and at the end
    /// what came out. Absent for a failure before it started.
    pub report: Option<Report>,
    /// The meeting's title, for notifications.
    pub title: Option<String>,
}

/// How often a running job sends progress, at most (step changes go at once).
const EMIT_EVERY: Duration = Duration::from_millis(150);

/// One write-up in progress.
pub struct Job {
    app: AppHandle,
    id: String,
    state: Mutex<JobState>,
}

struct JobState {
    tracker: Tracker,
    /// A model download under way: how far, and what.
    download: Option<(f64, String)>,
    last_emit: Option<Instant>,
}

impl Job {
    fn new(app: &AppHandle, id: &str, plan: &[StepKind], audio_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            app: app.clone(),
            id: id.to_string(),
            state: Mutex::new(JobState {
                tracker: Tracker::new(plan, audio_ms, Instant::now()),
                download: None,
                last_emit: None,
            }),
        })
    }

    /// Changes the tracker and sends progress: at once when `now_please`
    /// (a step began or ended), else at most every [`EMIT_EVERY`].
    fn update(&self, now_please: bool, change: impl FnOnce(&mut Tracker, Instant)) {
        let now = Instant::now();
        let progress = {
            let mut state = lock(&self.state);
            change(&mut state.tracker, now);
            if !now_please
                && state
                    .last_emit
                    .is_some_and(|t| now.duration_since(t) < EMIT_EVERY)
            {
                return;
            }
            state.last_emit = Some(now);
            self.progress(&state, now)
        };
        send(&self.app, progress);
    }

    fn progress(&self, state: &JobState, now: Instant) -> Progress {
        let report = state.tracker.report(now);
        let active = state.tracker.active();
        let (stage, fraction, message) = match &state.download {
            Some((fraction, message)) => {
                (Stage::Downloading, Some(*fraction), Some(message.clone()))
            }
            None => (
                match active.map(|s| s.kind) {
                    Some(StepKind::Speakers) => Stage::Identifying,
                    Some(StepKind::Notes) => Stage::Summarizing,
                    _ => Stage::Transcribing,
                },
                active.and_then(|s| s.fraction),
                None,
            ),
        };
        Progress {
            meeting_id: self.id.clone(),
            stage,
            fraction,
            message,
            report: Some(report),
            title: None,
        }
    }

    fn start(&self, kind: StepKind) {
        self.update(true, |t, now| t.start(kind, now));
    }

    fn finish(&self, kind: StepKind) {
        self.update(true, |t, now| t.finish(kind, now));
    }

    fn skip(&self, kind: StepKind) {
        self.update(true, |t, _| t.skip(kind));
    }

    /// A model download's progress (`None` once it's done).
    fn download(&self, progress: Option<(f64, String)>) {
        let done = progress.is_none();
        lock(&self.state).download = progress;
        self.update(done, |_, _| {});
    }

    fn phase(&self, phase: Phase) {
        let kind = |track| match track {
            Track::Mic => StepKind::You,
            Track::Call => StepKind::Call,
        };
        match phase {
            Phase::Ready { mic, call } => self.update(true, |t, now| {
                t.finish(StepKind::Prepare, now);
                if !mic {
                    t.skip(StepKind::You);
                }
                if !call {
                    t.skip(StepKind::Call);
                }
                t.start(if mic { StepKind::You } else { StepKind::Call }, now);
            }),
            Phase::Percent(track, p) => self.update(false, |t, now| {
                t.start(kind(track), now);
                t.progress(kind(track), f64::from(p) / 100.0, now);
            }),
            Phase::Line(track, text) => self.update(false, |t, now| {
                t.start(kind(track), now);
                t.heard(track == Track::Mic, &text);
            }),
            // Straight on to the call track (if there is one).
            Phase::Done(track) => self.update(true, |t, now| {
                t.finish(kind(track), now);
                t.start(StepKind::Call, now);
            }),
        }
    }

    /// Finished: what came out, and the last progress, with the title.
    fn done(&self, outcome: impl FnOnce(u64) -> Outcome) -> Progress {
        let now = Instant::now();
        let mut state = lock(&self.state);
        state.download = None;
        state.tracker.done(outcome, now);
        let mut progress = self.progress(&state, now);
        progress.stage = Stage::Done;
        progress.fraction = None;
        progress
    }

    fn failed(&self, message: String) -> Progress {
        let now = Instant::now();
        let mut state = lock(&self.state);
        state.download = None;
        state.tracker.fail(now);
        let mut progress = self.progress(&state, now);
        progress.stage = Stage::Failed;
        progress.fraction = None;
        progress.message = Some(message);
        progress
    }
}

/// A loaded speech model, kept between meetings and dictations: loading
/// takes seconds.
pub struct LoadedTranscriber {
    model: String,
    vad: bool,
    gpu: GpuChoice,
    transcriber: Arc<dyn Transcriber>,
}

impl LoadedTranscriber {
    /// The speech model's short name, e.g. `turbo` or `parakeet-v2`.
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
        send(
            &app,
            Progress {
                meeting_id,
                stage: Stage::Failed,
                fraction: None,
                message: Some(message),
                report: None,
                title: None,
            },
        );
        return;
    }
    let settings = lock(&state.settings).clone();
    let plan = plan(steps, &settings);
    let (audio_ms, title) = state
        .library
        .get(&meeting_id)
        .map(|d| (d.meeting.duration_ms, d.meeting.title))
        .unwrap_or_default();
    let job = Job::new(&app, &meeting_id, &plan, audio_ms);
    // Shown straight away, before the first step has anything to say.
    job.start(plan[0]);
    tauri::async_runtime::spawn(async move {
        let result = run(&app, &meeting_id, steps, &job).await;
        let mut progress = match result {
            Ok(()) => {
                let library = &app.state::<AppState>().library;
                let detail = library.get(&meeting_id).ok();
                let segments = detail
                    .as_ref()
                    .map(|d| d.segments.as_slice())
                    .unwrap_or_default();
                let notes = detail
                    .as_ref()
                    .and_then(|d| d.summaries.first())
                    .map(|s| s.markdown.as_str());
                let outcome =
                    job.done(|took| quill_core::processing::outcome(segments, notes, took));
                if let Some(o) = outcome.report.as_ref().and_then(|r| r.outcome.as_ref()) {
                    tracing::info!(
                        meeting_id,
                        ms = o.took_ms,
                        words = o.words,
                        people = o.people,
                        action_items = o.action_items,
                        "wrote up the meeting"
                    );
                }
                outcome
            }
            Err(e) => {
                tracing::error!(meeting_id, "processing failed: {e}");
                job.failed(e)
            }
        };
        progress.title = Some(title);
        send(&app, progress.clone());
        crate::wrapup::finished(&app, &progress);
        // Shrink the audio once it's been transcribed. Still registered as
        // processing meanwhile, so the recordings can't be moved under it.
        if matches!(steps, Steps::TranscribeOnly | Steps::TranscribeAndSummarize) {
            let app2 = app.clone();
            let id = meeting_id.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || compact(&app2, &id)).await;
        }
        lock(&app.state::<AppState>().processing).remove(&meeting_id);
        // A new transcript to index for search by meaning.
        crate::semantic::wake(&app);
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

/// The steps a run takes, in order.
fn plan(steps: Steps, settings: &Settings) -> Vec<StepKind> {
    let mut plan = Vec::new();
    match steps {
        Steps::Speakers(_) => plan.push(StepKind::Speakers),
        Steps::SummarizeOnly => plan.push(StepKind::Notes),
        Steps::TranscribeOnly | Steps::TranscribeAndSummarize => {
            plan.extend([StepKind::Prepare, StepKind::You, StepKind::Call]);
            if settings.identify_speakers {
                plan.push(StepKind::Speakers);
            }
            if steps == Steps::TranscribeAndSummarize {
                plan.push(StepKind::Notes);
            }
        }
    }
    plan
}

async fn run(app: &AppHandle, id: &str, steps: Steps, job: &Arc<Job>) -> Result<(), String> {
    let state = app.state::<AppState>();
    let settings = lock(&state.settings).clone();

    if let Steps::Speakers(count) = steps {
        let app2 = app.clone();
        let id2 = id.to_string();
        let job2 = Arc::clone(job);
        let segments = tauri::async_runtime::spawn_blocking(move || {
            let state = app2.state::<AppState>();
            let dir = state.library.meeting_dir(&id2);
            let track = speakers::load_track_transcript(&dir)
                .or_else(|| two_track_from_library(&app2, &id2))
                .ok_or("this meeting's original transcript is missing; transcribe it again")?;
            identify_speakers(&app2, &id2, &track, count, &job2)
        })
        .await
        .map_err(|e| e.to_string())??;
        save_labelled(app, id, &segments)?;
        return Ok(());
    }

    if steps != Steps::SummarizeOnly {
        let app2 = app.clone();
        let id2 = id.to_string();
        let settings2 = settings.clone();
        let job2 = Arc::clone(job);
        // Whisper is CPU/GPU-bound and blocking: keep it off the async
        // runtime's worker threads.
        let segments = tauri::async_runtime::spawn_blocking(move || {
            let transcriber = transcriber(&app2, Some(&job2), &settings2)?;
            let state = app2.state::<AppState>();
            let dir = state.library.meeting_dir(&id2);
            let opts = TranscribeOptions {
                language: settings2.language.clone(),
                ..Default::default()
            };
            let report_job = Arc::clone(&job2);
            let report: Arc<dyn Fn(Phase) + Send + Sync> =
                Arc::new(move |phase| report_job.phase(phase));
            let segments = meeting::transcribe_recording_with(
                &dir,
                transcriber.as_ref(),
                &opts,
                true,
                Some(report),
            )
            .map_err(|e| e.to_string())?;
            // Kept as a file, like `quill-cli transcribe <dir>` does; speakers
            // are split again from it.
            if let Err(e) = speakers::save_track_transcript(&dir, &segments) {
                tracing::warn!(meeting_id = id2, "couldn't save transcript.json: {e}");
            }
            if !settings2.identify_speakers {
                return Ok::<_, String>(speakers::Labelled::unchanged(&segments));
            }
            // Telling speakers apart is a bonus: if it fails, keep "Others".
            match identify_speakers(&app2, &id2, &segments, None, &job2) {
                Ok(labelled) => Ok(labelled),
                Err(e) => {
                    tracing::warn!(meeting_id = id2, "couldn't tell speakers apart: {e}");
                    job2.skip(StepKind::Speakers);
                    Ok(speakers::Labelled::unchanged(&segments))
                }
            }
        })
        .await
        .map_err(|e| e.to_string())??;
        save_labelled(app, id, &segments)?;
    }

    if steps == Steps::TranscribeOnly {
        return Ok(());
    }
    let detail = state.library.get(id).map_err(|e| e.to_string())?;
    if detail.segments.is_empty() {
        // Nothing was said; nothing to summarize.
        job.skip(StepKind::Notes);
        return Ok(());
    }
    job.start(StepKind::Notes);
    let (provider, model) = notes_provider(app, &settings)?;
    let date = meeting_date(&detail.meeting.started_at);
    let summary = summary::summarize(
        provider.as_ref(),
        &model,
        &quill_llm::prompts::SUMMARY,
        &detail.segments,
        &date,
        &settings.dictation.vocabulary,
    )
    .await
    .map_err(|e| e.to_string())?;
    state
        .library
        .save_summary(id, &summary)
        .map_err(|e| e.to_string())?;
    job.finish(StepKind::Notes);
    Ok(())
}

/// Saves a transcript, and the voices of the people on the call for
/// recognising them later (`quill_core::voices`).
fn save_labelled(app: &AppHandle, id: &str, labelled: &speakers::Labelled) -> Result<(), String> {
    let library = &app.state::<AppState>().library;
    library
        .save_transcript(id, &labelled.segments)
        .map_err(|e| e.to_string())?;
    // Only costs recognising this meeting's people later; not worth failing
    // the transcript over.
    if let Err(e) = library.save_meeting_voices(id, speakers::VOICE_MODEL, &labelled.voices) {
        tracing::warn!(meeting_id = id, "couldn't save the call's voices: {e}");
    }
    Ok(())
}

/// Relabels the "Others" lines of a two-track transcript by voice, naming
/// people whose voices Quill knows (if that's on). The models run only the
/// first time for a meeting; later calls (a different speaker count) reuse
/// the cached analysis. Blocking.
fn identify_speakers(
    app: &AppHandle,
    id: &str,
    track: &[quill_stt::Segment],
    count: Option<usize>,
    job: &Arc<Job>,
) -> Result<speakers::Labelled, String> {
    if !track
        .iter()
        .any(|s| s.speaker.as_deref() == Some(meeting::OTHERS))
    {
        // Only you spoke: nobody to tell apart.
        job.skip(StepKind::Speakers);
        return Ok(speakers::Labelled::unchanged(track));
    }
    job.start(StepKind::Speakers);
    let dir = app.state::<AppState>().library.meeting_dir(id);
    let started = std::time::Instant::now();
    let analysis = match speakers::cached_analysis(&dir, speakers::VOICE_MODEL) {
        Some(analysis) => analysis,
        None => {
            for model in speakers::required_models() {
                ensure_downloaded(app, Some(job), model)?;
            }
            let models_dir = quill_core::paths::models_dir().ok_or("no home directory")?;
            let diarizer = speakers::diarizer(&models_dir).map_err(|e| e.to_string())?;
            match speakers::analyze(&dir, speakers::VOICE_MODEL, &diarizer)
                .map_err(|e| e.to_string())?
            {
                Some(analysis) => analysis,
                None => {
                    job.skip(StepKind::Speakers);
                    return Ok(speakers::Labelled::unchanged(track));
                }
            }
        }
    };
    let state = app.state::<AppState>();
    let recognize = lock(&state.settings).recognize_voices;
    let known = if recognize {
        state
            .library
            .known_voices_to_match(speakers::VOICE_MODEL)
            .unwrap_or_else(|e| {
                tracing::warn!("couldn't read known voices: {e}");
                Vec::new()
            })
    } else {
        Vec::new()
    };
    let labelled = speakers::label_and_recognize(
        track,
        &analysis,
        count,
        &known,
        &speakers::MatchOptions::default(),
    );
    job.finish(StepKind::Speakers);
    // Counts only: names and voices are never logged.
    tracing::info!(
        meeting_id = id,
        speakers = labelled.speakers,
        requested = count,
        known_voices = known.len(),
        recognized = labelled.recognized(),
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

/// Loads dictation's speech model in the background at startup, if it's
/// already downloaded, so the first dictation doesn't wait for it.
/// (Meetings load theirs when they need it, if it's another.)
pub fn preload(app: &AppHandle) {
    let settings = lock(&app.state::<AppState>().settings).clone();
    let Some(dir) = quill_core::paths::models_dir() else {
        return;
    };
    let downloaded = models::speech_files(settings.dictation_model())
        .is_some_and(|files| files.iter().all(|m| m.is_downloaded(&dir)))
        && (!settings.vad || models::VAD_MODEL.is_downloaded(&dir));
    if downloaded {
        let started = std::time::Instant::now();
        match dictation_transcriber(app, &settings) {
            Ok(_) => tracing::info!(
                ms = started.elapsed().as_millis(),
                model = settings.dictation_model(),
                "speech model loaded"
            ),
            Err(e) => tracing::warn!("couldn't preload the speech model: {e}"),
        }
    }
}

/// The summary model from settings, for meeting notes and daily summaries,
/// with an error that says where to fix it.
pub fn notes_provider(
    app: &AppHandle,
    settings: &Settings,
) -> Result<(Box<dyn quill_llm::LlmProvider>, String), String> {
    llm_provider(app, &settings.summary_model, settings.zdr).map_err(|e| match e {
        // The library's message suggests the CLI; point to Settings.
        quill_llm::LlmError::MissingKey(_) => "No OpenRouter key yet. Add one in Settings → \
             OpenRouter, or choose a local Ollama model in Settings → Models."
            .to_string(),
        e => e.to_string(),
    })
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

/// Returns the meetings' speech model from settings, loading (and first
/// downloading) it if it isn't in memory. Download progress goes to
/// `job`'s progress events, if given. Dictation shares it unless it has a
/// model of its own; it's safe to use from several threads at once.
pub fn transcriber(
    app: &AppHandle,
    job: Option<&Arc<Job>>,
    settings: &Settings,
) -> Result<Arc<dyn Transcriber>, String> {
    load_speech_model(app, job, settings, &settings.stt_model)
}

/// Returns dictation's speech model, as [`transcriber`] does the meetings'.
pub fn dictation_transcriber(
    app: &AppHandle,
    settings: &Settings,
) -> Result<Arc<dyn Transcriber>, String> {
    load_speech_model(app, None, settings, settings.dictation_model())
}

/// Speech model `name`, from memory or loaded (and first downloaded).
/// Keeps at most the two the settings use (meetings' and dictation's)
/// loaded, so switching between a meeting and a dictation never reloads.
fn load_speech_model(
    app: &AppHandle,
    job: Option<&Arc<Job>>,
    settings: &Settings,
    name: &str,
) -> Result<Arc<dyn Transcriber>, String> {
    let state = app.state::<AppState>();
    let mut cached = lock(&state.transcriber);
    let current = |t: &LoadedTranscriber| t.vad == settings.vad && t.gpu == settings.gpu;
    if let Some(t) = cached.iter().find(|t| t.model == name && current(t)) {
        return Ok(Arc::clone(&t.transcriber));
    }

    let files =
        models::speech_files(name).ok_or_else(|| format!("unknown speech model `{name}`"))?;
    for file in files {
        ensure_downloaded(app, job, file)?;
    }
    let vad_model = if settings.vad {
        Some(ensure_downloaded(app, job, &models::VAD_MODEL)?)
    } else {
        None
    };
    let dir = quill_core::paths::models_dir().ok_or("no home directory")?;
    let config = WhisperConfig {
        vad_model,
        gpu: settings.gpu.clone(),
        ..WhisperConfig::default()
    };
    let transcriber: Arc<dyn Transcriber> =
        Arc::from(quill_stt::load(name, &dir, &config).map_err(|e| e.to_string())?);
    // Drop models the settings no longer use (or loaded with other
    // settings), and this one's stale copy.
    cached.retain(|t| {
        t.model != name
            && current(t)
            && (t.model == settings.stt_model || t.model == settings.dictation_model())
    });
    cached.push(LoadedTranscriber {
        model: name.to_string(),
        vad: settings.vad,
        gpu: settings.gpu.clone(),
        transcriber: Arc::clone(&transcriber),
    });
    Ok(transcriber)
}

/// Downloads `model` if needed (see `models::ensure`), showing progress on
/// `job` too, if given.
fn ensure_downloaded(
    app: &AppHandle,
    job: Option<&Arc<Job>>,
    model: &'static ModelInfo,
) -> Result<std::path::PathBuf, String> {
    let message = if speakers::required_models().contains(&model) {
        "Downloading the voice models".to_string()
    } else {
        // A Parakeet model is four files, named after the model.
        let name = models::PARAKEET_MODELS
            .iter()
            .find(|p| p.files().contains(&model))
            .map_or(model.name, |p| p.name);
        format!("Downloading the {name} speech model")
    };
    let mut last = -1.0;
    let mut downloaded = false;
    let result = crate::models::ensure(app, model, |done, total| {
        let fraction = done as f64 / total.max(1) as f64;
        downloaded = true;
        // Throttle: one event per percent.
        if fraction - last >= 0.01 {
            last = fraction;
            if let Some(job) = job {
                job.download(Some((fraction, message.clone())));
            }
        }
    });
    if downloaded && let Some(job) = job {
        job.download(None);
    }
    result
}

fn send(app: &AppHandle, progress: Progress) {
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
