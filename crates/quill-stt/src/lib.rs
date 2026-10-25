//! Speech-to-text for Quill: the `Transcriber` trait and its two engines,
//! whisper.cpp (`whisper-rs`) and NVIDIA Parakeet (through sherpa-onnx).
//!
//! Input is always 16 kHz mono `f32` PCM; `quill-audio` handles getting audio
//! into that shape.

pub mod gpu;
pub mod models;
mod parakeet;
mod vad;
mod whisper;

pub use gpu::{Gpu, GpuChoice};
pub use parakeet::{ParakeetConfig, ParakeetTranscriber};
pub use whisper::{WhisperConfig, WhisperTranscriber, gpu_backend};

use serde::{Deserialize, Serialize};

/// One timestamped stretch of transcribed speech.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// Filled in later by the meeting pipeline ("Me" / "Others") or by
    /// diarization; the transcriber itself leaves it `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
}

/// Whether Whisper knows this ISO 639-1 language code (e.g. `"en"`).
pub fn language_supported(code: &str) -> bool {
    whisper_rs::get_lang_id(code).is_some()
}

/// Per-call knobs for a transcription.
#[derive(Debug, Clone, Default)]
pub struct TranscribeOptions {
    /// ISO 639-1 code such as `"en"`. `None` lets the model detect it.
    pub language: Option<String>,
    /// Text that primes the decoder: custom vocabulary, names, spelling
    /// style. Whisper only.
    pub initial_prompt: Option<String>,
    /// Names and terms to spell exactly. Parakeet favours them while
    /// decoding (sherpa-onnx hotwords); Whisper gets them through
    /// `initial_prompt` instead, so it ignores this.
    pub vocabulary: Vec<String>,
    /// Tune for a clip of a few seconds (dictation) rather than a recording:
    /// greedy decoding, VAD trimming only the outer silence, and on Metal an
    /// encoder sized to the clip, with repeated segments dropped. Leave off
    /// for meetings.
    pub short_clip: bool,
    /// Don't retry at higher temperatures when the decode looks unsure.
    /// For live dictation's passes over audio that ends mid-word, where
    /// retries cost seconds and the next pass redoes it anyway.
    pub no_fallback: bool,
    /// Told how the transcription is going, for showing progress. Doesn't
    /// change the result.
    pub progress: Option<Progress>,
}

/// What a [`Progress`] hook hears while a transcription runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgressEvent {
    /// How far through the audio Whisper is, 0–100.
    Percent(u8),
    /// A line as soon as it's recognised (before speech is mapped back to
    /// the recording's timeline).
    Line(String),
}

/// A hook for [`ProgressEvent`]s, shareable with Whisper's callbacks.
#[derive(Clone)]
pub struct Progress(pub std::sync::Arc<dyn Fn(ProgressEvent) + Send + Sync>);

impl Progress {
    pub fn new(f: impl Fn(ProgressEvent) + Send + Sync + 'static) -> Self {
        Self(std::sync::Arc::new(f))
    }
}

impl std::fmt::Debug for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Progress(..)")
    }
}

/// Anything that turns 16 kHz mono audio into timestamped text.
///
/// `Send + Sync` lets one loaded model be shared across threads (e.g. behind
/// an `Arc`), which the app needs so the dictation hotkey and the meeting
/// pipeline can use the same resident model.
pub trait Transcriber: Send + Sync {
    fn transcribe(
        &self,
        pcm_16k_mono: &[f32],
        opts: &TranscribeOptions,
    ) -> Result<Vec<Segment>, SttError>;

    /// Where speech ends in 16 kHz mono `pcm`: the end of the last stretch
    /// Silero VAD hears as speech. For measuring how long a dictation took
    /// to arrive after the speaker stopped talking. `None` without a VAD
    /// model, or with no speech.
    fn speech_end(&self, _pcm: &[f32]) -> Option<std::time::Duration> {
        None
    }
}

/// Loads speech model `name` from `models_dir`, whichever engine it
/// belongs to: a Whisper size (`"turbo"`) or a Parakeet model
/// (`"parakeet-v2"`). Parakeet runs on the CPU, so it uses only `config`'s
/// VAD model and threads (at most 4: more were slower).
pub fn load(
    name: &str,
    models_dir: &std::path::Path,
    config: &WhisperConfig,
) -> Result<Box<dyn Transcriber>, SttError> {
    if let Some(parakeet) = models::find_parakeet(name) {
        let config = ParakeetConfig {
            threads: config.threads.min(parakeet::MAX_THREADS),
            vad_model: config.vad_model.clone(),
        };
        return Ok(Box::new(ParakeetTranscriber::load(
            parakeet, models_dir, &config,
        )?));
    }
    let model = models::MODELS
        .iter()
        .find(|m| m.name == name)
        .ok_or_else(|| SttError::UnknownModel(name.to_string()))?;
    Ok(Box::new(WhisperTranscriber::load(
        &model.path_in(models_dir),
        config,
    )?))
}

/// Errors from loading models or transcribing.
#[derive(Debug, thiserror::Error)]
pub enum SttError {
    #[error("model file not found: {0}")]
    ModelNotFound(std::path::PathBuf),
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(std::path::PathBuf),
    #[error("whisper.cpp error: {0}")]
    Whisper(#[from] whisper_rs::WhisperError),
    #[error("sherpa-onnx could not load the Parakeet model (see the log for details)")]
    ParakeetLoad,
    #[error("unknown speech model `{0}`")]
    UnknownModel(String),
    #[error("model download failed: {0}")]
    Download(String),
    #[error("checksum mismatch for {file}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        file: String,
        expected: String,
        actual: String,
    },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
