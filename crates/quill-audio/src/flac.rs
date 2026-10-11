//! Storing recordings compactly as FLAC (lossless compression).
//!
//! The capture writes 32-bit float WAV at the device rate (48 kHz, the
//! system track in stereo): about 2 GB per hour for both tracks. Everything
//! downstream (echo cancellation, Whisper) works on 16 kHz mono, so once a
//! recording is done we keep exactly that, as 16-bit FLAC: roughly 1/20 of
//! the size, and nothing later steps use is lost.

use std::path::Path;

use flacenc::component::BitRepr;
use flacenc::error::Verify;

use crate::{AudioBuffer, AudioError};

const BITS: usize = 16;

/// Writes mono `samples` (in -1.0..=1.0) as 16-bit FLAC. Writes to a
/// temporary file first, so `path` either doesn't exist or is complete.
pub fn write_flac(path: &Path, samples: &[f32], sample_rate: u32) -> Result<(), AudioError> {
    let scale = ((1_i32 << (BITS - 1)) - 1) as f32;
    let ints: Vec<i32> = samples
        .iter()
        .map(|s| (s.clamp(-1.0, 1.0) * scale).round() as i32)
        .collect();
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| AudioError::Flac(format!("{e:?}")))?;
    let source = flacenc::source::MemSource::from_samples(&ints, 1, BITS, sample_rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| AudioError::Flac(format!("{e:?}")))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| AudioError::Flac(format!("{e:?}")))?;

    let partial = path.with_extension("flac.part");
    std::fs::write(&partial, sink.as_slice())?;
    std::fs::rename(&partial, path)?;
    Ok(())
}

/// Reads a FLAC file into normalized `f32` samples.
pub fn read_flac(path: &Path) -> Result<AudioBuffer, AudioError> {
    let mut reader = claxon::FlacReader::open(path).map_err(|e| AudioError::Flac(e.to_string()))?;
    let info = reader.streaminfo();
    if info.channels == 0 || info.bits_per_sample == 0 || info.bits_per_sample > 32 {
        return Err(AudioError::UnsupportedFormat(format!(
            "{}-bit FLAC with {} channels",
            info.bits_per_sample, info.channels
        )));
    }
    let scale = 1.0 / (1_i64 << (info.bits_per_sample - 1)) as f32;
    let samples = reader
        .samples()
        .map(|s| s.map(|s| s as f32 * scale))
        .collect::<Result<Vec<f32>, _>>()
        .map_err(|e| AudioError::Flac(e.to_string()))?;
    Ok(AudioBuffer {
        samples,
        sample_rate: info.sample_rate,
        channels: u16::try_from(info.channels).unwrap_or(u16::MAX),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_within_16_bit_precision_and_compresses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("speech.flac");
        // Two seconds of a decaying tone with silence after it: shaped a
        // bit like speech, and compressible like it.
        let samples: Vec<f32> = (0..32_000)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                if t < 1.2 {
                    (std::f32::consts::TAU * 220.0 * t).sin() * 0.3 * (-t * 2.0).exp()
                } else {
                    0.0
                }
            })
            .collect();

        write_flac(&path, &samples, 16_000).unwrap();
        assert!(!path.with_extension("flac.part").exists());
        let back = read_flac(&path).unwrap();

        assert_eq!((back.sample_rate, back.channels), (16_000, 1));
        assert_eq!(back.samples.len(), samples.len());
        let worst = samples
            .iter()
            .zip(&back.samples)
            .fold(0.0_f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(worst <= 1.0 / 32_767.0, "max error {worst}");
        // 16-bit PCM would be 64 kB.
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size < 40_000, "{size} bytes");
    }
}
