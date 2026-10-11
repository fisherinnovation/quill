//! Menu-bar (macOS) / system-tray (Windows) icon: start and stop recording
//! without opening the window, with a live timer while recording.

use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Wry};

use crate::AppState;
use crate::commands::{self, lock};

const TOGGLE: &str = "toggle-recording";
const OPEN: &str = "open";
const QUIT: &str = "quit";

/// Handles to update when recording starts or stops.
pub struct Tray {
    icon: TrayIcon,
    toggle: MenuItem<Wry>,
}

pub fn create(app: &AppHandle) -> tauri::Result<Tray> {
    let toggle = MenuItem::with_id(app, TOGGLE, "Start Recording", true, None::<&str>)?;
    let open = MenuItem::with_id(app, OPEN, "Open Quill", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, "Quit Quill", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&toggle, &PredefinedMenuItem::separator(app)?, &open, &quit],
    )?;

    let icon = TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
        // A template icon is drawn by macOS in the menu bar's colour, so it
        // fits light and dark menu bars.
        .icon_as_template(true)
        .tooltip("Quill")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            TOGGLE => {
                // Off the main thread, for the same reason as the commands.
                let app = app.clone();
                tauri::async_runtime::spawn_blocking(move || {
                    let recording = lock(&app.state::<AppState>().recording).is_some();
                    let result = if recording {
                        commands::stop(&app).map(|_| ())
                    } else {
                        commands::start(&app).map(|_| ())
                    };
                    if let Err(e) = result {
                        tracing::error!("recording toggle failed: {e}");
                        show_window(&app);
                    }
                });
            }
            OPEN => show_window(app),
            QUIT => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(Tray { icon, toggle })
}

/// Reflects the recording state in the menu and menu-bar title.
pub fn set_recording(app: &AppHandle, recording: bool) {
    let state = app.state::<AppState>();
    let tray = lock(&state.tray);
    let Some(tray) = tray.as_ref() else {
        return;
    };
    let _ = tray.toggle.set_text(if recording {
        "Stop Recording"
    } else {
        "Start Recording"
    });
    let _ = tray.icon.set_title(recording.then_some("● 0:00"));
    if recording {
        spawn_timer(app.clone());
    }
}

/// Shows dictation in the menu-bar title, unless a recording's timer is
/// using it.
pub fn set_dictation(app: &AppHandle, phase: crate::dictation::Phase) {
    use crate::dictation::Phase;
    let state = app.state::<AppState>();
    if lock(&state.recording).is_some() {
        return;
    }
    if let Some(tray) = lock(&state.tray).as_ref() {
        let _ = tray.icon.set_title(match phase {
            Phase::Listening => Some("● Listening"),
            Phase::Processing => Some("…"),
            Phase::Idle => None,
        });
    }
}

/// Updates the "● m:ss" title once a second until recording stops.
fn spawn_timer(app: AppHandle) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let state = app.state::<AppState>();
            let Some(elapsed) = lock(&state.recording).as_ref().map(|r| r.started.elapsed()) else {
                return;
            };
            let secs = elapsed.as_secs();
            let title = format!("● {}:{:02}", secs / 60, secs % 60);
            if let Some(tray) = lock(&state.tray).as_ref() {
                let _ = tray.icon.set_title(Some(title));
            }
        }
    });
}

pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
