//! Short sounds: dictation start/stop cues and the access-check chime.
//!
//! Generated from a few notes each (no audio files to ship) and played on the
//! default output device. Dictation's cues come in sets ([`Sounds`]) the user
//! picks from; Classic is the original pair of blips.

use std::f32::consts::TAU;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use serde::{Deserialize, Serialize};

use crate::AudioError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// Dictation started listening.
    Start,
    /// Dictation stopped listening.
    Stop,
    /// Dictation cancelled or failed.
    Cancel,
    /// The access check's two-note chime (louder and longer, so the system
    /// audio tap clearly hears it). The same whatever [`Sounds`] is chosen.
    Chime,
}

/// The set of sounds dictation plays: a setting.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sounds {
    /// Two sine blips, rising to start and falling to stop. The original.
    #[default]
    Classic,
    /// The same idea lower, slower and quieter.
    Soft,
    /// A quick pitch sweep, like a bubble popping.
    Bubble,
    /// A short knock, like tapping a wooden block.
    Wood,
    /// Bright, ringing glass.
    Glass,
    /// Two mellow marimba notes.
    Marimba,
    /// Two fast chirps, up to start and down to stop.
    Chirp,
    /// A plain electronic beep.
    Digital,
    /// Silence.
    Off,
}

impl Sounds {
    /// Every set, in the order offered.
    pub const ALL: [Self; 9] = [
        Self::Classic,
        Self::Soft,
        Self::Bubble,
        Self::Wood,
        Self::Glass,
        Self::Marimba,
        Self::Chirp,
        Self::Digital,
        Self::Off,
    ];
}

/// One note: a pitch (gliding to another over `glide` seconds, if they
/// differ), when it starts, and its overtones as `(multiple, level)`.
#[derive(Debug, Clone, Copy)]
struct Note {
    freq: f32,
    to: f32,
    glide: f32,
    start: f32,
    partials: &'static [(f32, f32)],
}

const SINE: &[(f32, f32)] = &[(1.0, 1.0)];
/// A wooden bar: the fundamental and an inharmonic knock above it.
const WOOD: &[(f32, f32)] = &[(1.0, 1.0), (2.76, 0.35)];
/// A struck glass: bright, inharmonic partials.
const GLASS: &[(f32, f32)] = &[(1.0, 1.0), (2.76, 0.4), (5.4, 0.15)];
/// A marimba bar: the fundamental and its fourth harmonic.
const MARIMBA: &[(f32, f32)] = &[(1.0, 1.0), (4.0, 0.25)];
/// A soft square wave: odd harmonics falling away.
const SQUARE: &[(f32, f32)] = &[(1.0, 1.0), (3.0, 0.33), (5.0, 0.2), (7.0, 0.14)];

const fn note(freq: f32, start: f32, partials: &'static [(f32, f32)]) -> Note {
    Note {
        freq,
        to: freq,
        glide: 0.0,
        start,
        partials,
    }
}

const fn glide(freq: f32, to: f32, glide: f32, start: f32) -> Note {
    Note {
        freq,
        to,
        glide,
        start,
        partials: SINE,
    }
}

/// How one cue sounds: its notes, how long it lasts, how quickly each note
/// swells and dies away, and how loud it is.
struct Shape {
    notes: Vec<Note>,
    length: f32,
    attack: f32,
    decay: f32,
    level: f32,
}

impl Cue {
    fn shape(self, sounds: Sounds) -> Option<Shape> {
        use Sounds::*;
        let shape = |notes: Vec<Note>, length, attack, decay, level| {
            Some(Shape {
                notes,
                length,
                attack,
                decay,
                level,
            })
        };
        match (self, sounds) {
            (Self::Chime, _) => shape(
                vec![note(659.25, 0.0, SINE), note(880.0, 0.18, SINE)],
                0.6,
                0.005,
                7.0,
                0.16,
            ),
            (_, Off) => None,

            (Self::Start, Classic) => shape(
                vec![note(880.0, 0.0, SINE), note(1318.5, 0.06, SINE)],
                0.16,
                0.005,
                7.0,
                0.08,
            ),
            (Self::Stop, Classic) => shape(
                vec![note(1318.5, 0.0, SINE), note(880.0, 0.06, SINE)],
                0.16,
                0.005,
                7.0,
                0.08,
            ),
            (Self::Cancel, Classic) => shape(
                vec![note(330.0, 0.0, SINE), note(330.0, 0.12, SINE)],
                0.24,
                0.005,
                7.0,
                0.08,
            ),

            (Self::Start, Soft) => shape(
                vec![note(523.25, 0.0, SINE), note(659.25, 0.09, SINE)],
                0.3,
                0.02,
                9.0,
                0.08,
            ),
            (Self::Stop, Soft) => shape(
                vec![note(659.25, 0.0, SINE), note(523.25, 0.09, SINE)],
                0.3,
                0.02,
                9.0,
                0.08,
            ),
            (Self::Cancel, Soft) => shape(
                vec![note(261.63, 0.0, SINE), note(261.63, 0.14, SINE)],
                0.32,
                0.02,
                9.0,
                0.09,
            ),

            (Self::Start, Bubble) => {
                shape(vec![glide(380.0, 950.0, 0.06, 0.0)], 0.13, 0.004, 16.0, 0.1)
            }
            (Self::Stop, Bubble) => {
                shape(vec![glide(950.0, 380.0, 0.06, 0.0)], 0.13, 0.004, 16.0, 0.1)
            }
            (Self::Cancel, Bubble) => shape(
                vec![
                    glide(500.0, 260.0, 0.05, 0.0),
                    glide(500.0, 260.0, 0.05, 0.11),
                ],
                0.22,
                0.004,
                16.0,
                0.1,
            ),

            (Self::Start, Wood) => shape(vec![note(1180.0, 0.0, WOOD)], 0.09, 0.002, 40.0, 0.105),
            (Self::Stop, Wood) => shape(vec![note(820.0, 0.0, WOOD)], 0.09, 0.002, 40.0, 0.105),
            (Self::Cancel, Wood) => shape(
                vec![note(520.0, 0.0, WOOD), note(520.0, 0.1, WOOD)],
                0.19,
                0.002,
                40.0,
                0.105,
            ),

            (Self::Start, Glass) => shape(
                vec![note(1568.0, 0.0, GLASS), note(2093.0, 0.05, GLASS)],
                0.45,
                0.003,
                6.5,
                0.05,
            ),
            (Self::Stop, Glass) => shape(
                vec![note(2093.0, 0.0, GLASS), note(1568.0, 0.05, GLASS)],
                0.45,
                0.003,
                6.5,
                0.05,
            ),
            (Self::Cancel, Glass) => shape(
                vec![note(784.0, 0.0, GLASS), note(784.0, 0.12, GLASS)],
                0.45,
                0.003,
                6.5,
                0.05,
            ),

            (Self::Start, Marimba) => shape(
                vec![note(659.25, 0.0, MARIMBA), note(987.77, 0.07, MARIMBA)],
                0.26,
                0.003,
                13.0,
                0.082,
            ),
            (Self::Stop, Marimba) => shape(
                vec![note(987.77, 0.0, MARIMBA), note(659.25, 0.07, MARIMBA)],
                0.26,
                0.003,
                13.0,
                0.082,
            ),
            (Self::Cancel, Marimba) => shape(
                vec![note(392.0, 0.0, MARIMBA), note(392.0, 0.12, MARIMBA)],
                0.28,
                0.003,
                13.0,
                0.082,
            ),

            (Self::Start, Chirp) => shape(
                vec![
                    glide(1900.0, 2900.0, 0.04, 0.0),
                    glide(1900.0, 2900.0, 0.04, 0.07),
                ],
                0.16,
                0.003,
                20.0,
                0.075,
            ),
            (Self::Stop, Chirp) => shape(
                vec![
                    glide(2900.0, 1900.0, 0.04, 0.0),
                    glide(2900.0, 1900.0, 0.04, 0.07),
                ],
                0.16,
                0.003,
                20.0,
                0.075,
            ),
            (Self::Cancel, Chirp) => shape(
                vec![
                    glide(1400.0, 900.0, 0.05, 0.0),
                    glide(1400.0, 900.0, 0.05, 0.1),
                ],
                0.2,
                0.003,
                20.0,
                0.075,
            ),

            (Self::Start, Digital) => {
                shape(vec![note(1000.0, 0.0, SQUARE)], 0.09, 0.003, 5.0, 0.09)
            }
            (Self::Stop, Digital) => shape(vec![note(750.0, 0.0, SQUARE)], 0.09, 0.003, 5.0, 0.09),
            (Self::Cancel, Digital) => shape(
                vec![note(400.0, 0.0, SQUARE), note(400.0, 0.12, SQUARE)],
                0.21,
                0.003,
                5.0,
                0.06,
            ),
        }
    }

    /// The cue's samples at `sample_rate`; empty for silence.
    pub fn samples(self, sounds: Sounds, sample_rate: u32) -> Vec<f32> {
        let Some(shape) = self.shape(sounds) else {
            return Vec::new();
        };
        let rate = sample_rate as f32;
        let len = (shape.length * rate) as usize;
        (0..len)
            .map(|i| {
                let t = i as f32 / rate;
                let tone: f32 = shape
                    .notes
                    .iter()
                    .filter(|n| t >= n.start)
                    .map(|n| {
                        let since = t - n.start;
                        // The attack swells in (no click), then exponential decay.
                        let envelope =
                            (since / shape.attack).min(1.0) * (-since * shape.decay).exp();
                        let phase = TAU * cycles(n, since);
                        let voice: f32 = n
                            .partials
                            .iter()
                            .map(|&(multiple, level)| (phase * multiple).sin() * level)
                            .sum();
                        voice * envelope
                    })
                    .sum();
                // Fade the tail to exactly zero, so stopping doesn't click.
                let fade = ((len - i) as f32 / (0.03 * rate)).min(1.0);
                tone * shape.level * fade
            })
            .collect()
    }

    fn duration(self, sounds: Sounds) -> Duration {
        Duration::from_secs_f32(self.shape(sounds).map_or(0.0, |s| s.length))
    }
}

/// Cycles a note has completed `since` seconds in, gliding linearly from its
/// pitch to `to` over `glide` seconds and holding there.
fn cycles(n: &Note, since: f32) -> f32 {
    if n.glide <= 0.0 || n.to == n.freq {
        return n.freq * since;
    }
    let slope = (n.to - n.freq) / n.glide;
    let t = since.min(n.glide);
    let gliding = n.freq * t + slope * t * t / 2.0;
    gliding + n.to * (since - t)
}

/// Plays a cue without blocking: a short-lived thread owns the output stream
/// until the sound has finished. Failures are logged, not returned; a missing
/// beep shouldn't stop anything.
pub fn play(cue: Cue, sounds: Sounds) {
    if cue.shape(sounds).is_none() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("quill-cue".into())
        .spawn(move || match start(cue, sounds) {
            Ok(stream) => {
                std::thread::sleep(cue.duration(sounds) + Duration::from_millis(50));
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
pub fn start(cue: Cue, sounds: Sounds) -> Result<cpal::Stream, AudioError> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or(AudioError::NoDevice("audio output"))?;
    let supported = device
        .default_output_config()
        .map_err(|e| AudioError::device("reading output format", e))?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let samples = cue.samples(sounds, config.sample_rate);
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

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn cues_are_quiet_audible_and_click_free() {
        for sounds in Sounds::ALL {
            for cue in [Cue::Start, Cue::Stop, Cue::Cancel, Cue::Chime] {
                let samples = cue.samples(sounds, 48_000);
                if sounds == Sounds::Off && cue != Cue::Chime {
                    assert!(samples.is_empty(), "{sounds:?} {cue:?} should be silent");
                    continue;
                }
                let peak = peak(&samples);
                assert!(
                    (0.03..0.25).contains(&peak),
                    "{sounds:?} {cue:?} peak {peak}"
                );
                assert!(
                    samples[0].abs() < 1e-6,
                    "{sounds:?} {cue:?} starts with a click"
                );
                assert!(
                    samples[samples.len() - 1].abs() < 1e-3,
                    "{sounds:?} {cue:?} ends with a click"
                );
            }
        }
        assert_eq!(Cue::Chime.samples(Sounds::Off, 48_000).len(), 28_800);
    }

    #[test]
    fn sets_are_about_as_loud_as_classic() {
        let classic = peak(&Cue::Start.samples(Sounds::Classic, 48_000));
        for sounds in Sounds::ALL.into_iter().filter(|s| *s != Sounds::Off) {
            for cue in [Cue::Start, Cue::Stop, Cue::Cancel] {
                let p = peak(&cue.samples(sounds, 48_000));
                assert!(
                    (0.5..=1.6).contains(&(p / classic)),
                    "{sounds:?} {cue:?} peak {p:.3} against classic {classic:.3}"
                );
            }
        }
    }

    #[test]
    fn classic_is_the_original_sound_sample_for_sample() {
        // The formula before sound sets: sine notes, 5 ms attack, decay 7,
        // a 30 ms fade at the end.
        let original = |notes: &[(f32, f32)], length: f32, level: f32| -> Vec<f32> {
            let rate = 48_000.0_f32;
            let len = (length * rate) as usize;
            (0..len)
                .map(|i| {
                    let t = i as f32 / rate;
                    let tone: f32 = notes
                        .iter()
                        .filter(|&&(_, start)| t >= start)
                        .map(|&(freq, start)| {
                            let since = t - start;
                            let envelope = (since / 0.005).min(1.0) * (-since * 7.0).exp();
                            (TAU * freq * since).sin() * envelope
                        })
                        .sum();
                    let fade = ((len - i) as f32 / (0.03 * rate)).min(1.0);
                    tone * level * fade
                })
                .collect()
        };
        let cases = [
            (
                Cue::Start,
                original(&[(880.0, 0.0), (1318.5, 0.06)], 0.16, 0.08),
            ),
            (
                Cue::Stop,
                original(&[(1318.5, 0.0), (880.0, 0.06)], 0.16, 0.08),
            ),
            (
                Cue::Cancel,
                original(&[(330.0, 0.0), (330.0, 0.12)], 0.24, 0.08),
            ),
            (
                Cue::Chime,
                original(&[(659.25, 0.0), (880.0, 0.18)], 0.6, 0.16),
            ),
        ];
        for (cue, expected) in cases {
            let now = cue.samples(Sounds::Classic, 48_000);
            assert_eq!(now.len(), expected.len(), "{cue:?}");
            let worst = now
                .iter()
                .zip(&expected)
                .fold(0.0_f32, |m, (a, b)| m.max((a - b).abs()));
            // Float rounding only (the phase is multiplied out in another
            // order): about 90 dB below the sound.
            assert!(worst < 1e-5, "{cue:?} differs by {worst}");
        }
    }

    #[test]
    fn a_glide_sweeps_between_its_pitches() {
        let n = glide(400.0, 800.0, 0.1, 0.0);
        // Halfway through, the pitch is 600 Hz: the cycles so far average 500.
        assert!((cycles(&n, 0.05) - 25.0).abs() < 1e-3);
        // After the glide it holds 800 Hz.
        let after = cycles(&n, 0.2) - cycles(&n, 0.1);
        assert!((after - 80.0).abs() < 1e-3);
    }
}
