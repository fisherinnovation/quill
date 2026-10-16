//! Dictation samples: the audio of each dictation and what was typed, kept
//! (only when the user turns it on) to measure accuracy on real speech with
//! `quill-cli dictate`. See `eval/dictation/README.md`.
//!
//! Each sample is three files in [`dir`], named by the local time:
//!
//! - `<name>.flac`: what Whisper heard, 16 kHz mono.
//! - `<name>.typed.txt`: what was typed. Copy it to `<name>.txt` and fix it
//!   to make the reference ([`reference_path`]).
//! - `<name>.json`: [`Meta`], so a rerun can use the same app style.

use std::path::{Path, PathBuf};

use quill_audio::STT_SAMPLE_RATE;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum SampleError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Audio(#[from] quill_audio::AudioError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// How a sample was dictated.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Meta {
    /// The app it was typed into: bundle id (macOS) or executable (Windows).
    pub app_id: Option<String>,
    pub app: Option<String>,
    pub stt_model: String,
    /// The clean-up model, if the typed text came from one.
    pub polish_model: Option<String>,
    /// Whisper's text, before clean-up.
    pub whisper: String,
}

/// Where samples are kept.
pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("dictation-samples")
}

/// Saves one dictation, returning the audio's path.
pub fn save(dir: &Path, pcm: &[f32], typed: &str, meta: &Meta) -> Result<PathBuf, SampleError> {
    std::fs::create_dir_all(dir)?;
    let stamp = jiff::Zoned::now().strftime("%Y-%m-%d_%H%M%S").to_string();
    // Two dictations can finish within a second.
    let mut name = stamp.clone();
    let mut n = 1;
    while dir.join(format!("{name}.flac")).exists() {
        n += 1;
        name = format!("{stamp}-{n}");
    }
    let audio = dir.join(format!("{name}.flac"));
    std::fs::write(dir.join(format!("{name}.typed.txt")), format!("{typed}\n"))?;
    std::fs::write(
        dir.join(format!("{name}.json")),
        serde_json::to_string_pretty(meta)?,
    )?;
    // Last, so a sample whose audio exists is complete.
    quill_audio::write_flac(&audio, pcm, STT_SAMPLE_RATE)?;
    Ok(audio)
}

/// How many samples there are and how much space they take.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub count: usize,
    pub bytes: u64,
}

pub fn usage(dir: &Path) -> std::io::Result<Usage> {
    let mut usage = Usage::default();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(usage),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        usage.bytes += meta.len();
        if entry.path().extension().is_some_and(|e| e == "flac") {
            usage.count += 1;
        }
    }
    Ok(usage)
}

/// Deletes every sample, references included.
pub fn clear(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// The reference for an audio file: `clip.txt` beside `clip.wav` or
/// `clip.flac`, holding what should have been typed.
pub fn reference_path(audio: &Path) -> PathBuf {
    audio.with_extension("txt")
}

/// The reference text for an audio file, if there is one.
pub fn reference(audio: &Path) -> std::io::Result<Option<String>> {
    read_optional(&reference_path(audio)).map(|t| t.map(|t| t.trim().to_string()))
}

/// The [`Meta`] saved with an audio file, if there is one.
pub fn meta(audio: &Path) -> Result<Option<Meta>, SampleError> {
    match read_optional(&audio.with_extension("json"))? {
        Some(json) => Ok(Some(serde_json::from_str(&json)?)),
        None => Ok(None),
    }
}

fn read_optional(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_counts_reads_back_and_clears() {
        let root = tempfile::tempdir().unwrap();
        let dir = dir(root.path());
        assert_eq!(usage(&dir).unwrap(), Usage::default());

        let meta = Meta {
            app_id: Some("com.tinyspeck.slackmacgap".into()),
            app: Some("Slack".into()),
            stt_model: "turbo".into(),
            polish_model: None,
            whisper: "Sounds good.".into(),
        };
        let pcm = vec![0.1_f32; 16_000];
        let first = save(&dir, &pcm, "Sounds good", &meta).unwrap();
        let second = save(&dir, &pcm, "Sounds good", &meta).unwrap();
        assert_ne!(first, second);
        assert_eq!(
            std::fs::read_to_string(first.with_extension("typed.txt")).unwrap(),
            "Sounds good\n"
        );
        assert_eq!(super::meta(&first).unwrap(), Some(meta));
        assert_eq!(quill_audio::load_for_stt(&first).unwrap().len(), 16_000);

        // No reference until the user writes one.
        assert_eq!(reference(&first).unwrap(), None);
        std::fs::write(reference_path(&first), "Sounds good.\n").unwrap();
        assert_eq!(reference(&first).unwrap().as_deref(), Some("Sounds good."));

        let used = usage(&dir).unwrap();
        assert_eq!(used.count, 2);
        assert!(used.bytes > 0);
        clear(&dir).unwrap();
        assert_eq!(usage(&dir).unwrap(), Usage::default());
        clear(&dir).unwrap();
    }
}
