//! Global-hotkey dictation: speak, and the text is typed into whatever app
//! has focus.
//!
//! The hotkey works two ways: **hold** it while speaking and release to type
//! (push-to-talk), or **tap** it to start and tap again to stop. Esc cancels
//! while listening.
//!
//! A controller thread owns the microphone (cpal streams can't move between
//! threads) and reacts to hotkey messages. When listening stops, the audio
//! goes to an async task: Whisper → optional polish → paste → history.
//!
//! The microphone stays open for [`KEEP_WARM`] after each dictation, so the
//! next one starts instantly and its pre-roll catches the first syllable.
//! macOS shows its orange microphone indicator during that time.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use quill_audio::STT_SAMPLE_RATE;
use quill_audio::cue::{self, Cue};
use quill_audio::live::MicStream;
use quill_core::dictation;
use quill_core::library::DictationEntry;
use quill_core::settings::DictationSettings;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use crate::commands::{CommandError, CommandResult, blocking, lock};
use crate::{AppState, pipeline, tray};

pub const STATE_EVENT: &str = "dictation-state";
pub const ENTRY_EVENT: &str = "dictation-entry";

/// Held at least this long, the hotkey is push-to-talk; shorter is a tap
/// that toggles listening on.
const HOLD: Duration = Duration::from_millis(350);
/// Listening stops by itself after this long.
const MAX_LISTEN: Duration = Duration::from_secs(5 * 60);
/// How long the microphone stays open after a dictation.
const KEEP_WARM: Duration = Duration::from_secs(30);
/// Clips shorter than this are accidental taps.
const MIN_AUDIO: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Listening,
    Processing,
}

#[derive(Debug, Clone, Serialize)]
pub struct DictationState {
    pub phase: Phase,
    /// The last failure, until the next dictation starts.
    pub error: Option<String>,
}

enum Msg {
    Pressed,
    Released,
    Cancel,
    /// (Re)register the hotkey from settings; replies with the outcome.
    Register(mpsc::Sender<Result<(), String>>),
}

/// Handle kept in `AppState`.
pub struct Dictation {
    tx: mpsc::Sender<Msg>,
    shared: Arc<Shared>,
}

struct Shared {
    listening: AtomicBool,
    /// Dictations between "stopped listening" and "typed".
    pending: AtomicUsize,
    error: Mutex<Option<String>>,
    /// The registered hotkey, to tell its events from Esc's.
    hotkey: Mutex<Option<Shortcut>>,
}

impl Dictation {
    /// Starts the controller thread. Call [`Dictation::register`] once the
    /// app is running to bind the hotkey.
    pub fn spawn(app: AppHandle) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            listening: AtomicBool::new(false),
            pending: AtomicUsize::new(0),
            error: Mutex::new(None),
            hotkey: Mutex::new(None),
        });
        let thread_shared = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name("quill-dictation".into())
            .spawn(move || Controller::new(app, thread_shared).run(&rx));
        if let Err(e) = spawned {
            tracing::error!("could not start dictation: {e}");
        }
        Self { tx, shared }
    }

    /// Binds the hotkey from settings (or unbinds it if dictation is off).
    /// Blocks until done, so don't call it on the main thread.
    pub fn register(&self) -> Result<(), String> {
        let (reply, result) = mpsc::channel();
        self.tx
            .send(Msg::Register(reply))
            .map_err(|_| "dictation isn't running".to_string())?;
        result
            .recv()
            .map_err(|_| "dictation isn't running".to_string())?
    }

    pub fn state(&self) -> DictationState {
        self.shared.state()
    }

    /// Called by the global-shortcut plugin, on the main thread: only
    /// forwards, never blocks.
    pub fn on_shortcut(&self, shortcut: &Shortcut, event: ShortcutEvent) {
        let is_hotkey = lock(&self.shared.hotkey).as_ref() == Some(shortcut);
        let msg = match (is_hotkey, event.state) {
            (true, ShortcutState::Pressed) => Msg::Pressed,
            (true, ShortcutState::Released) => Msg::Released,
            (false, ShortcutState::Pressed) => Msg::Cancel,
            (false, ShortcutState::Released) => return,
        };
        let _ = self.tx.send(msg);
    }
}

impl Shared {
    fn state(&self) -> DictationState {
        let phase = if self.listening.load(Ordering::SeqCst) {
            Phase::Listening
        } else if self.pending.load(Ordering::SeqCst) > 0 {
            Phase::Processing
        } else {
            Phase::Idle
        };
        DictationState {
            phase,
            error: lock(&self.error).clone(),
        }
    }
}

/// Tells the UI and the menu bar what dictation is doing.
fn notify(app: &AppHandle, shared: &Shared) {
    let state = shared.state();
    tray::set_dictation(app, state.phase);
    let _ = app.emit(STATE_EVENT, state);
}

fn escape() -> Shortcut {
    Shortcut::new(None, tauri_plugin_global_shortcut::Code::Escape)
}

struct Controller {
    app: AppHandle,
    shared: Arc<Shared>,
    mic: Option<MicStream>,
    last_used: Instant,
    listening: Option<Listening>,
}

struct Listening {
    since: Instant,
    /// Started by a tap: keeps listening after the key is released.
    toggled: bool,
}

impl Controller {
    fn new(app: AppHandle, shared: Arc<Shared>) -> Self {
        Self {
            app,
            shared,
            mic: None,
            last_used: Instant::now(),
            listening: None,
        }
    }

    fn run(mut self, rx: &mpsc::Receiver<Msg>) {
        loop {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(Msg::Pressed) => match &self.listening {
                    None => self.start(),
                    Some(l) if l.toggled => self.stop(),
                    // Key repeat while held.
                    Some(_) => {}
                },
                Ok(Msg::Released) => {
                    if let Some(l) = &mut self.listening
                        && !l.toggled
                    {
                        if l.since.elapsed() >= HOLD {
                            self.stop();
                        } else {
                            l.toggled = true;
                        }
                    }
                }
                Ok(Msg::Cancel) => self.cancel(),
                Ok(Msg::Register(reply)) => {
                    let _ = reply.send(self.register());
                }
                Err(RecvTimeoutError::Timeout) => match &self.listening {
                    Some(l) if l.since.elapsed() >= MAX_LISTEN => self.stop(),
                    Some(_) => {}
                    None if self.last_used.elapsed() >= KEEP_WARM => self.mic = None,
                    None => {}
                },
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn register(&mut self) -> Result<(), String> {
        let settings = lock(&self.app.state::<AppState>().settings)
            .dictation
            .clone();
        let shortcuts = self.app.global_shortcut();
        if let Some(old) = lock(&self.shared.hotkey).take() {
            let _ = shortcuts.unregister(old);
        }
        if !settings.enabled {
            self.mic = None;
            return Ok(());
        }
        let hotkey: Shortcut = settings
            .hotkey
            .parse()
            .map_err(|e| format!("“{}” isn't a valid shortcut: {e}", settings.hotkey))?;
        shortcuts.register(hotkey).map_err(|e| {
            format!(
                "couldn't use {} as the dictation shortcut (another app may have it): {e}",
                settings.hotkey
            )
        })?;
        *lock(&self.shared.hotkey) = Some(hotkey);
        tracing::info!(hotkey = %settings.hotkey, "dictation shortcut registered");
        Ok(())
    }

    fn start(&mut self) {
        if self.mic.as_ref().is_none_or(|m| !m.is_healthy()) {
            self.mic = None;
            match MicStream::open() {
                Ok(mic) => self.mic = Some(mic),
                Err(e) => {
                    self.fail(format!("couldn't open the microphone: {e}"));
                    return;
                }
            }
        }
        let Some(mic) = &self.mic else {
            return;
        };
        mic.begin();
        cue::play(Cue::Start);
        *lock(&self.shared.error) = None;
        self.listening = Some(Listening {
            since: Instant::now(),
            toggled: false,
        });
        self.shared.listening.store(true, Ordering::SeqCst);
        // Esc cancels, but only while listening: a global Esc would steal it
        // from every other app.
        if let Err(e) = self.app.global_shortcut().register(escape()) {
            tracing::debug!("couldn't bind Esc to cancel: {e}");
        }
        notify(&self.app, &self.shared);
    }

    fn stop(&mut self) {
        let Some(listening) = self.end_listening() else {
            return;
        };
        let Some(mic) = &self.mic else {
            return;
        };
        let pcm = match mic.finish() {
            Ok(pcm) => pcm,
            Err(e) => {
                self.fail(format!("couldn't read the microphone: {e}"));
                return;
            }
        };
        let audio = Duration::from_secs_f64(pcm.len() as f64 / f64::from(STT_SAMPLE_RATE));
        if audio < MIN_AUDIO {
            tracing::debug!(?audio, "too short; ignoring");
            notify(&self.app, &self.shared);
            return;
        }
        cue::play(Cue::Stop);
        tracing::debug!(?audio, held = ?listening.since.elapsed(), "processing dictation");
        self.shared.pending.fetch_add(1, Ordering::SeqCst);
        notify(&self.app, &self.shared);
        let app = self.app.clone();
        let shared = Arc::clone(&self.shared);
        tauri::async_runtime::spawn(async move {
            let stopped = Instant::now();
            let result = process(&app, pcm, audio, stopped).await;
            shared.pending.fetch_sub(1, Ordering::SeqCst);
            if let Err(e) = result {
                tracing::error!("dictation failed: {e}");
                cue::play(Cue::Cancel);
                *lock(&shared.error) = Some(e);
            }
            notify(&app, &shared);
        });
    }

    fn cancel(&mut self) {
        if self.end_listening().is_some() {
            if let Some(mic) = &self.mic {
                mic.cancel();
            }
            cue::play(Cue::Cancel);
            notify(&self.app, &self.shared);
        }
    }

    fn end_listening(&mut self) -> Option<Listening> {
        let listening = self.listening.take()?;
        self.shared.listening.store(false, Ordering::SeqCst);
        self.last_used = Instant::now();
        let _ = self.app.global_shortcut().unregister(escape());
        Some(listening)
    }

    fn fail(&self, error: String) {
        tracing::error!("dictation: {error}");
        cue::play(Cue::Cancel);
        *lock(&self.shared.error) = Some(error);
        notify(&self.app, &self.shared);
    }
}

/// Runs `f` on the main thread and waits for its result. Don't call it
/// from the main thread.
fn on_main_thread<T: Send + 'static>(
    app: &AppHandle,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .map_err(|e| e.to_string())?;
    rx.recv()
        .map_err(|_| "the main thread didn't run the paste".to_string())
}

/// Whisper → polish (if useful) → paste → history.
async fn process(
    app: &AppHandle,
    pcm: Vec<f32>,
    audio: Duration,
    stopped: Instant,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let settings = lock(&state.settings).clone();

    let raw = {
        let app = app.clone();
        let settings = settings.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let transcriber = pipeline::transcriber(&app, None, &settings)?;
            dictation::transcribe(transcriber.as_ref(), &pcm, &settings.dictation)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??
    };
    let stt_ms = stopped.elapsed().as_millis();
    if raw.is_empty() {
        tracing::debug!(stt_ms, "no speech");
        return Ok(());
    }

    let mut text = raw.clone();
    let mut polish_model = None;
    if settings.dictation.polish && dictation::needs_polish(&raw) {
        let polished =
            match pipeline::llm_provider(app, &settings.dictation.polish_model, settings.zdr) {
                Ok((provider, model)) => dictation::polish(
                    provider.as_ref(),
                    &model,
                    &quill_llm::prompts::DICTATION,
                    &raw,
                    &settings.dictation,
                )
                .await
                .map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
        match polished {
            Ok(p) => {
                if p.rejected.is_none() {
                    polish_model = Some(p.model);
                }
                text = p.text;
            }
            // Typing the unpolished text beats typing nothing.
            Err(e) => tracing::warn!("polish failed; typing the transcript: {e}"),
        }
    }

    let (inserted, latency) = {
        let app = app.clone();
        let text = text.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let clipboard = quill_insert::Clipboard::put(&text).map_err(|e| e.to_string())?;
            // The keystroke must be simulated on the main thread (see
            // quill_insert::Clipboard).
            let inserted =
                on_main_thread(&app, quill_insert::press_paste)?.map_err(|e| e.to_string())?;
            let latency = stopped.elapsed();
            if inserted == quill_insert::Inserted::Pasted {
                // Put the user's clipboard back once the app has read ours.
                // Without a paste, our text stays there to paste by hand.
                std::thread::spawn(move || clipboard.restore());
            }
            Ok::<_, String>((inserted, latency))
        })
        .await
        .map_err(|e| e.to_string())??
    };
    let latency_ms = u64::try_from(latency.as_millis()).unwrap_or(u64::MAX);
    // Timings only: dictated text stays out of the logs.
    tracing::info!(
        stt_ms,
        latency_ms,
        polished = polish_model.is_some(),
        ?inserted,
        "dictation typed"
    );

    let pasted = inserted == quill_insert::Inserted::Pasted;
    let entry = DictationEntry {
        id: 0,
        created_at: String::new(),
        text,
        raw: Some(raw),
        polish_model,
        audio_ms: u64::try_from(audio.as_millis()).unwrap_or(u64::MAX),
        latency_ms,
        pasted,
    };
    if settings.dictation.keep_history {
        match state.library.add_dictation(&entry) {
            Ok(saved) => {
                let _ = app.emit(ENTRY_EVENT, saved);
            }
            Err(e) => tracing::warn!("couldn't save dictation history: {e}"),
        }
    }
    if !pasted {
        return Err(
            "Quill isn't allowed to paste, so the text is on your clipboard. Allow Quill under \
             Privacy & Security → Accessibility."
                .into(),
        );
    }
    Ok(())
}

// ---- Commands -------------------------------------------------------------

#[tauri::command]
pub fn dictation_state(dictation: tauri::State<'_, Dictation>) -> DictationState {
    dictation.state()
}

/// Saves the dictation settings and rebinds the hotkey. If the new hotkey
/// can't be bound, the old settings stay.
#[tauri::command]
pub async fn save_dictation_settings(
    app: AppHandle,
    dictation: DictationSettings,
) -> CommandResult<()> {
    blocking(move || {
        let state = app.state::<AppState>();
        let previous = lock(&state.settings).clone();
        let mut settings = previous.clone();
        settings.dictation = dictation;
        *lock(&state.settings) = settings.clone();
        if let Err(e) = app.state::<Dictation>().register() {
            *lock(&state.settings) = previous;
            // Put the old hotkey back.
            let _ = app.state::<Dictation>().register();
            return Err(CommandError::Other(e));
        }
        if let Err(e) = settings.save(&state.data_dir) {
            *lock(&state.settings) = previous;
            let _ = app.state::<Dictation>().register();
            return Err(e.into());
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn list_dictations(state: tauri::State<'_, AppState>) -> CommandResult<Vec<DictationEntry>> {
    Ok(state.library.dictations(100)?)
}

#[tauri::command]
pub fn delete_dictation(state: tauri::State<'_, AppState>, id: i64) -> CommandResult<()> {
    Ok(state.library.delete_dictation(id)?)
}

#[tauri::command]
pub fn clear_dictations(state: tauri::State<'_, AppState>) -> CommandResult<()> {
    Ok(state.library.clear_dictations()?)
}

/// Whether Quill may simulate the paste keystroke (macOS Accessibility).
#[tauri::command]
pub async fn paste_permission() -> CommandResult<bool> {
    blocking(|| Ok(quill_insert::can_simulate_paste(false))).await
}

/// Shows macOS's own "allow in System Settings" prompt for Accessibility
/// and opens that settings page.
#[tauri::command]
pub async fn request_paste_permission() -> CommandResult<bool> {
    blocking(|| {
        let allowed = quill_insert::can_simulate_paste(true);
        if !allowed && cfg!(target_os = "macos") {
            let _ = tauri_plugin_opener::open_url(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
                None::<&str>,
            );
        }
        Ok(allowed)
    })
    .await
}

/// The language dictation uses when none is set, e.g. "en".
#[tauri::command]
pub fn system_language() -> Option<String> {
    dictation::language(None)
}
