//! The floating dictation indicator: a small pill near the bottom of the
//! screen while dictation listens ("● ▂▅▇▅▂" following the mic level) or
//! types ("Typing…"), with captions of what's been said above it.
//!
//! It's a second, transparent window that can never take focus: the text is
//! pasted into whatever app has focus, so the indicator must not be it. It
//! also ignores the mouse, so clicks go to whatever is underneath.

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

use crate::dictation::Phase;

pub const LABEL: &str = "indicator";
/// Mic level for the indicator's bars, ~20 times a second while listening.
pub const LEVEL_EVENT: &str = "dictation-level";
/// What's been said so far (`quill_core::live::Caption`), for the captions
/// above the pill.
pub const CAPTION_EVENT: &str = "dictation-caption";

/// Logical size; the page draws the pill at the bottom centre and the
/// captions above it. The rest is transparent and ignores the mouse.
const WIDTH: f64 = 640.0;
const HEIGHT: f64 = 160.0;
/// Gap above the bottom of the usable screen area (above the Dock).
const MARGIN: f64 = 28.0;

/// Creates the (hidden) indicator window, so showing it later is instant.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Quill dictation")
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
    window.set_ignore_cursor_events(true)?;
    Ok(())
}

/// Shows the indicator while listening or typing, hides it otherwise.
pub fn update(app: &AppHandle, phase: Phase) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let result = if phase == Phase::Idle {
        window.hide()
    } else {
        // Place it when it appears, not on every change, so it doesn't jump
        // between listening and typing.
        if !window.is_visible().unwrap_or(false) {
            place(app, &window);
        }
        window.show()
    };
    if let Err(e) = result {
        tracing::debug!("indicator: {e}");
    }
}

/// The mic level while listening: for the pill's meter, and the main
/// window's Dictations page, whose waveform follows the voice.
pub fn level(app: &AppHandle, level: f32) {
    let _ = app.emit_to(LABEL, LEVEL_EVENT, level);
    let _ = app.emit_to("main", LEVEL_EVENT, level);
}

pub fn caption(app: &AppHandle, caption: &quill_core::live::Caption) {
    let _ = app.emit_to(LABEL, CAPTION_EVENT, caption);
}

/// Bottom centre of the screen the mouse is on (where the user is working).
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
