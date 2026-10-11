//! Acoustic echo cancellation for recordings made without headphones.
//!
//! When a call plays through speakers, the mic hears the other people too.
//! We have exactly what the speakers played (the system-audio track, aligned
//! with the mic track), so WebRTC's echo canceller (AEC3, via `sonora`) can
//! subtract it from the mic. Video-call apps do this live; we do it after
//! the recording, which is simpler: no real-time deadlines and no clock
//! drift between capture callbacks.

use sonora::config::EchoCanceller;
use sonora::{AudioProcessing, Config, StreamConfig};

use crate::AudioError;

/// Removes the far end's echo from `mic`.
///
/// `far_end` is what the speakers played, sample-aligned with `mic` (as the
/// recorder writes them). Both mono at `sample_rate` (16 kHz is what we use;
/// AEC3 also supports 32 and 48 kHz). Returns the cleaned mic signal, the
/// same length as `mic`.
pub fn cancel_echo(mic: &[f32], far_end: &[f32], sample_rate: u32) -> Result<Vec<f32>, AudioError> {
    let config = Config {
        echo_canceller: Some(EchoCanceller::default()),
        ..Default::default()
    };
    let stream = StreamConfig::new(sample_rate, 1);
    let mut apm = AudioProcessing::builder()
        .config(config)
        .capture_config(stream)
        .render_config(stream)
        .build();

    // AEC3 works on 10 ms frames.
    let frame = sample_rate as usize / 100;
    let mut out = Vec::with_capacity(mic.len());
    let mut far_buf = vec![0.0_f32; frame];
    let mut mic_buf = vec![0.0_f32; frame];
    let mut far_out = vec![0.0_f32; frame];
    let mut mic_out = vec![0.0_f32; frame];

    for start in (0..mic.len()).step_by(frame) {
        // Copy one frame of each, zero-padding past the end of either track.
        fill(&mut mic_buf, mic, start);
        fill(&mut far_buf, far_end, start);

        // "Render" = what went to the speakers, "capture" = what the mic
        // heard. Render first, so the canceller knows what to look for.
        apm.process_render_f32(&[&far_buf], &mut [&mut far_out])
            .map_err(|e| AudioError::Capture(format!("echo canceller: {e:?}")))?;
        apm.process_capture_f32(&[&mic_buf], &mut [&mut mic_out])
            .map_err(|e| AudioError::Capture(format!("echo canceller: {e:?}")))?;

        let n = frame.min(mic.len() - start);
        out.extend_from_slice(&mic_out[..n]);
    }
    Ok(out)
}

fn fill(buf: &mut [f32], src: &[f32], start: usize) {
    // `get` instead of indexing: `start` can be past the end of a shorter
    // track, where `&src[start..]` would panic.
    let tail = src.get(start..).unwrap_or(&[]);
    let n = tail.len().min(buf.len());
    buf[..n].copy_from_slice(&tail[..n]);
    buf[n..].fill(0.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const RATE: u32 = 16_000;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    fn db(ratio: f32) -> f32 {
        20.0 * ratio.log10()
    }

    /// Deterministic speech-like noise: white noise shaped by a slow
    /// amplitude envelope (syllables), so the canceller sees varied input.
    fn far_end(secs: usize) -> Vec<f32> {
        let mut state = 0x1234_5678_u32;
        (0..secs * RATE as usize)
            .map(|i| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                let noise = state as f32 / u32::MAX as f32 - 0.5;
                let envelope = 0.5 + 0.5 * (TAU * 3.0 * i as f32 / RATE as f32).sin();
                noise * envelope * 0.6
            })
            .collect()
    }

    #[test]
    fn suppresses_echo_and_keeps_near_end_speech() {
        let secs = 12;
        let far = far_end(secs);
        // The room: echo arrives 60 ms late at a third of the volume, with a
        // weaker reflection 25 ms after that.
        let (d1, d2) = (RATE as usize * 60 / 1000, RATE as usize * 85 / 1000);
        let echo: Vec<f32> = (0..far.len())
            .map(|i| {
                let a = if i >= d1 { far[i - d1] * 0.33 } else { 0.0 };
                let b = if i >= d2 { far[i - d2] * 0.1 } else { 0.0 };
                a + b
            })
            .collect();
        // The local speaker talks (a 220 Hz tone) only in the last 3 seconds.
        let near_start = (secs - 3) * RATE as usize;
        let near: Vec<f32> = (0..far.len())
            .map(|i| {
                if i >= near_start {
                    0.2 * (TAU * 220.0 * i as f32 / RATE as f32).sin()
                } else {
                    0.0
                }
            })
            .collect();
        let mic: Vec<f32> = echo.iter().zip(&near).map(|(e, n)| e + n).collect();

        let out = cancel_echo(&mic, &far, RATE).unwrap();
        assert_eq!(out.len(), mic.len());

        // Echo-only stretch, after a few seconds to converge: much quieter.
        let (a, b) = (5 * RATE as usize, near_start);
        let erle = db(rms(&mic[a..b]) / rms(&out[a..b]).max(1e-9));
        assert!(erle > 10.0, "echo reduced by only {erle:.1} dB");

        // Double talk: the near-end tone survives (within a few dB), even
        // though echo is still present underneath it.
        let near_kept = db(rms(&out[near_start..]) / rms(&near[near_start..]));
        assert!(near_kept > -6.0, "near-end speech lost {near_kept:.1} dB");
    }

    #[test]
    fn silence_in_silence_out_and_odd_lengths() {
        let out = cancel_echo(&[0.0; 1234], &[0.0; 99], RATE).unwrap();
        assert_eq!(out.len(), 1234);
        assert!(out.iter().all(|v| v.abs() < 1e-6));
    }
}
