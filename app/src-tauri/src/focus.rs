//! Handing focus back after a click on one of Quill's floating panels (the
//! call prompt, live notes). They never take focus, but on macOS a click on
//! any of an app's windows still activates the app, bringing Quill's main
//! window forward. So a panel notes which app was in front (the call,
//! usually) and gives it focus back the moment a button is pressed.

use std::sync::Mutex;

use tauri::AppHandle;

use crate::commands::lock;

/// The app to give focus back to, by process id.
#[derive(Default)]
pub struct ReturnFocus(Mutex<Option<i32>>);

impl ReturnFocus {
    /// The app in front now is the one to go back to (none if it's Quill).
    pub fn note_front(&self) {
        *lock(&self.0) = other_frontmost_app();
    }

    /// Like [`note_front`](Self::note_front), but keeps the last one if
    /// Quill is in front (the user may have switched since).
    pub fn update_front(&self) {
        if let Some(pid) = other_frontmost_app() {
            *lock(&self.0) = Some(pid);
        }
    }

    /// Makes that app active again.
    pub fn give_back(&self, app: &AppHandle) {
        #[cfg(target_os = "macos")]
        if let Some(pid) = *lock(&self.0) {
            // AppKit wants the main thread.
            let _ = app.run_on_main_thread(move || {
                quill_insert::refocus(pid);
            });
        }
        #[cfg(not(target_os = "macos"))]
        let _ = app;
    }
}

/// The app in front, unless it's Quill.
fn other_frontmost_app() -> Option<i32> {
    #[cfg(target_os = "macos")]
    {
        let own = i32::try_from(std::process::id()).ok();
        quill_insert::frontmost_pid().filter(|pid| Some(*pid) != own)
    }
    #[cfg(not(target_os = "macos"))]
    None
}
