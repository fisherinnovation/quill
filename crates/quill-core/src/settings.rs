//! User settings, stored as `settings.toml` in the data directory.
//!
//! Every field has a default, and unknown or missing fields are tolerated,
//! so older and newer versions of the app can share one file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Summary model until the M4 eval picks one: cheap, fast, 1M-token context.
pub const DEFAULT_SUMMARY_MODEL: &str = "anthropic/claude-haiku-5.5";

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
