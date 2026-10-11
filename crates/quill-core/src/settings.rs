//! User settings, stored as `settings.toml` in the data directory.
//!
//! Every field has a default, and unknown or missing fields are tolerated,
//! so older and newer versions of the app can share one file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Summary model until the M4 eval picks one: cheap, fast, 1M-token context.
pub const DEFAULT_SUMMARY_MODEL: &str = "anthropic/claude-haiku-5.5";

/// Dictation polish model: the fastest consistent one in the M6 comparison
/// (median 0.66 s; see eval/dictation/README.md).
pub const DEFAULT_POLISH_MODEL: &str = "google/gemini-3.5-flash-lite";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Whisper model name (see `quill-cli models list`).
    pub stt_model: String,
    /// Spoken language, e.g. "en"; `None` auto-detects.
    pub language: Option<String>,
    /// Skip silence with Silero VAD before transcribing.
    pub vad: bool,
    /// OpenRouter model id or `ollama:<model>`.
    pub summary_model: String,
    /// OpenRouter: zero-data-retention endpoints only.
    pub zdr: bool,
    /// Summarize automatically after transcribing.
    pub auto_summarize: bool,
    /// Where meeting audio lives; `None` = `<data dir>/recordings`. Change it
    /// with `Library::move_recordings` so existing meetings move too.
    pub recordings_dir: Option<PathBuf>,
    /// The app's first-run screen has been completed.
    pub onboarded: bool,
    // Tables must come after plain values in TOML, so keep this last.
    pub dictation: DictationSettings,
}

/// Global-hotkey dictation (M6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DictationSettings {
    pub enabled: bool,
    /// Global shortcut, e.g. "Alt+Space". Hold it to talk, or tap it to
    /// start and tap again to stop.
    pub hotkey: String,
    /// Spoken language: a code like "en", "auto" to detect it each time
    /// (adds ~0.5 s), or `None` for the system language.
    pub language: Option<String>,
    /// Clean up the text with an LLM before typing it.
    pub polish: bool,
    /// OpenRouter model id or `ollama:<model>` for the polish step.
    pub polish_model: String,
    /// Names and terms to spell exactly (fed to Whisper and the polish step).
    pub vocabulary: Vec<String>,
    /// Applied to the text before it's typed, whole words, ignoring case.
    pub replacements: Vec<Replacement>,
    /// Keep a history of what was dictated (in the local library).
    pub keep_history: bool,
    /// Styles chosen for particular apps, overriding the built-in ones
    /// (`dictation::style_for`).
    pub app_styles: Vec<AppStyle>,
}

/// How dictated text should read in a particular app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    /// As spoken, cleaned up.
    Default,
    /// Chat: relaxed, no trailing period on one-line messages.
    Casual,
    /// Email and documents: complete, properly punctuated sentences.
    Formal,
    /// Code editors and terminals: identifiers and commands kept exact.
    Technical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppStyle {
    /// Bundle id (macOS) or executable name (Windows).
    pub app_id: String,
    /// Display name, for the settings list.
    pub name: String,
    pub style: Style,
}

impl Default for DictationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            hotkey: "Alt+Space".into(),
            language: None,
            polish: true,
            polish_model: DEFAULT_POLISH_MODEL.into(),
            vocabulary: Vec::new(),
            replacements: Vec::new(),
            keep_history: true,
            app_styles: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replacement {
    pub from: String,
    pub to: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            stt_model: "turbo".into(),
            language: None,
            vad: true,
            summary_model: DEFAULT_SUMMARY_MODEL.into(),
            zdr: false,
            auto_summarize: true,
            recordings_dir: None,
            onboarded: false,
            dictation: DictationSettings::default(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("settings.toml is invalid: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("could not write settings: {0}")]
    Write(#[from] toml::ser::Error),
}

impl Settings {
    /// The recordings folder in effect.
    pub fn recordings_dir(&self, data_dir: &Path) -> PathBuf {
        self.recordings_dir
            .clone()
            .unwrap_or_else(|| data_dir.join("recordings"))
    }

    /// Reads `settings.toml` from `data_dir`, or defaults if it doesn't exist.
    pub fn load(data_dir: &Path) -> Result<Self, SettingsError> {
        match std::fs::read_to_string(data_dir.join("settings.toml")) {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, data_dir: &Path) -> Result<(), SettingsError> {
        std::fs::create_dir_all(data_dir)?;
        // Write then rename, so a crash mid-write can't leave half a file.
        let tmp = data_dir.join("settings.toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(tmp, data_dir.join("settings.toml"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()).unwrap(), Settings::default());

        let custom = Settings {
            language: Some("en".into()),
            summary_model: "ollama:qwen3:8b".into(),
            recordings_dir: Some("/Volumes/Archive/Quill".into()),
            dictation: DictationSettings {
                vocabulary: vec!["Quill".into(), "OpenRouter".into()],
                replacements: vec![Replacement {
                    from: "open router".into(),
                    to: "OpenRouter".into(),
                }],
                app_styles: vec![AppStyle {
                    app_id: "com.apple.mail".into(),
                    name: "Mail".into(),
                    style: Style::Casual,
                }],
                ..DictationSettings::default()
            },
            ..Settings::default()
        };
        custom.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()).unwrap(), custom);
    }

    #[test]
    fn partial_and_unknown_fields_are_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.toml"),
            "stt_model = \"small\"\nfuture_option = 3\n",
        )
        .unwrap();
        let s = Settings::load(dir.path()).unwrap();
        assert_eq!(s.stt_model, "small");
        assert!(s.vad);
    }
}
