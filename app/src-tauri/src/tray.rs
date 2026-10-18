//! Menu-bar (macOS) / system-tray (Windows) icon: start and stop recording
//! without opening the window, with a live timer while recording.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Wry};

use crate::AppState;
use crate::commands::{self, lock};

const TOGGLE: &str = "toggle-recording";
const PAUSE: &str = "pause-recording";
const OPEN: &str = "open";
const ABOUT: &str = "tray-about";
const QUIT: &str = "quit";

/// The logo's feather and pulse line (app/scripts/tray_icon.py): black on
/// transparent for a macOS template icon; on Windows, where a black glyph
/// would vanish on a dark taskbar, the full-colour logo.
#[cfg(target_os = "macos")]
const ICON: &[u8] = include_bytes!("../icons/tray.png");
#[cfg(not(target_os = "macos"))]
const ICON: &[u8] = include_bytes!("../icons/tray-windows.png");

/// Handles to update when recording starts or stops.
pub struct Tray {
    icon: TrayIcon,
    toggle: MenuItem<Wry>,
    pause: MenuItem<Wry>,
}

/// Set while the menu-bar timer is running, so pausing and resuming don't
/// start another.
static TIMER: AtomicBool = AtomicBool::new(false);

pub fn create(app: &AppHandle) -> tauri::Result<Tray> {
    let toggle = MenuItem::with_id(app, TOGGLE, "Start Recording", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, PAUSE, "Pause Recording", false, None::<&str>)?;
    let open = MenuItem::with_id(app, OPEN, "Open Quill", true, None::<&str>)?;
    let about = MenuItem::with_id(app, ABOUT, "About Quill", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, "Quit Quill", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &toggle,
            &pause,
            &PredefinedMenuItem::separator(app)?,
            &open,
            &about,
            &quit,
        ],
    )?;

    let icon = TrayIconBuilder::with_id("main")
        .icon(Image::from_bytes(ICON)?)
        // A template icon is drawn by macOS in the menu bar's colour, so it
        // fits light and dark menu bars. (Ignored elsewhere.)
        .icon_as_template(cfg!(target_os = "macos"))
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
            PAUSE => {
                let paused = lock(&app.state::<AppState>().recording)
                    .as_ref()
                    .is_some_and(crate::recorder::ActiveRecording::is_paused);
                if let Err(e) = commands::set_paused(app, !paused) {
                    tracing::warn!("pause from the menu bar: {e}");
                }
            }
            OPEN => show_window(app),
            ABOUT => crate::about::show(app),
            QUIT => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(Tray {
        icon,
        toggle,
        pause,
    })
}

/// Reflects the recording state in the menu and menu-bar title.
pub fn set_recording(app: &AppHandle, recording: bool, paused: bool) {
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
    let _ = tray.pause.set_enabled(recording);
    let _ = tray.pause.set_text(if paused {
        "Resume Recording"
    } else {
        "Pause Recording"
    });
    if recording && !TIMER.swap(true, Ordering::SeqCst) {
        let _ = tray.icon.set_title(Some("● 0:00"));
        spawn_timer(app.clone());
    } else if !recording {
        let _ = tray.icon.set_title(None::<&str>);
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

/// Updates the "● m:ss" title (❚❚ while paused) until recording stops.
fn spawn_timer(app: AppHandle) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let state = app.state::<AppState>();
            let Some((elapsed, paused)) = lock(&state.recording)
                .as_ref()
                .map(|r| (r.elapsed(), r.is_paused()))
            else {
                TIMER.store(false, Ordering::SeqCst);
                return;
            };
            let secs = elapsed.as_secs();
            let mark = if paused { "❚❚" } else { "●" };
            let title = format!("{mark} {}:{:02}", secs / 60, secs % 60);
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
