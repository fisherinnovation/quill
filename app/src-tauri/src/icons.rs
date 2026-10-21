//! App icons for the interface: beside each dictation, the top apps and the
//! per-app styles. Read from the installed apps (`quill_insert::icon`), kept
//! in memory, never written to disk.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use base64::Engine;

use crate::commands::{CommandResult, blocking, lock};

/// Data URLs by app id; `None` for an app with no icon to show.
static CACHE: LazyLock<Mutex<HashMap<String, Option<String>>>> = LazyLock::new(Default::default);

/// Icons for these apps (`FrontApp::id`s) as PNG data URLs, `null` for one
/// that can't be found (uninstalled; on Windows, not in front since Quill
/// started). Looking one up the first time can take a moment.
#[tauri::command]
pub async fn app_icons(ids: Vec<String>) -> CommandResult<HashMap<String, Option<String>>> {
    blocking(move || {
        let mut icons = HashMap::new();
        for id in ids {
            if let Some(icon) = lock(&CACHE).get(&id) {
                icons.insert(id, icon.clone());
                continue;
            }
            let icon = quill_insert::icon::png(&id).map(|png| {
                format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(png)
                )
            });
            // A miss on Windows may only mean the app hasn't been in front
            // yet this run, so it's asked again next time.
            if icon.is_some() || !cfg!(windows) {
                lock(&CACHE).insert(id.clone(), icon.clone());
            }
            icons.insert(id, icon);
        }
        Ok(icons)
    })
    .await
}
