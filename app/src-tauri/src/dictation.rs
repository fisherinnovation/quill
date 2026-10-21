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
use quill_audio::live::{MicStream, MicTap};
use quill_core::library::{DictationEntry, DictationStat, Suggestion, SuggestionStatus};
use quill_core::live::{Caption, LiveTranscript};
use quill_core::settings::Settings;
use quill_core::settings::{DictationSettings, FN_HOTKEY, Style};
use quill_core::{dictation, samples};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use crate::commands::{CommandError, CommandResult, blocking, lock};
use crate::{AppState, indicator, pipeline, tray};

pub const STATE_EVENT: &str = "dictation-state";
pub const ENTRY_EVENT: &str = "dictation-entry";
/// A sample was saved; carries the new [`samples::Usage`].
pub const SAMPLES_EVENT: &str = "dictation-samples";

/// Held at least this long, the hotkey is push-to-talk; shorter is a tap
/// that toggles listening on.
const HOLD: Duration = Duration::from_millis(350);
/// Listening stops by itself after this long.
const MAX_LISTEN: Duration = Duration::from_secs(5 * 60);
/// How long the microphone stays open after a dictation.
const KEEP_WARM: Duration = Duration::from_secs(30);
/// Clips shorter than this are accidental taps.
const MIN_AUDIO: Duration = Duration::from_millis(400);
/// fn waits this long before listening, so fn+Delete, fn+arrows and the
/// like (a key within this time) don't start and cancel a dictation each
/// time. The microphone's pre-roll covers the delay when it's warm.
const FN_GRACE: Duration = Duration::from_millis(150);

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
    /// The fn key (macOS), with its grace period: see [`FN_GRACE`]. Only
    /// the macOS fn-key watcher sends these.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    FnDown,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    FnUp,
    /// A key pressed while fn was held: a shortcut, not dictation.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    FnCombo,
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
    /// Set while the fn key is the trigger and being watched.
    fn_watching: AtomicBool,
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
            fn_watching: AtomicBool::new(false),
        });
        let thread_shared = Arc::clone(&shared);
        let thread_tx = tx.clone();
        let spawned = std::thread::Builder::new()
            .name("quill-dictation".into())
            .spawn(move || Controller::new(app, thread_shared, thread_tx).run(&rx));
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

    /// If the fn key is the trigger but couldn't be watched (no
    /// Accessibility access yet), tries again. Blocking.
    pub fn retry_fn_key(&self, app: &AppHandle) {
        let wants_fn = {
            let state = app.state::<AppState>();
            let settings = lock(&state.settings);
            settings.dictation.enabled && settings.dictation.hotkey == FN_HOTKEY
        };
        if wants_fn
            && !self.shared.fn_watching.load(Ordering::SeqCst)
            && let Err(e) = self.register()
        {
            tracing::warn!("fn key still unavailable: {e}");
        }
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
    indicator::update(app, state.phase);
    let _ = app.emit(STATE_EVENT, state);
}

fn escape() -> Shortcut {
    Shortcut::new(None, tauri_plugin_global_shortcut::Code::Escape)
}

struct Controller {
    app: AppHandle,
    shared: Arc<Shared>,
    /// For the fn-key watcher (macOS) to send to.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    tx: mpsc::Sender<Msg>,
    mic: Option<MicStream>,
    last_used: Instant,
    listening: Option<Listening>,
    /// Transcribing as the user speaks, for captions, while listening.
    live: Option<LiveWorker>,
    /// When fn went down, while waiting out [`FN_GRACE`].
    fn_pending: Option<Instant>,
    #[cfg(target_os = "macos")]
    fn_key: Option<quill_insert::fn_key::FnKey>,
}

struct Listening {
    since: Instant,
    /// Started by a tap: keeps listening after the key is released.
    toggled: bool,
    /// The app in front when dictation started: where the text will go.
    app: Option<quill_insert::FrontApp>,
}

impl Controller {
    fn new(app: AppHandle, shared: Arc<Shared>, tx: mpsc::Sender<Msg>) -> Self {
        Self {
            app,
            shared,
            tx,
            mic: None,
            last_used: Instant::now(),
            listening: None,
            live: None,
            fn_pending: None,
            #[cfg(target_os = "macos")]
            fn_key: None,
        }
    }

    fn run(mut self, rx: &mpsc::Receiver<Msg>) {
        loop {
            // While listening, wake often to feed the indicator's meter (and
            // while fn's grace period runs).
            let tick = if self.listening.is_some() || self.fn_pending.is_some() {
                Duration::from_millis(50)
            } else {
                Duration::from_millis(250)
            };
            match rx.recv_timeout(tick) {
                Ok(Msg::Pressed) => match &self.listening {
                    None => self.start(Instant::now()),
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
                Ok(Msg::FnDown) => match &self.listening {
                    // A tap started it: this press stops it.
                    Some(l) if l.toggled => self.stop(),
                    Some(_) => {}
                    None => self.fn_pending = Some(Instant::now()),
                },
                Ok(Msg::FnUp) => {
                    if let Some(since) = self.fn_pending.take() {
                        // Released within the grace period: a tap, which
                        // starts hands-free listening.
                        self.start(since);
                        if let Some(l) = &mut self.listening {
                            l.toggled = true;
                        }
                    } else if let Some(l) = &mut self.listening
                        && !l.toggled
                    {
                        if l.since.elapsed() >= HOLD {
                            self.stop();
                        } else {
                            l.toggled = true;
                        }
                    }
                }
                Ok(Msg::FnCombo) => {
                    // Within the grace period nothing started; after it,
                    // drop the dictation this press began.
                    if self.fn_pending.take().is_none()
                        && self.listening.as_ref().is_some_and(|l| !l.toggled)
                    {
                        self.cancel();
                    }
                }
                Ok(Msg::Register(reply)) => {
                    let _ = reply.send(self.register());
                }
                Err(RecvTimeoutError::Timeout) if self.fn_pending.is_some() => {
                    if let Some(since) = self.fn_pending
                        && since.elapsed() >= FN_GRACE
                    {
                        self.fn_pending = None;
                        self.start(since);
                    }
                }
                Err(RecvTimeoutError::Timeout) => match &self.listening {
                    Some(l) if l.since.elapsed() >= MAX_LISTEN => self.stop(),
                    Some(_) => {
                        if let Some(mic) = &self.mic {
                            indicator::level(&self.app, mic.level());
                        }
                    }
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
        self.set_fn_key(false);
        let shortcuts = self.app.global_shortcut();
        if let Some(old) = lock(&self.shared.hotkey).take() {
            let _ = shortcuts.unregister(old);
        }
        if !settings.enabled {
            self.mic = None;
            return Ok(());
        }
        if settings.hotkey == FN_HOTKEY {
            return self.use_fn_key();
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

    /// Starts listening; `since` is when the key went down.
    fn start(&mut self, since: Instant) {
        // Never start a 1.5 GB download silently from a key press: start it,
        // and bring up the Dictation page, where its progress shows.
        let settings = lock(&self.app.state::<AppState>().settings).clone();
        let missing = crate::models::missing_for_dictation(&settings);
        if !missing.is_empty() {
            tracing::info!("dictation needs models that aren't downloaded yet");
            crate::models::download_in_background(&self.app, missing);
            crate::tray::show_window(&self.app);
            let _ = self.app.emit(crate::NAVIGATE_EVENT, "dictation");
            // The Dictation page shows the download's progress.
            cue::play(Cue::Cancel);
            return;
        }
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
        if settings.dictation.live_captions {
            indicator::caption(&self.app, &Caption::default());
            self.live = Some(LiveWorker::spawn(self.app.clone(), mic.tap(), settings));
        }
        cue::play(Cue::Start);
        *lock(&self.shared.error) = None;
        self.listening = Some(Listening {
            since,
            toggled: false,
            app: quill_insert::frontmost_app(),
        });
        self.shared.listening.store(true, Ordering::SeqCst);
        // Esc cancels, but only while listening: a global Esc would steal it
        // from every other app.
        if let Err(e) = self.app.global_shortcut().register(escape()) {
            tracing::debug!("couldn't bind Esc to cancel: {e}");
        }
        notify(&self.app, &self.shared);
    }

    /// Watches the fn key (starting the watcher the first time).
    #[cfg(target_os = "macos")]
    fn use_fn_key(&mut self) -> Result<(), String> {
        if self.fn_key.is_none() {
            let tx = self.tx.clone();
            let key = quill_insert::fn_key::FnKey::watch(move |event| {
                use quill_insert::fn_key::FnEvent;
                let _ = tx.send(match event {
                    FnEvent::Down => Msg::FnDown,
                    FnEvent::Up => Msg::FnUp,
                    FnEvent::Combo => Msg::FnCombo,
                });
            })
            .map_err(|e| e.to_string())?;
            self.fn_key = Some(key);
        }
        self.set_fn_key(true);
        tracing::info!("dictation uses the fn key");
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    fn use_fn_key(&mut self) -> Result<(), String> {
        Err("the fn key can only be the dictation shortcut on macOS".into())
    }

    fn set_fn_key(&mut self, on: bool) {
        #[cfg(target_os = "macos")]
        if let Some(key) = &self.fn_key {
            key.set_active(on);
        }
        self.fn_pending = None;
        self.shared
            .fn_watching
            .store(on && cfg!(target_os = "macos"), Ordering::SeqCst);
    }

    fn stop(&mut self) {
        let Some(listening) = self.end_listening() else {
            return;
        };
        // Dropped on the early returns below, which stops it.
        let live = self.live.take();
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
        let target = listening.app;
        tauri::async_runtime::spawn(async move {
            let stopped = Instant::now();
            let result = process(&app, pcm, audio, stopped, target, live).await;
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
        // It finishes the pass it's on and ends; nothing waits for it.
        self.live = None;
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

/// How often captions update while listening: a pass over the unlocked
/// audio takes ~0.2 s on turbo (eval/dictation/README.md).
const LIVE_STEP: Duration = Duration::from_millis(600);

/// Live captions (Settings → Dictation): on its own thread, transcribes what's
/// been said so far every [`LIVE_STEP`] and shows it above the indicator.
/// Handed back on release, it leaves only the end to transcribe. Dropping it
/// stops it.
struct LiveWorker {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<Option<LiveTranscript>>>,
}

impl LiveWorker {
    fn spawn(app: AppHandle, tap: MicTap, settings: Settings) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let handle = std::thread::spawn(move || {
            let transcriber = match pipeline::transcriber(&app, None, &settings) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("no live captions: {e}");
                    return None;
                }
            };
            let mut live = LiveTranscript::new();
            while !stopping.load(Ordering::SeqCst) {
                let started = Instant::now();
                let pcm = tap.so_far().map_err(|e| e.to_string());
                let caption = pcm.and_then(|pcm| {
                    live.update(transcriber.as_ref(), &pcm, &settings.dictation)
                        .map_err(|e| e.to_string())
                });
                match caption {
                    Ok(caption) if !stopping.load(Ordering::SeqCst) => {
                        indicator::caption(&app, &caption);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        // The release does a single pass instead.
                        tracing::warn!("live captions stopped: {e}");
                        return None;
                    }
                }
                while started.elapsed() < LIVE_STEP && !stopping.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            Some(live)
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }

    /// Stops, waiting for a pass in progress, and hands back what's locked
    /// in (`None` if it failed).
    fn finish(mut self) -> Option<LiveTranscript> {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.take()?.join().ok().flatten()
    }
}

impl Drop for LiveWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
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

/// Whisper → polish (if useful) → paste → history (and a sample, if the
/// user keeps them).
async fn process(
    app: &AppHandle,
    pcm: Vec<f32>,
    audio: Duration,
    stopped: Instant,
    target: Option<quill_insert::FrontApp>,
    live: Option<LiveWorker>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let settings = lock(&state.settings).clone();
    let sample_pcm = settings.dictation.save_samples.then(|| pcm.clone());

    // With live captions, most of it is already transcribed: only the end
    // is left. Otherwise, or if they failed, one pass over it all.
    let (raw, live_progress) = {
        let app = app.clone();
        let settings = settings.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let transcriber = pipeline::transcriber(&app, None, &settings)?;
            let live = live.and_then(LiveWorker::finish);
            let raw = match &live {
                Some(live) => live.finish(transcriber.as_ref(), &pcm, &settings.dictation),
                None => dictation::transcribe(transcriber.as_ref(), &pcm, &settings.dictation),
            }
            .map_err(|e| e.to_string())?;
            if live.is_some() {
                // All of it, while clean-up runs.
                indicator::caption(
                    &app,
                    &Caption {
                        committed: raw.clone(),
                        tentative: String::new(),
                    },
                );
            }
            Ok::<_, String>((raw, live.map(|l| l.progress())))
        })
        .await
        .map_err(|e| e.to_string())??
    };
    let stt_ms = stopped.elapsed().as_millis();
    if raw.is_empty() {
        tracing::debug!(stt_ms, "no speech");
        return Ok(());
    }

    let style = target.as_ref().map_or(Style::Default, |app| {
        dictation::style_for(&app.id, &settings.dictation)
    });
    let destination = target.as_ref().map(|app| dictation::Destination {
        app: &app.name,
        style,
    });
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
                    destination,
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
    let text = dictation::apply_style(&text, style);

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
    let (live_passes, live_locked_ms) = live_progress.unwrap_or_default();
    tracing::info!(
        stt_ms,
        latency_ms,
        polished = polish_model.is_some(),
        live_passes,
        live_locked_ms,
        ?inserted,
        "dictation typed"
    );

    if let Some(pcm) = sample_pcm {
        let dir = samples::dir(&state.data_dir);
        let meta = samples::Meta {
            app_id: target.as_ref().map(|a| a.id.clone()),
            app: target.as_ref().map(|a| a.name.clone()),
            stt_model: settings.stt_model.clone(),
            polish_model: polish_model.clone(),
            whisper: raw.clone(),
        };
        let typed = text.clone();
        let app = app.clone();
        // In the background: the text is already typed.
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = samples::save(&dir, &pcm, &typed, &meta) {
                tracing::warn!("couldn't save the dictation sample: {e}");
            }
            if let Ok(usage) = samples::usage(&dir) {
                let _ = app.emit(SAMPLES_EVENT, usage);
            }
        });
    }

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
        app: target.as_ref().map(|a| a.name.clone()),
        app_id: target.as_ref().map(|a| a.id.clone()),
        corrected: None,
    };
    // Numbers only (no text), for the Insights page, whatever the history
    // setting.
    let stat = DictationStat {
        created_at: jiff::Timestamp::now().to_string(),
        words: u64::try_from(entry.text.split_whitespace().count()).unwrap_or(u64::MAX),
        audio_ms: entry.audio_ms,
        latency_ms,
        polished: entry.polish_model.is_some(),
        pasted,
        app_id: target.as_ref().map(|a| a.id.clone()),
        app: target.as_ref().map(|a| a.name.clone()),
    };
    if let Err(e) = state.library.record_dictation_stats(&stat) {
        tracing::warn!("couldn't record dictation stats: {e}");
    }
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

/// A page of the dictation history, newest first: up to `limit` entries
/// (at most 100) older than the entry `before`, or the newest if `None`.
#[tauri::command]
pub fn list_dictations(
    state: tauri::State<'_, AppState>,
    before: Option<i64>,
    limit: usize,
) -> CommandResult<Vec<DictationEntry>> {
    Ok(state.library.dictations(before, limit.min(100))?)
}

/// Words, pace, time saved and what clean-up changed for one dictation,
/// with the user's usual speaking speed to compare against.
#[tauri::command]
pub async fn dictation_stats(
    app: AppHandle,
    id: i64,
) -> CommandResult<quill_core::stats::DictationStats> {
    blocking(move || {
        let state = app.state::<AppState>();
        let entry = state.library.dictation(id)?;
        let usual = quill_core::insights::compute(&state.library, &jiff::Zoned::now())?
            .dictation
            .speaking_wpm;
        Ok(quill_core::stats::dictation(&entry, usual))
    })
    .await
}

#[tauri::command]
pub fn get_dictation(state: tauri::State<'_, AppState>, id: i64) -> CommandResult<DictationEntry> {
    Ok(state.library.dictation(id)?)
}

#[tauri::command]
pub fn delete_dictation(state: tauri::State<'_, AppState>, id: i64) -> CommandResult<()> {
    Ok(state.library.delete_dictation(id)?)
}

#[tauri::command]
pub fn clear_dictations(state: tauri::State<'_, AppState>) -> CommandResult<()> {
    Ok(state.library.clear_dictations()?)
}

/// How many dictation samples are saved (Dictation → Testing accuracy), and
/// how much space they take.
#[tauri::command]
pub async fn dictation_samples(app: AppHandle) -> CommandResult<samples::Usage> {
    blocking(move || {
        let dir = samples::dir(&app.state::<AppState>().data_dir);
        samples::usage(&dir).map_err(|e| CommandError::Other(e.to_string()))
    })
    .await
}

/// Opens the folder of dictation samples.
#[tauri::command]
pub fn show_dictation_samples(state: tauri::State<'_, AppState>) -> CommandResult<()> {
    let dir = samples::dir(&state.data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| CommandError::Other(e.to_string()))?;
    tauri_plugin_opener::open_path(dir, None::<&str>)
        .map_err(|e| CommandError::Other(e.to_string()))
}

/// Deletes every dictation sample, with any references written for them.
#[tauri::command]
pub async fn clear_dictation_samples(app: AppHandle) -> CommandResult<()> {
    blocking(move || {
        let dir = samples::dir(&app.state::<AppState>().data_dir);
        samples::clear(&dir).map_err(|e| CommandError::Other(e.to_string()))
    })
    .await
}

/// The samples to review, and how accurate the reviewed ones were.
#[derive(Serialize)]
pub struct SampleReview {
    samples: Vec<samples::Sample>,
    accuracy: samples::Accuracy,
}

#[tauri::command]
pub async fn sample_review(app: AppHandle) -> CommandResult<SampleReview> {
    blocking(move || {
        let dir = samples::dir(&app.state::<AppState>().data_dir);
        let list = samples::list(&dir).map_err(|e| CommandError::Other(e.to_string()))?;
        let accuracy = samples::accuracy(&list);
        Ok(SampleReview {
            samples: list,
            accuracy,
        })
    })
    .await
}

/// A sample's audio (FLAC), to play while reviewing it.
#[tauri::command]
pub async fn sample_audio(app: AppHandle, name: String) -> CommandResult<tauri::ipc::Response> {
    blocking(move || {
        let dir = samples::dir(&app.state::<AppState>().data_dir);
        let path =
            samples::audio_path(&dir, &name).map_err(|e| CommandError::Other(e.to_string()))?;
        let bytes = std::fs::read(path).map_err(|e| CommandError::Other(e.to_string()))?;
        Ok(tauri::ipc::Response::new(bytes))
    })
    .await
}

/// Saves what should have been typed for a sample.
#[tauri::command]
pub async fn set_sample_reference(app: AppHandle, name: String, text: String) -> CommandResult<()> {
    blocking(move || {
        let dir = samples::dir(&app.state::<AppState>().data_dir);
        samples::set_reference(&dir, &name, &text).map_err(|e| CommandError::Other(e.to_string()))
    })
    .await
}

/// Deletes one sample.
#[tauri::command]
pub async fn delete_sample(app: AppHandle, name: String) -> CommandResult<()> {
    blocking(move || {
        let dir = samples::dir(&app.state::<AppState>().data_dir);
        samples::remove(&dir, &name).map_err(|e| CommandError::Other(e.to_string()))
    })
    .await
}

/// Whether Quill may simulate the paste keystroke (macOS Accessibility).
#[tauri::command]
pub async fn paste_permission(app: AppHandle) -> CommandResult<bool> {
    blocking(move || {
        let allowed = quill_insert::can_simulate_paste(false);
        // The fn key needs the same permission: once it's granted, start
        // watching fn if that's the shortcut.
        if allowed {
            app.state::<Dictation>().retry_fn_key(&app);
        }
        Ok(allowed)
    })
    .await
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

/// An app dictated into, with the style in effect there.
#[derive(Debug, Clone, Serialize)]
pub struct AppStyleInfo {
    pub app_id: String,
    pub name: String,
    pub style: Style,
    /// The style Quill would use without a choice from the user.
    pub built_in: Style,
    pub words: u64,
}

/// Apps dictated into (most used first), plus any with a chosen style.
#[tauri::command]
pub fn dictation_apps(state: tauri::State<'_, AppState>) -> CommandResult<Vec<AppStyleInfo>> {
    let settings = lock(&state.settings).dictation.clone();
    let mut apps: Vec<AppStyleInfo> = state
        .library
        .dictation_apps()?
        .into_iter()
        .map(|a| AppStyleInfo {
            style: dictation::style_for(&a.app_id, &settings),
            built_in: dictation::built_in_style(&a.app_id.to_lowercase()),
            app_id: a.app_id,
            name: a.name,
            words: a.words,
        })
        .collect();
    for chosen in &settings.app_styles {
        if !apps
            .iter()
            .any(|a| a.app_id.eq_ignore_ascii_case(&chosen.app_id))
        {
            apps.push(AppStyleInfo {
                app_id: chosen.app_id.clone(),
                name: chosen.name.clone(),
                style: chosen.style,
                built_in: dictation::built_in_style(&chosen.app_id.to_lowercase()),
                words: 0,
            });
        }
    }
    Ok(apps)
}

/// The language dictation uses when none is set, e.g. "en".
#[tauri::command]
pub fn system_language() -> Option<String> {
    dictation::language(None)
}

/// What macOS does on its own when fn / 🌐 is pressed (Keyboard settings →
/// "Press 🌐 key to"), which also happens when fn triggers dictation.
/// `None` off macOS.
#[tauri::command]
pub async fn globe_key_action() -> CommandResult<Option<String>> {
    #[cfg(target_os = "macos")]
    {
        blocking(|| {
            let action = quill_insert::fn_key::globe_action();
            Ok(serde_json::to_value(action)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned)))
        })
        .await
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(None)
    }
}

/// Opens macOS Keyboard settings, where "Press 🌐 key to" is.
#[tauri::command]
pub fn open_keyboard_settings() -> CommandResult<()> {
    tauri_plugin_opener::open_url(
        "x-apple.systempreferences:com.apple.Keyboard-Settings.extension",
        None::<&str>,
    )
    .map_err(|e| CommandError::Other(e.to_string()))
}

/// A saved correction and what it suggests learning.
#[derive(Debug, Clone, Serialize)]
pub struct Correction {
    pub entry: DictationEntry,
    /// All pending and accepted suggestions, the new ones included.
    pub suggestions: Vec<Suggestion>,
}

/// Saves the user's fixed version of a dictation and records the rules it
/// suggests (see `quill_core::corrections`). Nothing is applied until the
/// user accepts a suggestion.
#[tauri::command]
pub async fn correct_dictation(app: AppHandle, id: i64, text: String) -> CommandResult<Correction> {
    blocking(move || {
        let state = app.state::<AppState>();
        let text = text.trim();
        if text.is_empty() {
            return Err(CommandError::Other("the corrected text is empty".into()));
        }
        let entry = state.library.correct_dictation(id, text)?;
        let settings = lock(&state.settings).dictation.clone();
        let rules: Vec<_> = quill_core::corrections::learn(&entry.text, text)
            .into_iter()
            .filter(|rule| !already_known(rule, &settings))
            .collect();
        tracing::info!(suggestions = rules.len(), "dictation corrected");
        state
            .library
            .add_suggestions(&rules, entry.app.as_deref())?;
        Ok(Correction {
            entry,
            suggestions: state.library.suggestions()?,
        })
    })
    .await
}

/// Whether the settings already have this rule.
fn already_known(rule: &quill_core::corrections::Learned, settings: &DictationSettings) -> bool {
    use quill_core::corrections::Kind;
    match rule.kind {
        Kind::Vocabulary => settings.vocabulary.iter().any(|v| v.trim() == rule.to),
        Kind::Replacement => settings
            .replacements
            .iter()
            .any(|r| r.from.trim().eq_ignore_ascii_case(&rule.from) && r.to.trim() == rule.to),
    }
}

/// Pending and accepted suggestions.
#[tauri::command]
pub fn list_suggestions(state: tauri::State<'_, AppState>) -> CommandResult<Vec<Suggestion>> {
    Ok(state.library.suggestions()?)
}

/// Marks a suggestion accepted (the UI adds the rule to the settings) or
/// dismissed (it won't be suggested again).
#[tauri::command]
pub fn set_suggestion_status(
    state: tauri::State<'_, AppState>,
    id: i64,
    status: SuggestionStatus,
) -> CommandResult<Vec<Suggestion>> {
    state.library.set_suggestion_status(id, status)?;
    Ok(state.library.suggestions()?)
}
