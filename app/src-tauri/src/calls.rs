//! Offering to record calls. Once a second, reads which apps are using a
//! microphone (`quill_audio::mic_users`: bundle ids only) and feeds them to
//! `quill_core::calls::Detector`. When a call starts, a small prompt offers
//! to record it; when it ends while Quill is recording, to stop.
//!
//! The prompt is its own window at the bottom centre of the screen, where
//! it's hard to miss. It never takes focus, so it doesn't pull the user out
//! of their call, but it does take clicks.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use quill_core::calls::{CallEvent, Detector};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::AppState;
use crate::commands::{CommandError, CommandResult, blocking, lock};

pub const LABEL: &str = "prompt";
/// Sent to the prompt window with what to show ([`Prompt`]).
pub const EVENT: &str = "call-prompt";

/// Logical size; the page draws the card at the bottom of it, with room
/// above for its shadow and its rise as it appears.
const WIDTH: f64 = 400.0;
const HEIGHT: f64 = 160.0;
/// Gap above the bottom of the usable screen area (above the Dock).
const MARGIN: f64 = 16.0;

/// What the prompt is asking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Prompt {
    /// A call started: record it?
    Record { app: String },
    /// The call ended while recording: stop?
    Stop { app: String },
}

/// The prompt showing, if any.
#[derive(Default)]
pub struct CallPrompt(Mutex<Option<Prompt>>);

/// Creates the (hidden) prompt window, so showing it later is instant.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
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
        .visible(false)
        .build()?;
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
            std::thread::sleep(Duration::from_secs(1));
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
        let shown = lock(&prompt.0);
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
    *lock(&app.state::<CallPrompt>().0) = Some(prompt.clone());
    place(app, &window);
    let _ = app.emit_to(LABEL, EVENT, Some(prompt));
    if let Err(e) = window.show() {
        tracing::warn!("couldn't show the call prompt: {e}");
    }
}

fn hide(app: &AppHandle) {
    if let Some(prompt) = app.try_state::<CallPrompt>() {
        *lock(&prompt.0) = None;
    }
    let _ = app.emit_to(LABEL, EVENT, None::<Prompt>);
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
}

/// Bottom centre of the screen the mouse is on, above the Dock.
fn place(app: &AppHandle, window: &tauri::WebviewWindow) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let (Some(monitor), Ok(size)) = (monitor, window.outer_size()) else {
        return;
    };
    let area = monitor.work_area();
    let margin = (MARGIN * monitor.scale_factor()) as i32;
    let x = area.position.x
        + (i32::try_from(area.size.width).unwrap_or(0) - i32::try_from(size.width).unwrap_or(0))
            / 2;
    let y = area.position.y + i32::try_from(area.size.height).unwrap_or(0)
        - i32::try_from(size.height).unwrap_or(0)
        - margin;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// What the prompt window should show now (it also gets [`EVENT`]s).
#[tauri::command]
pub fn call_prompt(prompt: tauri::State<'_, CallPrompt>) -> Option<Prompt> {
    lock(&prompt.0).clone()
}

/// The prompt's buttons: "record", "stop" or "dismiss".
#[tauri::command]
pub async fn answer_call_prompt(app: AppHandle, answer: String) -> CommandResult<()> {
    hide(&app);
    blocking(move || match answer.as_str() {
        "record" => crate::commands::start(&app).map(|_| ()),
        "stop" => crate::commands::stop(&app).map(|_| ()),
        "dismiss" => Ok(()),
        other => Err(CommandError::Other(format!("unknown answer {other:?}"))),
    })
    .await
}
