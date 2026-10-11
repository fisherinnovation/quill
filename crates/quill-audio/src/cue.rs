//! Short sounds: dictation start/stop cues and the access-check chime.
//!
//! Generated as sine tones (no audio files to ship) and played on the default
//! output device.

use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

use crate::AudioError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// Dictation started listening: a quick rising blip.
    Start,
    /// Dictation stopped listening: the same blip falling.
    Stop,
    /// Dictation cancelled or failed: two low blips.
    Cancel,
    /// The access check's two-note chime (louder and longer, so the system
    /// audio tap clearly hears it).
    Chime,
}

impl Cue {
    /// `(frequency Hz, start s)` per note, note length, peak level.
    fn notes(self) -> (&'static [(f32, f32)], f32, f32) {
        match self {
            Self::Start => (&[(880.0, 0.0), (1318.5, 0.06)], 0.16, 0.08),
            Self::Stop => (&[(1318.5, 0.0), (880.0, 0.06)], 0.16, 0.08),
            Self::Cancel => (&[(330.0, 0.0), (330.0, 0.12)], 0.24, 0.08),
            Self::Chime => (&[(659.25, 0.0), (880.0, 0.18)], 0.6, 0.16),
        }
    }

    pub fn samples(self, sample_rate: u32) -> Vec<f32> {
        let (notes, length_secs, level) = self.notes();
        let rate = sample_rate as f32;
        let len = (length_secs * rate) as usize;
        (0..len)
            .map(|i| {
                let t = i as f32 / rate;
                let tone: f32 = notes
                    .iter()
                    .filter(|&&(_, start)| t >= start)
                    .map(|&(freq, start)| {
                        let since = t - start;
                        // 5 ms attack (no click), then exponential decay.
                        let envelope = (since / 0.005).min(1.0) * (-since * 7.0).exp();
                        (std::f32::consts::TAU * freq * since).sin() * envelope
                    })
                    .sum();
                // Fade the tail to exactly zero, so stopping doesn't click.
                let fade = ((len - i) as f32 / (0.03 * rate)).min(1.0);
                tone * level * fade
            })
            .collect()
    }

    fn duration(self) -> Duration {
        Duration::from_secs_f32(self.notes().1)
    }
}

/// Plays a cue without blocking: a short-lived thread owns the output stream
/// until the sound has finished. Failures are logged, not returned; a missing
/// beep shouldn't stop anything.
pub fn play(cue: Cue) {
    let spawned = std::thread::Builder::new()
        .name("quill-cue".into())
        .spawn(move || match start(cue) {
            Ok(stream) => {
                std::thread::sleep(cue.duration() + Duration::from_millis(50));
                drop(stream);
            }
            Err(e) => tracing::debug!("could not play {cue:?}: {e}"),
        });
    if let Err(e) = spawned {
        tracing::debug!("could not play {cue:?}: {e}");
    }
}

/// Starts `cue` on the default output device. Dropping the returned stream
/// stops it.
pub fn start(cue: Cue) -> Result<cpal::Stream, AudioError> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or(AudioError::NoDevice("audio output"))?;
    let supported = device
        .default_output_config()
        .map_err(|e| AudioError::device("reading output format", e))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let samples = cue.samples(config.sample_rate);
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, &config, samples),
        SampleFormat::I16 => build::<i16>(&device, &config, samples),
        SampleFormat::I32 => build::<i32>(&device, &config, samples),
        SampleFormat::U16 => build::<u16>(&device, &config, samples),
        other => Err(AudioError::UnsupportedFormat(format!(
            "output format {other}"
        ))),
    }?;
    stream
        .play()
        .map_err(|e| AudioError::device("starting sound", e))?;
    Ok(stream)
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    samples: Vec<f32>,
) -> Result<cpal::Stream, AudioError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels).max(1);
    let mut next = 0;
    device
        .build_output_stream::<T, _, _>(
            *config,
            // Same sample on every channel; silence once the sound is over.
            move |data: &mut [T], _info| {
                for frame in data.chunks_mut(channels) {
                    let sample = samples.get(next).copied().unwrap_or(0.0);
                    next += 1;
                    frame.fill(T::from_sample(sample));
                }
            },
            |err| tracing::warn!("sound stream error: {err}"),
            None,
        )
        .map_err(|e| AudioError::device("opening output stream", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cues_are_quiet_audible_and_click_free() {
        for cue in [Cue::Start, Cue::Stop, Cue::Cancel, Cue::Chime] {
            let samples = cue.samples(48_000);
            let peak = samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
            assert!((0.03..0.25).contains(&peak), "{cue:?} peak {peak}");
            assert!(samples[0].abs() < 1e-6, "{cue:?} starts with a click");
            assert!(
                samples[samples.len() - 1].abs() < 1e-3,
                "{cue:?} ends with a click"
            );
        }
        assert_eq!(Cue::Chime.samples(48_000).len(), 28_800);
    }
}
