//! Search in the app: the embedding model for search by meaning, kept in
//! memory while it's in use; a background worker that indexes the library;
//! and searching across everything. The logic is in `quill_core::semantic`
//! (transcripts) and `quill_core::search` (everything else).
//!
//! The model never downloads on its own: the user asks for it (the search
//! window, or Settings → Models). Until it's there, and when Settings →
//! Meetings → Search by meaning is off, search is by words only. Indexing
//! waits while a meeting records or transcribes and while dictating, so it
//! never competes with them.

use std::collections::HashSet;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use quill_core::search::{Results, Scope};
use quill_core::semantic::{self, Coverage};
use quill_embed::{E5Embedder, Embedder};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;
use crate::commands::{CommandError, CommandResult, lock};

/// Sent whenever [`IndexStatus`] changes.
pub const SEMANTIC_EVENT: &str = "semantic-index";

/// The model is unloaded after this long unused: it takes about 0.5 GB.
const IDLE_UNLOAD: Duration = Duration::from_secs(10 * 60);
/// How often the worker looks for work without being woken.
const POLL: Duration = Duration::from_secs(60);
/// How often it checks whether what it waits for has finished.
const BUSY_POLL: Duration = Duration::from_secs(5);

/// Most results of each kind a search across everything returns...
const ALL_LIMIT: usize = 5;
/// ...and of the one kind a narrowed search asks for.
const SCOPED_LIMIT: usize = 50;

/// Kept in Tauri's state.
#[derive(Default)]
pub struct Semantic {
    /// Loaded on first use; held while loading, so it loads once.
    embedder: Mutex<Option<Arc<E5Embedder>>>,
    last_used: Mutex<Option<Instant>>,
    /// Set by [`wake`], cleared by the worker.
    wake: Mutex<bool>,
    woken: Condvar,
    status: Mutex<IndexStatus>,
}

/// What the search window shows about search by meaning.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct IndexStatus {
    /// Settings → Meetings → Search by meaning.
    pub enabled: bool,
    /// The model's files are downloaded.
    pub model_ready: bool,
    /// What there is to index (transcripts, notes, action items
    /// and dictations), and how much of it is.
    pub indexed: u64,
    pub total: u64,
    /// Indexing waits for a recording, transcription or dictation.
    pub paused: bool,
    pub error: Option<String>,
}

/// Tells the worker to look for work now: after a transcript is saved, the
/// model downloaded, or the setting changed.
pub fn wake(app: &AppHandle) {
    let semantic = app.state::<Semantic>();
    *lock(&semantic.wake) = true;
    semantic.woken.notify_all();
}

/// Starts the background worker.
pub fn spawn(app: AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("quill-semantic".into())
        .spawn(move || run(&app));
    if let Err(e) = spawned {
        tracing::error!("could not start indexing for search: {e}");
    }
}

fn run(app: &AppHandle) {
    let mut failed = HashSet::new();
    loop {
        match step(app, &mut failed) {
            Step::Again => {}
            Step::Busy => sleep(app, BUSY_POLL),
            Step::Idle => {
                unload_if_idle(app);
                sleep(app, POLL);
            }
        }
    }
}

enum Step {
    /// Indexed a meeting or a batch of items (or was stopped): look again
    /// straight away.
    Again,
    /// Waiting for something else to finish.
    Busy,
    /// Nothing to do.
    Idle,
}

/// Transcripts and everything else, together.
fn coverage(app: &AppHandle) -> Coverage {
    let library = &app.state::<AppState>().library;
    let meetings = library
        .semantic_coverage(E5Embedder::ID)
        .unwrap_or_default();
    let items = library
        .search_items_coverage(E5Embedder::ID)
        .unwrap_or_default();
    Coverage {
        indexed: meetings.indexed + items.indexed,
        total: meetings.total + items.total,
    }
}

/// One pass: brings the status up to date and indexes one meeting, or once
/// they're done, a batch of notes, action items and dictations.
fn step(app: &AppHandle, failed: &mut HashSet<String>) -> Step {
    let state = app.state::<AppState>();
    let on = enabled(app);
    let ready = model_ready();
    let coverage = coverage(app);
    let waiting = busy(app);
    update_status(app, |s| {
        s.enabled = on;
        s.model_ready = ready;
        s.indexed = coverage.indexed;
        s.total = coverage.total;
        s.paused = waiting && coverage.indexed < coverage.total;
    });
    if !on || !ready {
        unload(app);
        return Step::Idle;
    }
    if coverage.indexed >= coverage.total {
        return Step::Idle;
    }
    if waiting {
        return Step::Busy;
    }
    let pending = match state.library.semantic_pending(E5Embedder::ID) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("search index: {e}");
            return Step::Idle;
        }
    };
    let next = pending.into_iter().find(|id| !failed.contains(id));
    let embedder = match load(app) {
        Ok(e) => e,
        Err(e) => {
            update_status(app, |s| s.error = Some(e.clone()));
            return Step::Idle;
        }
    };
    let keep_going = || enabled(app) && !busy(app);
    let Some(id) = next else {
        let step = match state.library.index_search_items(&*embedder, &keep_going) {
            Ok(Some(0)) => Step::Idle,
            Ok(_) => {
                update_status(app, |s| s.error = None);
                Step::Again
            }
            Err(e) => {
                tracing::warn!("couldn't index notes, action items or dictations for search: {e}");
                update_status(app, |s| s.error = Some(e.to_string()));
                Step::Idle
            }
        };
        *lock(&app.state::<Semantic>().last_used) = Some(Instant::now());
        return step;
    };
    let started = Instant::now();
    match state.library.index_meeting(&id, &*embedder, &keep_going) {
        Ok(Some(chunks)) => {
            tracing::info!(
                meeting_id = id,
                chunks,
                ms = started.elapsed().as_millis(),
                "indexed for search by meaning"
            );
            update_status(app, |s| s.error = None);
        }
        // Stopped part way, or the transcript changed: it's still pending.
        Ok(None) => {}
        Err(e) => {
            tracing::warn!(meeting_id = id, "couldn't index for search: {e}");
            failed.insert(id);
            update_status(app, |s| s.error = Some(e.to_string()));
        }
    }
    *lock(&app.state::<Semantic>().last_used) = Some(Instant::now());
    Step::Again
}

/// Waits up to `timeout`, or until [`wake`].
fn sleep(app: &AppHandle, timeout: Duration) {
    let semantic = app.state::<Semantic>();
    let mut woken = lock(&semantic.wake);
    if !*woken {
        woken = match semantic.woken.wait_timeout(woken, timeout) {
            Ok((guard, _)) => guard,
            Err(poisoned) => poisoned.into_inner().0,
        };
    }
    *woken = false;
}

fn enabled(app: &AppHandle) -> bool {
    lock(&app.state::<AppState>().settings).semantic_search
}

fn model_ready() -> bool {
    quill_core::paths::models_dir().is_some_and(|dir| semantic::model_ready(&dir))
}

/// Recording, transcribing or summarizing a meeting, moving recordings, or
/// dictating: indexing waits, so they get the computer to themselves.
fn busy(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    lock(&state.recording).is_some()
        || !lock(&state.processing).is_empty()
        || state.moving.load(std::sync::atomic::Ordering::SeqCst)
        || app
            .try_state::<crate::dictation::Dictation>()
            .is_some_and(|d| d.state().phase != crate::dictation::Phase::Idle)
}

/// The model, loading it the first time (about half a second).
fn load(app: &AppHandle) -> Result<Arc<E5Embedder>, String> {
    let semantic = app.state::<Semantic>();
    let mut slot = lock(&semantic.embedder);
    *lock(&semantic.last_used) = Some(Instant::now());
    if let Some(e) = slot.as_ref() {
        return Ok(Arc::clone(e));
    }
    let dir = quill_core::paths::models_dir().ok_or("no home directory")?;
    let started = Instant::now();
    let embedder = semantic::load_embedder(&dir, semantic::default_threads())
        .map(Arc::new)
        .map_err(|e| format!("couldn't load the search model: {e}"))?;
    tracing::info!(
        ms = started.elapsed().as_millis(),
        "loaded the search model"
    );
    *slot = Some(Arc::clone(&embedder));
    Ok(embedder)
}

/// Frees the model's memory (a search or indexing in progress keeps its
/// own reference and finishes).
pub fn unload(app: &AppHandle) {
    if lock(&app.state::<Semantic>().embedder).take().is_some() {
        tracing::info!("unloaded the search model");
    }
}

fn unload_if_idle(app: &AppHandle) {
    let unused =
        lock(&app.state::<Semantic>().last_used).is_none_or(|t| t.elapsed() >= IDLE_UNLOAD);
    if unused {
        unload(app);
    }
}

fn update_status(app: &AppHandle, change: impl FnOnce(&mut IndexStatus)) {
    let semantic = app.state::<Semantic>();
    let mut status = lock(&semantic.status);
    let before = status.clone();
    change(&mut status);
    if *status != before
        && let Err(e) = app.emit(SEMANTIC_EVENT, status.clone())
    {
        tracing::warn!("could not emit search index status: {e}");
    }
}

/// Searches what `scope` covers by words, and by meaning too when it's on
/// and the model is downloaded: a few results of each kind across
/// everything, more when narrowed to one. Blocking: the first search loads
/// the model.
pub fn search(app: &AppHandle, query: &str, scope: Scope) -> CommandResult<Results> {
    let embedder = if enabled(app) && model_ready() {
        match load(app) {
            Ok(e) => Some(e),
            Err(e) => {
                tracing::warn!("searching by words only: {e}");
                None
            }
        }
    } else {
        None
    };
    let limit = if scope == Scope::All {
        ALL_LIMIT
    } else {
        SCOPED_LIMIT
    };
    let started = Instant::now();
    let results = app
        .state::<AppState>()
        .library
        .search_everything(
            query,
            scope,
            embedder.as_deref().map(|e| e as &dyn Embedder),
            limit,
        )
        .map_err(|e| CommandError::Other(e.to_string()))?;
    tracing::debug!(
        ?scope,
        results = results.len(),
        ms = started.elapsed().as_millis(),
        "searched"
    );
    Ok(results)
}

/// Where search by meaning stands, for the search window.
#[tauri::command]
pub fn semantic_status(app: AppHandle) -> IndexStatus {
    let coverage = coverage(&app);
    let enabled = enabled(&app);
    let ready = model_ready();
    update_status(&app, |s| {
        s.enabled = enabled;
        s.model_ready = ready;
        s.indexed = coverage.indexed;
        s.total = coverage.total;
    });
    lock(&app.state::<Semantic>().status).clone()
}

/// Loads the model in the background when the search window opens, so the
/// first search doesn't wait for it.
#[tauri::command]
pub fn prepare_search(app: AppHandle) {
    if !enabled(&app) || !model_ready() {
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = load(&app) {
            tracing::warn!("{e}");
        }
    });
}

/// Downloads the search model's files, when the user asks. Progress
/// arrives as `model-download` events; indexing starts when it's done.
#[tauri::command]
pub fn download_search_model(app: AppHandle) {
    crate::models::download_in_background(&app, semantic::required_models().iter().collect());
}
