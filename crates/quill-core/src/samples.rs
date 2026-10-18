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
//!
//! The app's review page ([`list`], [`set_reference`]) writes the
//! references, and scores what was typed against them ([`accuracy`]).

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

/// One sample, for review.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sample {
    /// The files' shared name, e.g. `2026-10-16_143012`.
    pub name: String,
    pub typed: String,
    /// Whisper's text, before clean-up.
    pub whisper: String,
    pub app: Option<String>,
    /// The clean-up model, if it changed the text.
    pub polish_model: Option<String>,
    pub duration_ms: u64,
    /// What should have been typed, once reviewed.
    pub reference: Option<String>,
    /// What was typed wrong, against the reference: `quil→quill`, `-the`.
    pub errors: Vec<String>,
}

/// A sample's name is a file stem Quill made; anything else (a path) is
/// refused.
fn valid(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn invalid(name: &str) -> SampleError {
    SampleError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        format!("not a sample: {name}"),
    ))
}

/// Every sample in `dir`, oldest first.
pub fn list(dir: &Path) -> Result<Vec<Sample>, SampleError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut audio: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "flac"))
        .collect();
    // By name, so `x` comes before `x-2` (the second within that second).
    audio.sort_by(|a, b| a.file_stem().cmp(&b.file_stem()));
    let mut out = Vec::new();
    for path in audio {
        let Some(name) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let typed = read_optional(&path.with_extension("typed.txt"))?
            .unwrap_or_default()
            .trim()
            .to_string();
        let meta = meta(&path)?.unwrap_or_default();
        let reference = reference(&path)?;
        let duration_ms = flac_duration_ms(&path).unwrap_or(0);
        let errors = reference
            .as_deref()
            .map(|r| crate::wer::align(r, &typed).errors())
            .unwrap_or_default();
        out.push(Sample {
            name,
            typed,
            whisper: meta.whisper,
            app: meta.app,
            polish_model: meta.polish_model,
            duration_ms,
            reference,
            errors,
        });
    }
    Ok(out)
}

/// A FLAC file's length, from its header (STREAMINFO: the sample rate and
/// total samples), without decoding it: the list is read after every save.
fn flac_duration_ms(path: &Path) -> Option<u64> {
    use std::io::Read;
    let mut head = [0_u8; 26];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    if &head[..4] != b"fLaC" || head[4] & 0x7f != 0 {
        return None;
    }
    // After the block header (4) and the block and frame sizes (10): the
    // rate (20 bits), channels and bits (8), and total samples (36).
    let packed = u64::from_be_bytes(head[18..26].try_into().ok()?);
    let rate = packed >> 44;
    let total = packed & ((1 << 36) - 1);
    (rate > 0).then(|| total * 1000 / rate)
}

/// The audio file of sample `name`.
pub fn audio_path(dir: &Path, name: &str) -> Result<PathBuf, SampleError> {
    if !valid(name) {
        return Err(invalid(name));
    }
    Ok(dir.join(format!("{name}.flac")))
}

/// Saves what should have been typed for sample `name`.
pub fn set_reference(dir: &Path, name: &str, text: &str) -> Result<(), SampleError> {
    let audio = audio_path(dir, name)?;
    if !audio.is_file() {
        return Err(invalid(name));
    }
    std::fs::write(reference_path(&audio), format!("{}\n", text.trim()))?;
    Ok(())
}

/// Deletes sample `name` (a muddled or accidental recording).
pub fn remove(dir: &Path, name: &str) -> Result<(), SampleError> {
    let audio = audio_path(dir, name)?;
    for path in [
        audio.with_extension("txt"),
        audio.with_extension("typed.txt"),
        audio.with_extension("json"),
        audio,
    ] {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    Ok(())
}

/// How accurate the reviewed samples were: what was typed, and Whisper's
/// own text, against the references.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Accuracy {
    pub reviewed: usize,
    pub typed: crate::wer::Tally,
    pub whisper: crate::wer::Tally,
    /// Samples clean-up changed.
    pub cleaned: usize,
    /// The commonest mistakes in what was typed, with how often.
    pub common: Vec<(String, usize)>,
}

pub fn accuracy(samples: &[Sample]) -> Accuracy {
    let mut acc = Accuracy::default();
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for sample in samples {
        let Some(reference) = &sample.reference else {
            continue;
        };
        acc.reviewed += 1;
        acc.typed += crate::wer::align(reference, &sample.typed).tally();
        let whisper = if sample.whisper.is_empty() {
            &sample.typed
        } else {
            &sample.whisper
        };
        acc.whisper += crate::wer::align(reference, whisper).tally();
        if sample.polish_model.is_some() {
            acc.cleaned += 1;
        }
        for error in &sample.errors {
            *counts.entry(error.clone()).or_default() += 1;
        }
    }
    let mut common: Vec<_> = counts.into_iter().collect();
    common.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    common.truncate(8);
    acc.common = common;
    acc
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

        // Reviewing: listed oldest first, with what was typed wrong.
        let listed = list(&dir).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].reference.as_deref(), Some("Sounds good."));
        assert_eq!(listed[0].duration_ms, 1_000);
        set_reference(&dir, &listed[1].name, "Sounds great").unwrap();
        let listed = list(&dir).unwrap();
        assert_eq!(listed[1].errors, ["good→great"]);
        let acc = accuracy(&listed);
        assert_eq!(acc.reviewed, 2);
        assert_eq!(acc.typed.reference_words, 4);
        assert_eq!(acc.typed.errors(), 1);
        assert_eq!(acc.common, [("good→great".to_string(), 1)]);
        assert!(set_reference(&dir, "../escape", "x").is_err());
        assert!(set_reference(&dir, "missing", "x").is_err());

        let used = usage(&dir).unwrap();
        assert_eq!(used.count, 2);
        assert!(used.bytes > 0);
        remove(&dir, &listed[1].name).unwrap();
        assert_eq!(list(&dir).unwrap().len(), 1);
        assert_eq!(usage(&dir).unwrap().count, 1);
        clear(&dir).unwrap();
        assert_eq!(usage(&dir).unwrap(), Usage::default());
        clear(&dir).unwrap();
    }
}
