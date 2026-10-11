//! Speech-to-text for Quill: the `Transcriber` trait and its whisper.cpp
//! (`whisper-rs`) implementation, with sherpa-onnx as a later alternative.
//!
//! Input is always 16 kHz mono `f32` PCM; `quill-audio` handles getting audio
//! into that shape.

pub mod models;
mod vad;
mod whisper;

pub use whisper::{WhisperConfig, WhisperTranscriber};

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

/// Per-call knobs for a transcription.
#[derive(Debug, Clone, Default)]
pub struct TranscribeOptions {
    /// ISO 639-1 code such as `"en"`. `None` lets the model detect it.
    pub language: Option<String>,
    /// Text that primes the decoder: custom vocabulary, names, spelling style.
    pub initial_prompt: Option<String>,
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
