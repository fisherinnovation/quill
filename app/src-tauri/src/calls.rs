//! Offering to record calls. Twice a second, reads which apps are using a
//! microphone (`quill_audio::mic_users`: bundle ids only) and feeds them to
//! `quill_core::calls::Detector`. When a call starts, a small prompt offers
//! to record it; when it ends while Quill is recording, to stop.
//!
//! The prompt is its own window at the bottom centre of the screen, where
//! it's hard to miss. It takes clicks without taking focus or bringing
//! Quill's main window forward (`floating.rs`), so it doesn't pull the user
//! out of their call.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use quill_core::calls::{CallEvent, Detector};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::AppState;
use crate::commands::{CommandError, CommandResult, blocking, lock};
use crate::floating::Place;

pub const LABEL: &str = "prompt";
/// Sent to the prompt window with what to show ([`Prompt`]).
pub const EVENT: &str = "call-prompt";

/// Logical size; the page draws the card at the bottom of it, with room
/// above for its shadow and its rise as it appears.
const WIDTH: f64 = 400.0;
const HEIGHT: f64 = 160.0;

/// What the prompt is asking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Prompt {
    /// A call started: record it?
    Record { app: String },
    /// The call ended while recording: stop?
    Stop { app: String },
}

/// The prompt showing, if any, and the app to give focus back to after a
/// click on it (the call, usually), by process id.
#[derive(Default)]
pub struct CallPrompt {
    shown: Mutex<Option<Prompt>>,
}

/// How often to look at which apps use the microphone. Cheap (a few Core
/// Audio property reads), and often enough that the prompt isn't late.
const POLL: Duration = Duration::from_millis(500);

/// The fade-out the page plays after a click, before the window hides.
const LEAVE: Duration = Duration::from_millis(180);

/// Creates the (hidden) prompt window, so showing it later is instant.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Quill")
        .inner_size(WIDTH, HEIGHT)
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
        .visible(false)
        .build()?;
    crate::floating::adopt(&window, false);
    Ok(())
}

/// Starts watching for calls. On platforms where Quill can't yet see which
/// apps use the microphone, it stops after logging that once.
pub fn spawn(app: AppHandle) {
    app.manage(CallPrompt::default());
    std::thread::spawn(move || {
        let own = app.config().identifier.clone();
        let mut detector = Detector::new();
        let mut failing = false;
        loop {
            std::thread::sleep(POLL);
            let state = app.state::<AppState>();
            let enabled = {
                let settings = lock(&state.settings);
                settings.offer_to_record_calls && settings.onboarded
            };
            if !enabled {
                detector = Detector::new();
                continue;
            }
            let users = match quill_audio::mic_users::mic_users() {
                Ok(users) => {
                    failing = false;
                    users
                }
                Err(quill_audio::AudioError::Unsupported(what)) => {
                    tracing::info!("not offering to record calls: {what} isn't supported here");
                    return;
                }
                Err(e) => {
                    if !failing {
                        tracing::warn!("can't see which apps use the microphone: {e}");
                        failing = true;
                    }
                    continue;
                }
            };
            let ids = users.iter().filter_map(|u| u.bundle_id.as_deref());
            let recording = lock(&state.recording).is_some();
            match detector.update(Instant::now(), ids, &own) {
                // App names only: never what the call is about.
                Some(CallEvent::Started { app: name }) => {
                    tracing::info!(app = name, recording, "call started");
                    if !recording {
                        show(&app, Prompt::Record { app: name.into() });
                    }
                }
                Some(CallEvent::Ended { app: name }) => {
                    tracing::info!(app = name, recording, "call ended");
                    if recording {
                        show(&app, Prompt::Stop { app: name.into() });
                    } else {
                        hide(&app);
                    }
                }
                None => {}
            }
        }
    });
}

/// Called whenever recording starts or stops: a prompt it answered (from
/// the app or the tray, say) goes away.
pub fn recording_changed(app: &AppHandle, recording: bool) {
    let stale = {
        let Some(prompt) = app.try_state::<CallPrompt>() else {
            return;
        };
        let shown = lock(&prompt.shown);
        matches!(
            (&*shown, recording),
            (Some(Prompt::Record { .. }), true) | (Some(Prompt::Stop { .. }), false)
        )
    };
    if stale {
        hide(app);
    }
}

fn show(app: &AppHandle, prompt: Prompt) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let state = app.state::<CallPrompt>();
    *lock(&state.shown) = Some(prompt.clone());
    let _ = app.emit_to(LABEL, EVENT, Some(prompt));
    crate::floating::show(&window, (WIDTH, HEIGHT), Place::BottomCentre);
}

fn hide(app: &AppHandle) {
    if let Some(prompt) = app.try_state::<CallPrompt>() {
        *lock(&prompt.shown) = None;
    }
    let _ = app.emit_to(LABEL, EVENT, None::<Prompt>);
    if let Some(window) = app.get_webview_window(LABEL) {
        crate::floating::hide(&window);
    }
}

/// What the prompt window should show now (it also gets [`EVENT`]s).
#[tauri::command]
pub fn call_prompt(prompt: tauri::State<'_, CallPrompt>) -> Option<Prompt> {
    lock(&prompt.shown).clone()
}

/// The prompt's buttons: "record", "stop" or "dismiss". The window hides
/// once the page's fade-out has played.
#[tauri::command]
pub async fn answer_call_prompt(app: AppHandle, answer: String) -> CommandResult<()> {
    // Answered: starting or stopping below mustn't hide it before its fade.
    *lock(&app.state::<CallPrompt>().shown) = None;
    blocking(move || {
        let result = match answer.as_str() {
            "record" => crate::commands::start(&app).map(|_| ()),
            "stop" => crate::commands::stop(&app).map(|_| ()),
            "dismiss" => Ok(()),
            other => Err(CommandError::Other(format!("unknown answer {other:?}"))),
        };
        std::thread::sleep(LEAVE);
        hide(&app);
        result
    })
    .await
}
