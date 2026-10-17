//! The About window, and the macOS app menu that opens it.
//!
//! macOS's standard About panel shows only the icon, name and version, so
//! Quill has its own: the build, this computer, the models in use, links,
//! credits, and a button to copy all of it for a bug report.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::AppState;
use crate::commands::{CommandError, CommandResult, lock};

pub const LABEL: &str = "about";
const REPOSITORY: &str = "https://github.com/fisherinnovation/quill";

/// Shows the About window, creating it the first time. Closing it only
/// hides it (see `main`), so it opens instantly after that.
pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let theme = crate::commands::lock(&app.state::<crate::AppState>().settings).theme;
    let builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .theme(crate::theme::native(theme))
        .title("About Quill")
        .inner_size(460.0, 700.0)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        .center();
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    if let Err(e) = builder.build() {
        tracing::warn!("couldn't open the About window: {e}");
    }
}

/// What the About window shows.
#[derive(Debug, Clone, Serialize)]
pub struct AboutInfo {
    pub version: String,
    /// The commit it was built from, and that commit's date; empty if
    /// built outside a git checkout.
    pub commit: String,
    pub commit_date: String,
    /// A debug build (`npm run app`, `app:bundle`) rather than a release.
    pub debug: bool,
    /// e.g. "macOS 26.0".
    pub os: String,
    /// e.g. "Apple Silicon".
    pub arch: String,
    pub speech_model: String,
    /// `None` when clean-up is off.
    pub cleanup_model: Option<String>,
    pub summary_model: String,
    pub identify_speakers: bool,
    pub data_dir: String,
}

#[tauri::command]
pub async fn about_info(app: AppHandle) -> CommandResult<AboutInfo> {
    crate::commands::blocking(move || {
        let state = app.state::<AppState>();
        let settings = lock(&state.settings).clone();
        Ok(AboutInfo {
            version: app.package_info().version.to_string(),
            commit: env!("QUILL_COMMIT").to_string(),
            commit_date: env!("QUILL_COMMIT_DATE").to_string(),
            debug: cfg!(debug_assertions),
            os: os_version(),
            arch: match std::env::consts::ARCH {
                "aarch64" if cfg!(target_os = "macos") => "Apple Silicon".into(),
                "x86_64" if cfg!(target_os = "macos") => "Intel".into(),
                other => other.into(),
            },
            speech_model: settings.stt_model,
            cleanup_model: settings
                .dictation
                .polish
                .then_some(settings.dictation.polish_model),
            summary_model: settings.summary_model,
            identify_speakers: settings.identify_speakers,
            data_dir: state.data_dir.display().to_string(),
        })
    })
    .await
}

fn os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        let version = std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        format!("macOS {version}").trim().to_string()
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::consts::OS.to_string()
    }
}

/// Pages the About window and the Help menu link to. The UI names one;
/// the URL is fixed here, so the web view never opens an arbitrary address.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Link {
    Repository,
    Releases,
    ReportProblem,
    Security,
    License,
    /// Where to create an OpenRouter API key.
    OpenrouterKeys,
}

impl Link {
    fn url(self) -> String {
        match self {
            Link::Repository => REPOSITORY.to_string(),
            Link::Releases => format!("{REPOSITORY}/releases"),
            Link::ReportProblem => format!("{REPOSITORY}/issues/new"),
            Link::Security => format!("{REPOSITORY}/blob/main/SECURITY.md"),
            Link::License => format!("{REPOSITORY}#license"),
            Link::OpenrouterKeys => "https://openrouter.ai/keys".to_string(),
        }
    }
}

#[tauri::command]
pub fn open_link(link: Link) -> CommandResult<()> {
    tauri_plugin_opener::open_url(link.url(), None::<&str>)
        .map_err(|e| CommandError::Other(e.to_string()))
}

/// Menu item ids handled in [`on_menu_event`].
const ABOUT: &str = "about";
const SETTINGS: &str = "settings";
const REPORT: &str = "report-problem";
const WEBSITE: &str = "website";

/// The macOS menu bar: the standard menus, with Quill's About window,
/// Settings (⌘,) and Help links.
#[cfg(target_os = "macos")]
pub fn menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem as P, Submenu};
    let sep = || P::separator(app);
    let app_menu = Submenu::with_items(
        app,
        "Quill",
        true,
        &[
            &MenuItem::with_id(app, ABOUT, "About Quill", true, None::<&str>)?,
            &sep()?,
            &MenuItem::with_id(app, SETTINGS, "Settings…", true, Some("CmdOrCtrl+,"))?,
            &sep()?,
            &P::services(app, None)?,
            &sep()?,
            &P::hide(app, None)?,
            &P::hide_others(app, None)?,
            &P::show_all(app, None)?,
            &sep()?,
            &P::quit(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &P::undo(app, None)?,
            &P::redo(app, None)?,
            &sep()?,
            &P::cut(app, None)?,
            &P::copy(app, None)?,
            &P::paste(app, None)?,
            &P::select_all(app, None)?,
        ],
    )?;
    let view = Submenu::with_items(app, "View", true, &[&P::fullscreen(app, None)?])?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &P::minimize(app, None)?,
            &P::maximize(app, None)?,
            &sep()?,
            &P::close_window(app, None)?,
        ],
    )?;
    let help = Submenu::with_items(
        app,
        "Help",
        true,
        &[
            &MenuItem::with_id(app, REPORT, "Report a Problem…", true, None::<&str>)?,
            &MenuItem::with_id(app, WEBSITE, "Quill on GitHub", true, None::<&str>)?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &edit, &view, &window, &help])
}

/// Handles the app menu's own items.
pub fn on_menu_event(app: &AppHandle, id: &str) {
    match id {
        ABOUT => show(app),
        SETTINGS => {
            crate::tray::show_window(app);
            let _ = app.emit(crate::NAVIGATE_EVENT, "settings");
        }
        REPORT => {
            let _ = open_link(Link::ReportProblem);
        }
        WEBSITE => {
            let _ = open_link(Link::Repository);
        }
        _ => {}
    }
}
