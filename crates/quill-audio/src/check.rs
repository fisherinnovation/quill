//! A short test recording that confirms Quill can hear both sources.
//!
//! On macOS a missing permission doesn't fail: the stream opens and delivers
//! silence. So the check records a few seconds from the microphone and the
//! system output while playing a quiet chime through the speakers. The system
//! tap records every process, Quill included, so it should hear the chime;
//! silence there means access was denied. A working microphone always picks
//! up some noise, so exact silence there means the same.
//!
//! Opening the devices is also what triggers the macOS permission prompts, so
//! running this during onboarding gets them out of the way before the first
//! meeting.

use std::path::Path;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

use crate::AudioError;
use crate::capture::{self, Source, TrackRequest, TrackSummary};

/// Records both sources into `dir` for `duration`, playing a chime partway
/// through, and returns what each track heard (see [`TrackSummary::peak`]).
/// The caller decides what to do with the files.
pub fn check_access(dir: &Path, duration: Duration) -> Result<Vec<TrackSummary>, AudioError> {
    std::fs::create_dir_all(dir)?;
    let requests = [
        TrackRequest {
            source: Source::Microphone,
            path: dir.join("mic.wav"),
        },
        TrackRequest {
            source: Source::SystemAudio,
            path: dir.join("system.wav"),
        },
    ];
    let recording = capture::start(&requests)?;
    // Give the tap a moment to start before the chime plays.
    std::thread::sleep(Duration::from_millis(300));
    let chime = play_chime();
    if let Err(e) = &chime {
        // Not fatal: the microphone result is still useful, and the system
        // track may pick up other audio.
        tracing::warn!("could not play the test chime: {e}");
    }
    std::thread::sleep(duration.saturating_sub(Duration::from_millis(300)));
    drop(chime);
    recording.stop()
}

/// Starts the chime on the default output device. Dropping the returned
/// stream stops it.
fn play_chime() -> Result<cpal::Stream, AudioError> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or(AudioError::NoDevice("audio output"))?;
    let supported = device
        .default_output_config()
        .map_err(|e| AudioError::device("reading output format", e))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let samples = chime(config.sample_rate);
    let stream = match format {
        SampleFormat::F32 => build_output::<f32>(&device, &config, samples),
        SampleFormat::I16 => build_output::<i16>(&device, &config, samples),
        SampleFormat::I32 => build_output::<i32>(&device, &config, samples),
        SampleFormat::U16 => build_output::<u16>(&device, &config, samples),
        other => Err(AudioError::UnsupportedFormat(format!(
            "output format {other}"
        ))),
    }?;
    stream
        .play()
        .map_err(|e| AudioError::device("starting chime", e))?;
    Ok(stream)
}

fn build_output<T>(
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
            // Same sample on every channel; silence once the chime is over.
            move |data: &mut [T], _info| {
                for frame in data.chunks_mut(channels) {
                    let sample = samples.get(next).copied().unwrap_or(0.0);
                    next += 1;
                    frame.fill(T::from_sample(sample));
                }
            },
            |err| tracing::warn!("chime stream error: {err}"),
            None,
        )
        .map_err(|e| AudioError::device("opening output stream", e))
}

/// Two soft rising notes (E5, A5), about 0.6 s, peaking around -16 dBFS.
fn chime(sample_rate: u32) -> Vec<f32> {
    const NOTES: [(f32, f32); 2] = [(659.25, 0.0), (880.0, 0.18)];
    const LENGTH_SECS: f32 = 0.6;
    const LEVEL: f32 = 0.16;
    let rate = sample_rate as f32;
    let len = (LENGTH_SECS * rate) as usize;
    (0..len)
        .map(|i| {
            let t = i as f32 / rate;
            let tone: f32 = NOTES
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
            let fade = ((len - i) as f32 / (0.05 * rate)).min(1.0);
            tone * LEVEL * fade
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chime_is_quiet_audible_and_click_free() {
        let samples = chime(48_000);
        assert_eq!(samples.len(), 28_800);
        let peak = samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!((0.05..0.25).contains(&peak), "peak {peak}");
        assert!(samples[0].abs() < 1e-6);
        assert!(samples[samples.len() - 1].abs() < 1e-3);
    }
}
