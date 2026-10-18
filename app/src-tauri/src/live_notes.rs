//! Live notes during a meeting (Settings → Meetings → Live notes): while
//! recording, a worker follows the two tracks as they're written, keeps a
//! rough transcript (`quill_core::live_meeting`), and every minute asks
//! the summary model to update the notes: what to ask before the end,
//! follow-ups, decisions. A panel at the right edge of the screen shows
//! them.
//!
//! The panel takes clicks without taking focus or bringing Quill's main
//! window over the call (`floating.rs`), and is hidden from screen sharing and recording, so the other
//! people on the call don't see it when you share your screen.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use quill_audio::growing::GrowingWav;
use quill_core::live_meeting::{self, Item, LiveTranscript, Notes};
use quill_core::meeting::{MIC_FILE, SYSTEM_FILE};
use quill_stt::TranscribeOptions;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::commands::lock;
use crate::floating::Place;
use crate::{AppState, pipeline};

pub const LABEL: &str = "live";
/// Sent to the panel with the [`LiveState`] whenever it changes.
pub const EVENT: &str = "live-notes";

/// How often the notes are updated: often enough to keep up with the
/// conversation (every two minutes felt behind it).
const NOTES_EVERY: Duration = Duration::from_secs(60);
/// How often the worker reads new audio and transcribes what's ready.
const TICK: Duration = Duration::from_secs(2);
/// Logical sizes: the panel, and the tab it collapses to.
const WIDTH: f64 = 340.0;
const MAX_HEIGHT: f64 = 640.0;
const TAB: (f64, f64) = (46.0, 140.0);
/// A track that has written nothing this long into the recording is
/// missing (no permission, no device): count it as silence.
const MISSING_AFTER: Duration = Duration::from_secs(10);

/// What the panel shows.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LiveState {
    /// A meeting is being recorded with live notes.
    pub active: bool,
    pub notes: Notes,
    /// Recording time when the notes were last updated, and when they will be.
    pub updated_ms: Option<u64>,
    pub next_ms: Option<u64>,
    pub updating: bool,
    /// The latest transcribed line, to show it's listening.
    pub heard: Option<String>,
    pub error: Option<String>,
    pub collapsed: bool,
}

/// The panel's state and the worker's controls.
#[derive(Default)]
pub struct LiveNotes {
    state: Mutex<LiveState>,
    dismissed: Mutex<Vec<Item>>,
    ticked: Mutex<Vec<String>>,
    refresh: AtomicBool,
    stop: Mutex<Option<Arc<AtomicBool>>>,
}

impl LiveNotes {
    fn emit(&self, app: &AppHandle) {
        let state = lock(&self.state).clone();
        let _ = app.emit_to(LABEL, EVENT, state);
    }
}

/// Creates the (hidden) panel window, so showing it later is instant.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    app.manage(LiveNotes::default());
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Quill live notes")
        .inner_size(WIDTH, MAX_HEIGHT)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focusable(false)
        .focused(false)
        // A click on it is a click, though the panel is never active.
        .accept_first_mouse(true)
        // Not in screen shares or recordings: only the person recording
        // sees it.
        .content_protected(true)
        .visible(false)
        .build()?;
    crate::floating::adopt(&window, true);
    Ok(())
}

/// Starts live notes for the meeting being recorded into `dir`, if the
/// setting is on.
pub fn start(app: &AppHandle, dir: PathBuf) {
    let settings = lock(&app.state::<AppState>().settings).clone();
    if !settings.live_notes {
        return;
    }
    let Some(live) = app.try_state::<LiveNotes>() else {
        return;
    };
    let stop = Arc::new(AtomicBool::new(false));
    if let Some(old) = lock(&live.stop).replace(Arc::clone(&stop)) {
        old.store(true, Ordering::SeqCst);
    }
    *lock(&live.state) = LiveState {
        active: true,
        next_ms: Some(NOTES_EVERY.as_millis() as u64),
        ..LiveState::default()
    };
    lock(&live.dismissed).clear();
    lock(&live.ticked).clear();
    live.emit(app);
    show(app, false);
    let app = app.clone();
    std::thread::Builder::new()
        .name("quill-live-notes".into())
        .spawn(move || work(&app, &dir, &stop))
        .ok();
}

/// Stops live notes when the recording stops.
pub fn stop(app: &AppHandle) {
    let Some(live) = app.try_state::<LiveNotes>() else {
        return;
    };
    if let Some(stop) = lock(&live.stop).take() {
        stop.store(true, Ordering::SeqCst);
    }
    lock(&live.state).active = false;
    live.emit(app);
    if let Some(window) = app.get_webview_window(LABEL) {
        crate::floating::hide(&window);
    }
}

/// The worker: follow the tracks, transcribe, update the notes.
fn work(app: &AppHandle, dir: &std::path::Path, stop: &AtomicBool) {
    let started = Instant::now();
    let live = app.state::<LiveNotes>();
    let settings = lock(&app.state::<AppState>().settings).clone();
    let transcriber = match pipeline::transcriber(app, None, &settings) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("no live notes: {e}");
            lock(&live.state).error = Some(format!("Couldn't start transcribing: {e}"));
            live.emit(app);
            return;
        }
    };
    let opts = TranscribeOptions {
        language: settings.language.clone(),
        ..TranscribeOptions::default()
    };
    let (mut mic, mut system) = (
        GrowingWav::new(dir.join(MIC_FILE)),
        GrowingWav::new(dir.join(SYSTEM_FILE)),
    );
    let (mut mic_total, mut system_total) = (0_usize, 0_usize);
    let mut transcript = LiveTranscript::new();
    let mut last_update = Instant::now();
    let mut lines_at_update = 0;
    let mut cost = 0.0_f64;
    let mut updates = 0_u32;

    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(TICK);
        let mut new_mic = mic.read_new().unwrap_or_default();
        let mut new_system = system.read_new().unwrap_or_default();
        // A track that never arrives (no system-audio permission, say)
        // shouldn't hold the other back: count it as silence.
        if started.elapsed() > MISSING_AFTER {
            if system_total == 0 && new_system.is_empty() {
                new_system = vec![0.0; new_mic.len()];
            }
            if mic_total == 0 && new_mic.is_empty() {
                new_mic = vec![0.0; new_system.len()];
            }
        }
        mic_total += new_mic.len();
        system_total += new_system.len();
        transcript.push(&new_mic, &new_system);

        let refresh = live.refresh.swap(false, Ordering::SeqCst);
        match transcript.step(transcriber.as_ref(), &opts, refresh) {
            Ok(0) => {}
            Ok(_) => {
                lock(&live.state).heard = transcript.lines().last().map(|l| l.text.clone());
                live.emit(app);
            }
            Err(e) => tracing::warn!("live transcription: {e}"),
        }

        let due = last_update.elapsed() >= NOTES_EVERY;
        let new_lines = transcript.lines().len() > lines_at_update;
        if !(refresh || (due && new_lines)) {
            if due {
                last_update = Instant::now();
                lock(&live.state).next_ms =
                    Some(elapsed_ms(started) + NOTES_EVERY.as_millis() as u64);
                live.emit(app);
            }
            continue;
        }
        lock(&live.state).updating = true;
        live.emit(app);
        let result = update_notes(app, &settings, &transcript, started);
        let mut state = lock(&live.state);
        state.updating = false;
        match result {
            Ok((notes, call_cost)) => {
                updates += 1;
                cost += call_cost.unwrap_or(0.0);
                // Counts only: what was said stays out of the logs.
                tracing::info!(
                    updates,
                    items = notes.items.len(),
                    cost_usd = cost,
                    "live notes updated"
                );
                state.notes = notes;
                state.error = None;
                state.updated_ms = Some(elapsed_ms(started));
            }
            Err(e) => {
                tracing::warn!("live notes: {e}");
                state.error = Some(e);
            }
        }
        lines_at_update = transcript.lines().len();
        last_update = Instant::now();
        state.next_ms = Some(elapsed_ms(started) + NOTES_EVERY.as_millis() as u64);
        drop(state);
        live.emit(app);
    }
}

fn update_notes(
    app: &AppHandle,
    settings: &quill_core::settings::Settings,
    transcript: &LiveTranscript,
    started: Instant,
) -> Result<(Notes, Option<f64>), String> {
    let live = app.state::<LiveNotes>();
    let (provider, model) = pipeline::llm_provider(app, &settings.summary_model, settings.zdr)
        .map_err(|e| e.to_string())?;
    let previous = lock(&live.state).notes.clone();
    let dismissed = lock(&live.dismissed).clone();
    let ticked = lock(&live.ticked).clone();
    let text = transcript.recent_text();
    tauri::async_runtime::block_on(live_meeting::update(
        provider.as_ref(),
        &model,
        &quill_llm::prompts::LIVE,
        live_meeting::Update {
            previous: &previous,
            dismissed: &dismissed,
            ticked: &ticked,
            transcript: &text,
            minutes: started.elapsed().as_secs() / 60,
        },
    ))
    .map_err(|e| e.to_string())
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Shows the panel at the right edge of the screen the mouse is on, open or
/// collapsed to a tab.
fn show(app: &AppHandle, collapsed: bool) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    if collapsed {
        crate::floating::show(&window, TAB, Place::RightThird);
    } else {
        crate::floating::show(&window, (WIDTH, MAX_HEIGHT), Place::RightTop);
    }
}

#[tauri::command]
pub fn live_notes_state(live: tauri::State<'_, LiveNotes>) -> LiveState {
    lock(&live.state).clone()
}

/// Updates the notes now, with everything heard so far.
#[tauri::command]
pub fn live_notes_refresh(app: AppHandle, live: tauri::State<'_, LiveNotes>) {
    live.refresh.store(true, Ordering::SeqCst);
    lock(&live.state).updating = true;
    live.emit(&app);
}

/// Hides an item for good (the model is told not to bring it back).
#[tauri::command]
pub fn live_notes_dismiss(app: AppHandle, live: tauri::State<'_, LiveNotes>, id: String) {
    let mut state = lock(&live.state);
    if let Some(i) = state.notes.items.iter().position(|i| i.id == id) {
        let item = state.notes.items.remove(i);
        lock(&live.dismissed).push(item);
    }
    drop(state);
    live.emit(&app);
}

/// Ticks an item off (or back on).
#[tauri::command]
pub fn live_notes_tick(app: AppHandle, live: tauri::State<'_, LiveNotes>, id: String, done: bool) {
    let mut ticked = lock(&live.ticked);
    ticked.retain(|t| *t != id);
    if done {
        ticked.push(id.clone());
    }
    drop(ticked);
    if let Some(item) = lock(&live.state)
        .notes
        .items
        .iter_mut()
        .find(|i| i.id == id)
    {
        item.done = done;
    }
    live.emit(&app);
}

#[tauri::command]
pub fn live_notes_collapse(app: AppHandle, live: tauri::State<'_, LiveNotes>, collapsed: bool) {
    lock(&live.state).collapsed = collapsed;
    live.emit(&app);
    show(&app, collapsed);
}
