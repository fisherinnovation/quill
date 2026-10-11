//! Types text into whatever app has focus, for dictation.
//!
//! There's no portable "type this string" API that works everywhere (and
//! simulating one keystroke per character is slow and breaks on non-ASCII and
//! keyboard layouts), so we paste: put the text on the clipboard, press
//! Cmd+V (Ctrl+V on Windows), then put back what the clipboard held before.
//!
//! The text is marked so clipboard managers don't record it (macOS
//! `org.nspasteboard.ConcealedType`, Windows "exclude from history").
//!
//! Use [`Clipboard::put`], [`press_paste`] (on the main thread on macOS),
//! then [`Clipboard::restore`].
//!
//! macOS: simulating keys needs the Accessibility permission. Without it the
//! keystroke is silently dropped, so check [`can_simulate_paste`] first and
//! fall back to leaving the text on the clipboard.

mod app;

pub use app::{FrontApp, frontmost_app};

use std::time::Duration;

use enigo::{Direction, Enigo, Key, Keyboard, Settings};

#[derive(Debug, thiserror::Error)]
pub enum InsertError {
    #[error("clipboard: {0}")]
    Clipboard(#[from] arboard::Error),
    #[error(
        "Quill isn't allowed to simulate a paste; enable it under Privacy & Security → Accessibility"
    )]
    NoPermission,
    #[error("simulating the paste failed: {0}")]
    Input(String),
}

/// How long the target app gets to read the clipboard after the paste
/// keystroke before we restore the previous contents. Apps read it when they
/// handle the key event, normally within a few ms; slow Electron apps can
/// take longer.
const RESTORE_AFTER: Duration = Duration::from_millis(400);

/// What happened to the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inserted {
    /// Pasted into the focused app; the old clipboard was restored.
    Pasted,
    /// Couldn't simulate the paste: the text is on the clipboard for the user
    /// to paste themselves.
    OnClipboard,
}

/// Whether simulated keystrokes will work (macOS: the Accessibility
/// permission). With `prompt`, macOS shows its "allow in System Settings"
/// dialog if not. Always true elsewhere.
pub fn can_simulate_paste(prompt: bool) -> bool {
    Enigo::new(&Settings {
        open_prompt_to_get_permissions: prompt,
        ..Settings::default()
    })
    .is_ok()
}

/// Pastes `text` into the focused app: [`Clipboard::put`], then
/// [`press_paste`] **on the main thread**, then [`Clipboard::restore`].
///
/// Split in three because of threads: on macOS, finding which key types "v"
/// on the user's keyboard layout calls the Text Input Sources API, which
/// aborts the process if it isn't on the main thread (macOS 26). The other
/// two steps wait or touch the clipboard and belong on a worker thread.
pub struct Clipboard {
    clipboard: arboard::Clipboard,
    previous: Saved,
    text: String,
}

impl Clipboard {
    /// Saves what's on the clipboard and replaces it with `text`.
    pub fn put(text: &str) -> Result<Self, InsertError> {
        let mut clipboard = arboard::Clipboard::new()?;
        let previous = Saved::take(&mut clipboard);
        set_concealed(&mut clipboard, text)?;
        Ok(Self {
            clipboard,
            previous,
            text: text.to_string(),
        })
    }

    /// Waits for the target app to read the clipboard, then puts back what
    /// was there before, unless something else changed it meanwhile (the
    /// user copied). Blocks for about half a second.
    pub fn restore(mut self) {
        std::thread::sleep(RESTORE_AFTER);
        if self.clipboard.get_text().is_ok_and(|now| now == self.text) {
            self.previous.restore(&mut self.clipboard);
        }
    }
}

/// Presses Cmd+V (Ctrl+V on Windows). On macOS, call it on the main thread
/// (see [`Clipboard`]). Returns [`Inserted::OnClipboard`] without pressing
/// anything if there's no permission to simulate keys.
pub fn press_paste() -> Result<Inserted, InsertError> {
    let mut enigo = match Enigo::new(&Settings {
        open_prompt_to_get_permissions: false,
        ..Settings::default()
    }) {
        Ok(enigo) => enigo,
        Err(enigo::NewConError::NoPermission) => return Ok(Inserted::OnClipboard),
        Err(e) => return Err(InsertError::Input(e.to_string())),
    };
    paste(&mut enigo).map_err(|e| InsertError::Input(e.to_string()))?;
    Ok(Inserted::Pasted)
}

fn paste(enigo: &mut Enigo) -> enigo::InputResult<()> {
    let modifier = if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    };
    enigo.key(modifier, Direction::Press)?;
    let result = enigo.key(Key::Unicode('v'), Direction::Click);
    // Release the modifier even if the click failed, or it stays "held".
    enigo.key(modifier, Direction::Release)?;
    result
}

#[cfg(target_os = "macos")]
fn set_concealed(clipboard: &mut arboard::Clipboard, text: &str) -> Result<(), arboard::Error> {
    use arboard::SetExtApple;
    clipboard.set().exclude_from_history().text(text)
}

#[cfg(windows)]
fn set_concealed(clipboard: &mut arboard::Clipboard, text: &str) -> Result<(), arboard::Error> {
    use arboard::SetExtWindows;
    clipboard
        .set()
        .exclude_from_history()
        .exclude_from_cloud()
        .text(text)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn set_concealed(clipboard: &mut arboard::Clipboard, text: &str) -> Result<(), arboard::Error> {
    clipboard.set_text(text)
}

/// The clipboard's previous contents, as far as we can restore them: text or
/// an image. Anything else (files, rich text only) can't be put back, so
/// it's left as our text rather than emptied.
enum Saved {
    Text(String),
    Image(arboard::ImageData<'static>),
    Nothing,
}

impl Saved {
    fn take(clipboard: &mut arboard::Clipboard) -> Self {
        if let Ok(text) = clipboard.get_text() {
            Self::Text(text)
        } else if let Ok(image) = clipboard.get_image() {
            Self::Image(image)
        } else {
            Self::Nothing
        }
    }

    fn restore(self, clipboard: &mut arboard::Clipboard) {
        let result = match self {
            Self::Text(text) => clipboard.set_text(text),
            Self::Image(image) => clipboard.set_image(image),
            Self::Nothing => return,
        };
        if let Err(e) = result {
            tracing::warn!("could not restore the clipboard: {e}");
        }
    }
}
