//! Turning a two-track meeting recording into one labelled transcript.
//!
//! The recorder writes `mic.wav` ("Me") and `system.wav` ("Others"), aligned
//! so that sample 0 is the same moment in both. Transcribing each track and
//! interleaving the segments by time gives reliable speaker labels without
//! any diarization model.
//!
//! Once transcribed, [`compact_recording`] replaces the WAVs with 16 kHz FLAC
//! (`mic.flac`, `system.flac`); everything here reads either.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use quill_audio::capture::{Source, TrackSummary};
use quill_stt::{Segment, SttError, TranscribeOptions, Transcriber};

/// What the recorder writes.
pub const MIC_FILE: &str = "mic.wav";
pub const SYSTEM_FILE: &str = "system.wav";
/// What [`compact_recording`] keeps.
pub const MIC_FLAC: &str = "mic.flac";
pub const SYSTEM_FLAC: &str = "system.flac";
pub const ME: &str = "Me";
pub const OTHERS: &str = "Others";

/// Errors from processing a meeting recording.
#[derive(Debug, thiserror::Error)]
pub enum MeetingError {
    #[error("no mic or system audio in {0}")]
    NoTracks(PathBuf),
    #[error("could not read {path}")]
    Audio {
        path: PathBuf,
        source: quill_audio::AudioError,
    },
    #[error(transparent)]
    Stt(#[from] SttError),
    #[error(transparent)]
    Diarize(#[from] quill_diarize::DiarizeError),
}

/// File next to the WAVs describing what was recorded (devices, formats,
/// lead-in, dropped samples, peaks), for debugging sync or permission
/// problems later.
pub const RECORDING_INFO_FILE: &str = "recording.json";

/// Writes [`RECORDING_INFO_FILE`] for a finished recording.
pub fn write_recording_info(dir: &Path, tracks: &[TrackSummary]) -> std::io::Result<()> {
    let tracks: Vec<_> = tracks
        .iter()
        .map(|t| {
            serde_json::json!({
                "source": match t.source {
                    Source::Microphone => "microphone",
                    Source::SystemAudio => "system",
                },
                "file": t.path.file_name().map(|f| f.to_string_lossy()),
                "device": t.device,
                "sample_rate": t.sample_rate,
                "channels": t.channels,
                "frames": t.frames,
                "lead_in_ms": t.lead_in_ms,
                "dropped_samples": t.dropped_samples,
                "peak": t.peak,
            })
        })
        .collect();
    let json = serde_json::to_string_pretty(&serde_json::json!({ "tracks": tracks }))?;
    std::fs::write(dir.join(RECORDING_INFO_FILE), json)
}

/// Transcribes whichever tracks exist in a recording directory and merges
/// them into one transcript labelled "Me" / "Others". With
/// `echo_cancellation`, speaker sound is first removed from the mic track
/// using the system track as the reference (see `quill_audio::aec`).
pub fn transcribe_recording(
    dir: &Path,
    transcriber: &dyn Transcriber,
    opts: &TranscribeOptions,
    echo_cancellation: bool,
) -> Result<Vec<Segment>, MeetingError> {
    transcribe_recording_with(dir, transcriber, opts, echo_cancellation, None)
}

/// Which track of a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Track {
    /// The microphone: you.
    Mic,
    /// The computer's sound: the call.
    Call,
}

/// What [`transcribe_recording_with`] reports as it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// The tracks are loaded (and the echo removed): which ones there are.
    Ready { mic: bool, call: bool },
    /// How far through a track Whisper is, 0–100.
    Percent(Track, u8),
    /// A line from a track, as soon as it's recognised.
    Line(Track, String),
    /// A track is done.
    Done(Track),
}

/// [`transcribe_recording`], telling `report` how it's going.
pub fn transcribe_recording_with(
    dir: &Path,
    transcriber: &dyn Transcriber,
    opts: &TranscribeOptions,
    echo_cancellation: bool,
    report: Option<std::sync::Arc<dyn Fn(Phase) + Send + Sync>>,
) -> Result<Vec<Segment>, MeetingError> {
    let mut mic = load_track(track_path(dir, MIC_FLAC, MIC_FILE).as_deref())?;
    let system = load_track(track_path(dir, SYSTEM_FLAC, SYSTEM_FILE).as_deref())?;
    if mic.is_none() && system.is_none() {
        return Err(MeetingError::NoTracks(dir.to_path_buf()));
    }
    // Remove what the speakers played from the mic before transcribing, so
    // the other people's words don't end up in "Me" lines.
    if echo_cancellation && let (Some(m), Some(s)) = (mic.as_mut(), system.as_ref()) {
        tracing::info!("cancelling speaker echo in the mic track");
        *m = quill_audio::aec::cancel_echo(m, s, quill_audio::STT_SAMPLE_RATE).map_err(
            |source| MeetingError::Audio {
                path: dir.join(MIC_FILE),
                source,
            },
        )?;
    }
    if let Some(report) = &report {
        report(Phase::Ready {
            mic: mic.is_some(),
            call: system.is_some(),
        });
    }
    let transcribe = |pcm: Option<Vec<f32>>, track: Track| -> Result<Vec<Segment>, MeetingError> {
        let Some(pcm) = pcm else {
            return Ok(Vec::new());
        };
        tracing::info!(track = ?track, "transcribing");
        let mut opts = opts.clone();
        if let Some(report) = &report {
            let report = std::sync::Arc::clone(report);
            opts.progress = Some(quill_stt::Progress::new(move |event| {
                report(match event {
                    quill_stt::ProgressEvent::Percent(p) => Phase::Percent(track, p),
                    quill_stt::ProgressEvent::Line(text) => Phase::Line(track, text),
                });
            }));
        }
        let segments = transcriber.transcribe(&pcm, &opts)?;
        if let Some(report) = &report {
            report(Phase::Done(track));
        }
        Ok(segments)
    };
    let mic_segments = transcribe(mic, Track::Mic)?;
    let system_segments = transcribe(system, Track::Call)?;
    Ok(merge_tracks(mic_segments, system_segments))
}

/// 16 kHz mono samples of a track, or `None` if the file doesn't exist.
/// The stored track: the FLAC if compacted (it's only renamed into place once
/// complete), else the WAV, else `None`.
fn track_path(dir: &Path, flac: &str, wav: &str) -> Option<PathBuf> {
    [flac, wav]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

/// The system-audio track as 16 kHz mono, if the recording has one.
pub fn load_system_track(dir: &Path) -> Result<Option<Vec<f32>>, MeetingError> {
    load_track(track_path(dir, SYSTEM_FLAC, SYSTEM_FILE).as_deref())
}

/// Whether a recording folder has any audio.
pub fn has_audio(dir: &Path) -> bool {
    track_path(dir, MIC_FLAC, MIC_FILE).is_some()
        || track_path(dir, SYSTEM_FLAC, SYSTEM_FILE).is_some()
}

/// Whether a recording folder still has uncompacted WAVs.
pub fn has_wav(dir: &Path) -> bool {
    dir.join(MIC_FILE).is_file() || dir.join(SYSTEM_FILE).is_file()
}

fn load_track(path: Option<&Path>) -> Result<Option<Vec<f32>>, MeetingError> {
    let Some(path) = path else {
        return Ok(None);
    };
    quill_audio::load_for_stt(path)
        .map(Some)
        .map_err(|source| MeetingError::Audio {
            path: path.to_path_buf(),
            source,
        })
}

/// Space saved by [`compact_recording`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Compacted {
    pub tracks: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

/// Replaces a recording's WAVs with 16 kHz mono FLAC, the audio the
/// pipeline actually uses (see `quill_audio::write_flac`). A WAV is deleted
/// only after its FLAC has been written and read back with the same length.
/// Folders already compacted are left alone. Don't run it on a recording in
/// progress.
pub fn compact_recording(dir: &Path) -> Result<Compacted, MeetingError> {
    let mut done = Compacted::default();
    for (wav, flac) in [(MIC_FILE, MIC_FLAC), (SYSTEM_FILE, SYSTEM_FLAC)] {
        let (wav, flac) = (dir.join(wav), dir.join(flac));
        if !wav.is_file() {
            continue;
        }
        let audio_error = |path: &Path| {
            let path = path.to_path_buf();
            move |source| MeetingError::Audio { path, source }
        };
        if !flac.is_file() {
            let pcm = quill_audio::load_for_stt(&wav).map_err(audio_error(&wav))?;
            quill_audio::write_flac(&flac, &pcm, quill_audio::STT_SAMPLE_RATE)
                .map_err(audio_error(&flac))?;
            let check = quill_audio::read_flac(&flac).map_err(audio_error(&flac))?;
            if check.samples.len() != pcm.len() {
                let _ = std::fs::remove_file(&flac);
                return Err(MeetingError::Audio {
                    path: flac,
                    source: quill_audio::AudioError::Flac(
                        "read back a different length than was written".into(),
                    ),
                });
            }
        }
        let size = |path: &Path| std::fs::metadata(path).map_or(0, |m| m.len());
        done.bytes_before += size(&wav);
        done.bytes_after += size(&flac);
        done.tracks += 1;
        std::fs::remove_file(&wav).map_err(|e| MeetingError::Audio {
            path: wav.clone(),
            source: e.into(),
        })?;
    }
    Ok(done)
}

/// Labels and interleaves the two tracks, dropping echo.
///
/// Echo: when the meeting plays through speakers, the mic hears the other
/// people too, so their words show up on both tracks. A mic segment is
/// dropped when most of its words also appear in the system segments that
/// overlap it in time. (All overlapping ones together: the two tracks often
/// split the same sentence at different points.)
/// (With headphones there is no echo and nothing is dropped.)
pub fn merge_tracks(mic: Vec<Segment>, system: Vec<Segment>) -> Vec<Segment> {
    // Iterators are lazy, so the echo filter would still be borrowing
    // `system` when `.chain(system.into_iter())` tries to move it. Collecting
    // the filtered mic segments first ends that borrow.
    let mut merged: Vec<Segment> = mic
        .into_iter()
        .filter(|m| !is_echo(m, &system))
        .map(|s| with_speaker(s, ME))
        .collect();
    merged.extend(system.into_iter().map(|s| with_speaker(s, OTHERS)));
    // `sort_by_key` is stable: segments starting together keep mic-then-system
    // order, so the output is deterministic.
    merged.sort_by_key(|s| s.start_ms);
    merged
}

fn with_speaker(segment: Segment, speaker: &str) -> Segment {
    Segment {
        speaker: Some(speaker.to_string()),
        ..segment
    }
}

/// Longest pause (ms) within one speaker's turn.
pub const TURN_MAX_GAP_MS: u64 = 5_000;
/// Longest turn (ms) before starting a new one, so timestamps stay useful.
pub const TURN_MAX_LEN_MS: u64 = 60_000;

/// Joins consecutive segments by the same speaker into turns.
///
/// Whisper cuts segments at arbitrary points ("…identified based on" /
/// "which audio device…"). Merging them reads better and gives the LLM
/// whole sentences. A new turn starts when the speaker changes, after a
/// pause longer than [`TURN_MAX_GAP_MS`], or once a turn reaches
/// [`TURN_MAX_LEN_MS`]. (The app's transcript view groups with the same
/// rules.)
pub fn turns(segments: &[Segment]) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::with_capacity(segments.len());
    for s in segments {
        if let Some(last) = out.last_mut()
            && last.speaker == s.speaker
            && s.start_ms <= last.end_ms + TURN_MAX_GAP_MS
            && s.end_ms.saturating_sub(last.start_ms) <= TURN_MAX_LEN_MS
        {
            last.text.push(' ');
            last.text.push_str(&s.text);
            last.end_ms = last.end_ms.max(s.end_ms);
            continue;
        }
        out.push(s.clone());
    }
    out
}

/// How far apart (ms) two segments can be and still count as overlapping.
/// Echo arrives a little late, and segment boundaries differ per track.
const ECHO_SLACK_MS: u64 = 1500;
/// Fraction of the mic segment's words that must also be in the system
/// segment for it to count as echo.
const ECHO_WORD_OVERLAP: f64 = 0.6;

fn is_echo(mic: &Segment, system: &[Segment]) -> bool {
    let mic_words = words(&mic.text);
    if mic_words.is_empty() {
        return false;
    }
    let nearby_words: HashSet<String> = system
        .iter()
        .filter(|s| {
            mic.start_ms <= s.end_ms + ECHO_SLACK_MS && s.start_ms <= mic.end_ms + ECHO_SLACK_MS
        })
        .flat_map(|s| words(&s.text))
        .collect();
    let shared = mic_words.intersection(&nearby_words).count();
    shared as f64 / mic_words.len() as f64 >= ECHO_WORD_OVERLAP
}

/// Lowercased words without punctuation, as a set.
fn words(text: &str) -> HashSet<String> {
    text.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric() || *c == '\'')
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start_ms: u64, end_ms: u64, text: &str) -> Segment {
        Segment {
            start_ms,
            end_ms,
            text: text.into(),
            speaker: None,
        }
    }

    fn summary(segments: &[Segment]) -> Vec<(u64, &str, &str)> {
        segments
            .iter()
            .map(|s| {
                (
                    s.start_ms,
                    s.speaker.as_deref().unwrap_or(""),
                    s.text.as_str(),
                )
            })
            .collect()
    }

    #[test]
    fn compaction_replaces_wavs_with_flac_that_reads_the_same() {
        let dir = tempfile::tempdir().unwrap();
        // A 48 kHz stereo float WAV, like the system-audio capture.
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(dir.path().join(SYSTEM_FILE), spec).unwrap();
        for i in 0..48_000 {
            let s = (i as f32 * 0.03).sin() * 0.2;
            writer.write_sample(s).unwrap();
            writer.write_sample(s).unwrap();
        }
        writer.finalize().unwrap();
        let before = quill_audio::load_for_stt(&dir.path().join(SYSTEM_FILE)).unwrap();

        let done = compact_recording(dir.path()).unwrap();

        assert_eq!(done.tracks, 1);
        assert!(done.bytes_after * 10 < done.bytes_before, "{done:?}");
        assert!(!dir.path().join(SYSTEM_FILE).exists());
        assert!(has_audio(dir.path()));
        let after = load_track(track_path(dir.path(), SYSTEM_FLAC, SYSTEM_FILE).as_deref())
            .unwrap()
            .unwrap();
        assert_eq!(after.len(), before.len());
        assert!(before.iter().zip(&after).all(|(a, b)| (a - b).abs() < 1e-4));
        // Running it again finds nothing to do.
        assert_eq!(compact_recording(dir.path()).unwrap().tracks, 0);
    }

    #[test]
    fn interleaves_and_labels_by_time() {
        let mic = vec![seg(0, 2000, "Hi all."), seg(9000, 11_000, "Sounds good.")];
        let system = vec![seg(3000, 8000, "Hello, let's start with the roadmap.")];

        let merged = merge_tracks(mic, system);

        assert_eq!(
            summary(&merged),
            vec![
                (0, "Me", "Hi all."),
                (3000, "Others", "Hello, let's start with the roadmap."),
                (9000, "Me", "Sounds good."),
            ]
        );
    }

    #[test]
    fn drops_mic_echo_of_system_audio() {
        // Speakers, not headphones: the mic hears the remote speaker ~200 ms
        // late, transcribed slightly differently.
        let system = vec![seg(3000, 8000, "Let's start with the Q4 roadmap, okay?")];
        let mic = vec![
            seg(3200, 8100, "let's start with the Q4 road map okay"),
            seg(9000, 10_000, "Sure, go ahead."),
        ];

        let merged = merge_tracks(mic, system);

        assert_eq!(
            summary(&merged),
            vec![
                (3000, "Others", "Let's start with the Q4 roadmap, okay?"),
                (9000, "Me", "Sure, go ahead."),
            ]
        );
    }

    #[test]
    fn drops_echo_split_across_system_segments() {
        let system = vec![
            seg(0, 4000, "First hire should be a senior backend engineer."),
            seg(4000, 8000, "The second can be more of a generalist."),
            seg(
                8000,
                12_000,
                "Honestly, on-call load is the main pain point.",
            ),
        ];
        // The mic split the same speech differently: no single system
        // segment holds 60% of these words, but together they do.
        let mic = vec![seg(
            4200,
            12_100,
            "The second can be more of a generalist. Honestly, on-call load is the main pain point.",
        )];

        let merged = merge_tracks(mic, system);

        assert!(merged.iter().all(|s| s.speaker.as_deref() == Some(OTHERS)));
    }

    #[test]
    fn keeps_mic_speech_that_only_shares_common_words() {
        // Talking over someone isn't echo just because both say "the".
        let system = vec![seg(0, 5000, "The budget is the main concern for the team.")];
        let mic = vec![seg(1000, 4000, "Can you share the slides?")];

        let merged = merge_tracks(mic, system);

        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn merges_consecutive_segments_into_turns() {
        let segs = vec![
            seg(0, 3000, "identified based on"),
            seg(3200, 6000, "which audio device"),
            seg(30_000, 31_000, "Later."),
        ];
        let mut others = seg(6500, 7000, "Right.");
        others.speaker = Some(OTHERS.into());
        let mut all = merge_tracks(segs, vec![others]);
        all.push(Segment {
            speaker: Some(ME.into()),
            ..seg(31_500, 32_000, "And more.")
        });

        let t = turns(&all);

        let summary: Vec<_> = t
            .iter()
            .map(|s| (s.start_ms, s.end_ms, s.text.as_str()))
            .collect();
        assert_eq!(
            summary,
            vec![
                (0, 6000, "identified based on which audio device"),
                (6500, 7000, "Right."),
                // The 23 s pause would also split, but the speaker change
                // already did; "Later." and "And more." are 0.5 s apart.
                (30_000, 32_000, "Later. And more."),
            ]
        );
    }

    #[test]
    fn long_monologues_are_split_into_turns_of_at_most_a_minute() {
        let segs: Vec<Segment> = (0..30)
            .map(|i| Segment {
                speaker: Some(ME.into()),
                ..seg(i * 4000, i * 4000 + 3900, "words")
            })
            .collect();
        let t = turns(&segs);
        assert!(t.len() >= 2);
        assert!(t.iter().all(|s| s.end_ms - s.start_ms <= TURN_MAX_LEN_MS));
    }

    #[test]
    fn same_words_far_apart_are_not_echo() {
        let system = vec![seg(0, 2000, "Thanks everyone.")];
        let mic = vec![seg(60_000, 62_000, "Thanks everyone.")];

        assert_eq!(merge_tracks(mic, system).len(), 2);
    }
}
