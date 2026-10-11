//! Reading WAV files into normalized `f32` samples.

use std::path::Path;

use hound::{SampleFormat, WavReader};

use crate::AudioError;

/// Decoded audio: interleaved `f32` samples in `-1.0..=1.0`.
///
/// "Interleaved" means one sample per channel, frame by frame: for stereo the
/// layout is `[L0, R0, L1, R1, ...]`.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioBuffer {
    /// Length of the audio in seconds.
    pub fn duration_secs(&self) -> f64 {
        let frames = self.samples.len() / usize::from(self.channels.max(1));
        frames as f64 / f64::from(self.sample_rate)
    }
}

/// Reads a PCM (8/16/24/32-bit integer) or 32-bit float WAV file.
pub fn read_wav(path: &Path) -> Result<AudioBuffer, AudioError> {
    let reader = WavReader::open(path)?;
    let spec = reader.spec();
    if spec.channels == 0 {
        return Err(AudioError::UnsupportedFormat("zero channels".into()));
    }

    let samples = match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Float, 32) => reader.into_samples::<f32>().collect::<Result<_, _>>()?,
        (SampleFormat::Int, bits @ 1..=32) => {
            // Integer samples range over ±2^(bits-1); dividing by that maps
            // them onto -1.0..=1.0. hound sign-extends every width into i32.
            let scale = 1.0 / (1_i64 << (bits - 1)) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|s| s as f32 * scale))
                .collect::<Result<_, _>>()?
        }
        (format, bits) => {
            return Err(AudioError::UnsupportedFormat(format!(
                "{bits}-bit {format:?} samples"
            )));
        }
    };

    Ok(AudioBuffer {
        samples,
        sample_rate: spec.sample_rate,
        channels: spec.channels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{WavSpec, WavWriter};

    fn write_wav<S: hound::Sample + Copy>(path: &Path, spec: WavSpec, samples: &[S]) {
        let mut writer = WavWriter::create(path, spec).unwrap();
        for &s in samples {
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn reads_16_bit_stereo_and_normalizes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stereo.wav");
        let spec = WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        write_wav(&path, spec, &[i16::MIN, 0, 16_384, i16::MAX]);

        let audio = read_wav(&path).unwrap();
        assert_eq!(audio.channels, 2);
        assert_eq!(audio.sample_rate, 44_100);
        assert_eq!(audio.samples[0], -1.0);
        assert_eq!(audio.samples[1], 0.0);
        assert_eq!(audio.samples[2], 0.5);
        assert!((audio.samples[3] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn reads_32_bit_float() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("float.wav");
        let spec = WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        write_wav(&path, spec, &[0.25_f32, -0.75]);

        let audio = read_wav(&path).unwrap();
        assert_eq!(audio.samples, vec![0.25, -0.75]);
        assert!((audio.duration_secs() - 2.0 / 48_000.0).abs() < 1e-12);
    }
}
