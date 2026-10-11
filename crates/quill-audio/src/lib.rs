//! Audio capture for Quill: microphone and system audio, resampling to
//! 16 kHz mono, voice activity detection and WAV writing.
//!
//! Implemented in M2. Audio callbacks must never block or allocate heavily;
//! they hand samples to a worker thread through a ring buffer or channel.
