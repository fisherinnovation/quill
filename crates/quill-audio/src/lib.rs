//! Audio capture for Quill: microphone and system audio, resampling to
//! 16 kHz mono, voice activity detection and WAV writing.
//!
//! - [`capture`]: record the microphone and system audio to WAV files.
//! - [`check`]: a short test recording that confirms both sources are heard.
//! - [`live`]: microphone capture into memory, for dictation.
//! - [`cue`]: short start/stop sounds.
//! - [`load_for_stt`]: read a WAV, downmix to mono and resample to the 16 kHz
//!   that Whisper expects.
//!
//! Audio callbacks must never block or allocate heavily; they hand samples to
//! a worker thread through a lock-free ring buffer.

pub mod aec;
pub mod capture;
pub mod check;
pub mod cue;
pub mod live;
mod resample;
mod wav;

pub use resample::{resample_mono, to_mono};
pub use wav::{AudioBuffer, read_wav};

use std::path::Path;

/// Sample rate Whisper (and most speech models) expect.
pub const STT_SAMPLE_RATE: u32 = 16_000;

/// Errors from reading or converting audio.
//
// `thiserror` writes the `Display` and `std::error::Error` impls for us: the
// `#[error("...")]` string is the message, and `#[from]` adds a `From` impl so
// `?` converts a `hound::Error` into `AudioError::Wav` automatically.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("could not read WAV file")]
    Wav(#[from] hound::Error),
    #[error("unsupported WAV format: {0}")]
    UnsupportedFormat(String),
    #[error("resampling failed: {0}")]
    Resample(String),
    #[error("no default {0} device")]
    NoDevice(&'static str),
    #[error("{context}: {source}")]
    Device {
        context: &'static str,
        source: cpal::Error,
    },
    #[error("capture failed: {0}")]
    Capture(String),
    #[error("WAV writer thread panicked")]
    WriterPanicked,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl AudioError {
    fn device(context: &'static str, source: cpal::Error) -> Self {
        Self::Device { context, source }
    }
}

/// Loads a WAV file as 16 kHz mono `f32` samples in `-1.0..=1.0`, ready to
/// hand to a speech-to-text engine.
pub fn load_for_stt(path: &Path) -> Result<Vec<f32>, AudioError> {
    let audio = read_wav(path)?;
    let mono = to_mono(&audio.samples, audio.channels);
    resample_mono(&mono, audio.sample_rate, STT_SAMPLE_RATE)
}
