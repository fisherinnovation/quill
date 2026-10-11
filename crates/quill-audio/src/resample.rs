//! Downmixing and sample-rate conversion.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

use crate::AudioError;

/// Averages interleaved multi-channel samples into a single mono channel.
pub fn to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    let channels = usize::from(channels.max(1));
    if channels == 1 {
        return samples.to_vec();
    }
    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Resamples a whole mono clip from `from_hz` to `to_hz`.
///
/// Uses rubato's FFT resampler, which band-limits the signal first, so
/// downsampling (e.g. 48 kHz → 16 kHz) doesn't fold high frequencies back
/// into the speech band as aliasing noise.
pub fn resample_mono(samples: &[f32], from_hz: u32, to_hz: u32) -> Result<Vec<f32>, AudioError> {
    if from_hz == to_hz || samples.is_empty() {
        return Ok(samples.to_vec());
    }

    // Chunk size only affects internal processing granularity; `process_all`
    // feeds the whole clip through chunk by chunk.
    const CHUNK_FRAMES: usize = 1024;
    let mut resampler = Fft::<f32>::new(
        from_hz as usize,
        to_hz as usize,
        CHUNK_FRAMES,
        1,
        FixedSync::Input,
    )
    .map_err(|e| AudioError::Resample(e.to_string()))?;

    // rubato reads through an "adapter" so it can accept many buffer layouts;
    // a mono slice is trivially interleaved with one channel.
    let input = InterleavedSlice::new(samples, 1, samples.len())
        .map_err(|e| AudioError::Resample(e.to_string()))?;
    let output = resampler
        .process_all(&input, samples.len(), None)
        .map_err(|e| AudioError::Resample(e.to_string()))?;
    Ok(output.take_data())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    #[test]
    fn to_mono_averages_channels() {
        let stereo = [1.0, 0.0, 0.5, 0.5, -1.0, 1.0];
        assert_eq!(to_mono(&stereo, 2), vec![0.5, 0.5, 0.0]);
    }

    #[test]
    fn same_rate_is_a_copy() {
        let samples = [0.1, 0.2, 0.3];
        assert_eq!(resample_mono(&samples, 16_000, 16_000).unwrap(), samples);
    }

    /// Counts sign changes; a pure tone of `f` Hz has `2 * f` per second.
    fn zero_crossings(samples: &[f32]) -> usize {
        samples
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count()
    }

    #[test]
    fn downsampling_keeps_length_and_pitch() {
        let (from, to, secs, tone_hz) = (48_000, 16_000, 2.0, 440.0);
        let input: Vec<f32> = (0..(from as f32 * secs) as usize)
            .map(|i| (TAU * tone_hz * i as f32 / from as f32).sin() * 0.5)
            .collect();

        let output = resample_mono(&input, from, to).unwrap();

        // One third as many samples (within a few frames of edge effects).
        let expected = (to as f32 * secs) as usize;
        assert!(output.len().abs_diff(expected) < 16, "len {}", output.len());
        // Still a 440 Hz tone: ~880 zero crossings per second.
        let crossings = zero_crossings(&output) as f32 / secs;
        assert!((crossings - 2.0 * tone_hz).abs() < 5.0, "{crossings}");
    }
}
