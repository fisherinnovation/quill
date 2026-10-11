//! Turning a two-track meeting recording into one labelled transcript.
//!
//! The recorder writes `mic.wav` ("Me") and `system.wav` ("Others"), aligned
//! so that sample 0 is the same moment in both. Transcribing each track and
//! interleaving the segments by time gives reliable speaker labels without
//! any diarization model.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use quill_stt::{Segment, SttError, TranscribeOptions, Transcriber};

pub const MIC_FILE: &str = "mic.wav";
pub const SYSTEM_FILE: &str = "system.wav";
pub const ME: &str = "Me";
pub const OTHERS: &str = "Others";

/// Errors from processing a meeting recording.
#[derive(Debug, thiserror::Error)]
pub enum MeetingError {
    #[error("no {MIC_FILE} or {SYSTEM_FILE} in {0}")]
    NoTracks(PathBuf),
    #[error("could not read {path}")]
    Audio {
        path: PathBuf,
        source: quill_audio::AudioError,
    },
    #[error(transparent)]
    Stt(#[from] SttError),
}

/// Transcribes whichever tracks exist in a recording directory and merges
/// them into one transcript labelled "Me" / "Others".
pub fn transcribe_recording(
    dir: &Path,
    transcriber: &dyn Transcriber,
    opts: &TranscribeOptions,
) -> Result<Vec<Segment>, MeetingError> {
    let mic = transcribe_track(&dir.join(MIC_FILE), transcriber, opts)?;
    let system = transcribe_track(&dir.join(SYSTEM_FILE), transcriber, opts)?;
    if mic.is_none() && system.is_none() {
        return Err(MeetingError::NoTracks(dir.to_path_buf()));
    }
    Ok(merge_tracks(
        mic.unwrap_or_default(),
        system.unwrap_or_default(),
    ))
}

/// `Ok(None)` if the file doesn't exist.
fn transcribe_track(
    path: &Path,
    transcriber: &dyn Transcriber,
    opts: &TranscribeOptions,
) -> Result<Option<Vec<Segment>>, MeetingError> {
    if !path.is_file() {
        return Ok(None);
    }
    let pcm = quill_audio::load_for_stt(path).map_err(|source| MeetingError::Audio {
        path: path.to_path_buf(),
        source,
    })?;
    tracing::info!(track = %path.display(), "transcribing");
    Ok(Some(transcriber.transcribe(&pcm, opts)?))
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
    fn same_words_far_apart_are_not_echo() {
        let system = vec![seg(0, 2000, "Thanks everyone.")];
        let mic = vec![seg(60_000, 62_000, "Thanks everyone.")];

        assert_eq!(merge_tracks(mic, system).len(), 2);
    }
}
