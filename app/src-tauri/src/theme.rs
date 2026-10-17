//! The app's theme (Settings → General → Appearance): the native window
//! appearance (title bar, scrollbars, menus) is set here, and each page sets
//! its own colours from the setting and the [`EVENT`] (see `main.tsx`).

use quill_core::settings::Theme;
use tauri::{AppHandle, Emitter, Manager};

/// Sent to every window when the theme changes; carries the [`Theme`].
pub const EVENT: &str = "theme";

/// The native theme for a setting: `None` follows the OS.
pub fn native(theme: Theme) -> Option<tauri::Theme> {
    match theme {
        Theme::System => None,
        Theme::Light => Some(tauri::Theme::Light),
        Theme::Dark => Some(tauri::Theme::Dark),
    }
}

/// Applies `theme` to every open window.
pub fn apply(app: &AppHandle, theme: Theme) {
    for (label, window) in app.webview_windows() {
        if let Err(e) = window.set_theme(native(theme)) {
            tracing::warn!(label, "couldn't set the window theme: {e}");
        }
    }
    let _ = app.emit(EVENT, theme);
}
