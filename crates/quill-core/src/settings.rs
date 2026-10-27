//! User settings, stored as `settings.toml` in the data directory.
//!
//! Every field has a default, and unknown or missing fields are tolerated,
//! so older and newer versions of the app can share one file.

use std::path::{Path, PathBuf};

use quill_audio::cue::Sounds;
use quill_stt::GpuChoice;
use serde::{Deserialize, Serialize};

/// Summary model: accurate, cheap, fast, with a 1M-token context.
pub const DEFAULT_SUMMARY_MODEL: &str = "anthropic/claude-haiku-5.5";

/// Dictation polish model: the fastest consistent one in the comparison in
/// eval/dictation/README.md (median 0.66 s).
pub const DEFAULT_POLISH_MODEL: &str = "google/gemini-3.5-flash-lite";

/// The localhost port the app serves AI assistants on, unless the user
/// picks another (`Settings::assistant_port`).
pub const DEFAULT_ASSISTANT_PORT: u16 = 47321;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Speech model name (see `quill-cli models list`): a Whisper size or
    /// a Parakeet model. Meetings use it, and dictation unless
    /// `dictation.stt_model` says otherwise.
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
    /// Tell the people on the call apart by voice ("Speaker 1", "Speaker
    /// 2"…) instead of labelling them all "Others".
    pub identify_speakers: bool,
    /// Where meeting audio lives; `None` = `<data dir>/recordings`. Change it
    /// with `Library::move_recordings` so existing meetings move too.
    pub recordings_dir: Option<PathBuf>,
    /// The app's first-run screen has been completed.
    pub onboarded: bool,
    /// Look for a new version of Quill on GitHub once a day. Updates are
    /// only installed when the user asks.
    pub check_for_updates: bool,
    /// Light or dark, or whatever the OS uses.
    pub theme: Theme,
    /// When a call app starts using the microphone, offer to record it
    /// (`quill_core::calls`); when it stops, offer to stop.
    pub offer_to_record_calls: bool,
    /// While recording, keep live notes (`quill_core::live_meeting`) in a
    /// panel: the transcript so far goes to the summary model every minute.
    pub live_notes: bool,
    /// A system notification when a meeting's notes are ready (or couldn't
    /// be written), if Quill isn't in front.
    pub notify_notes_ready: bool,
    /// Remember the voices of people the user names on calls, and name them
    /// automatically in later meetings (`quill_core::voices`).
    pub recognize_voices: bool,
    /// Search by meaning as well as by words (`quill_core::semantic` and
    /// `quill_core::search`), once the search model is downloaded.
    pub semantic_search: bool,
    /// Let AI assistants such as Claude read meetings, notes and action
    /// items (`quill_core::assistants`): the app serves them on
    /// `http://127.0.0.1:<assistant_port>/mcp` while it's on, and
    /// `quill-cli mcp` over stdio. Off unless the user turns it on; checked
    /// on every request, so turning it off takes effect at once.
    pub assistant_access: bool,
    /// The localhost port the app serves assistants on.
    pub assistant_port: u16,
    // Tables must come after plain values in TOML, so keep these last.
    /// Which graphics card transcribes, where there's a choice (Windows).
    pub gpu: GpuChoice,
    pub dictation: DictationSettings,
}

/// The app's colours.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    /// Follow the OS, including when it switches between light and dark.
    #[default]
    System,
    Light,
    Dark,
}

/// `DictationSettings::hotkey` value meaning the fn (🌐) key on its own
/// (macOS only).
pub const FN_HOTKEY: &str = "Fn";

/// Global-hotkey dictation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DictationSettings {
    pub enabled: bool,
    /// Global shortcut, e.g. "Alt+Space", or [`FN_HOTKEY`] for the fn key on
    /// macOS. Hold it to talk, or tap it to start and tap again to stop.
    pub hotkey: String,
    /// Spoken language: a code like "en", "auto" to detect it each time
    /// (adds ~0.5 s), or `None` for the system language.
    pub language: Option<String>,
    /// Speech model for dictation, when it isn't the meetings' one
    /// (`Settings::stt_model`): e.g. a Parakeet model for dictation and
    /// Whisper for meetings. `None` uses the meetings' model.
    pub stt_model: Option<String>,
    /// Clean up the text with an LLM before typing it.
    pub polish: bool,
    /// OpenRouter model id or `ollama:<model>` for the polish step.
    pub polish_model: String,
    /// What the user tells the clean-up model about themselves: role,
    /// names they use, spelling preferences. Only used when polishing.
    pub about: String,
    /// Names and terms to spell exactly (fed to Whisper and the polish step).
    pub vocabulary: Vec<String>,
    /// Applied to the text before it's typed, whole words, ignoring case.
    pub replacements: Vec<Replacement>,
    /// Keep a history of what was dictated (in the local library).
    pub keep_history: bool,
    /// Keep each dictation's audio and typed text, to measure accuracy
    /// (`quill_core::samples`). Off unless the user turns it on.
    pub save_samples: bool,
    /// Show what's being said while dictating (`quill_core::live`).
    pub live_captions: bool,
    /// The sounds dictation plays to start, stop and cancel.
    pub sound: Sounds,
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
            stt_model: None,
            polish: true,
            polish_model: DEFAULT_POLISH_MODEL.into(),
            about: String::new(),
            vocabulary: Vec::new(),
            replacements: Vec::new(),
            keep_history: true,
            save_samples: false,
            live_captions: true,
            sound: Sounds::Classic,
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
            identify_speakers: true,
            recordings_dir: None,
            onboarded: false,
            check_for_updates: true,
            theme: Theme::System,
            offer_to_record_calls: true,
            live_notes: true,
            notify_notes_ready: true,
            recognize_voices: true,
            semantic_search: true,
            assistant_access: false,
            assistant_port: DEFAULT_ASSISTANT_PORT,
            gpu: GpuChoice::Automatic,
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
    /// The speech model dictation uses.
    pub fn dictation_model(&self) -> &str {
        self.dictation
            .stt_model
            .as_deref()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or(&self.stt_model)
    }

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
            theme: Theme::Dark,
            gpu: GpuChoice::Named("NVIDIA GeForce RTX 4070".into()),
            dictation: DictationSettings {
                stt_model: Some("parakeet-v2".into()),
                sound: Sounds::Marimba,
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

    #[test]
    fn dictation_uses_the_meetings_model_unless_told_otherwise() {
        let mut s = Settings::default();
        assert_eq!(s.dictation_model(), "turbo");
        s.dictation.stt_model = Some("parakeet-v2".into());
        assert_eq!(s.dictation_model(), "parakeet-v2");
        assert_eq!(s.stt_model, "turbo");
        s.dictation.stt_model = Some(String::new());
        assert_eq!(s.dictation_model(), "turbo");
    }
}
