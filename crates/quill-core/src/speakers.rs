//! Telling the people on the call apart: turning the "Others" lines of a
//! two-track transcript into "Speaker 1", "Speaker 2"… by voice.
//!
//! The slow part, running the diarization models over the system track, is
//! done once and cached next to the audio as [`ANALYSIS_FILE`]. Splitting
//! the speakers again (say, once the user says how many people were on the
//! call) then takes milliseconds. Re-splitting starts from the two-track
//! transcript saved as [`TRANSCRIPT_FILE`], so names given since are
//! replaced.
//!
//! People the user has named are remembered by voice ("known voices", see
//! `crate::voices`), and [`label_and_recognize`] gives their names to
//! matching speakers in later meetings.

use std::path::Path;

use quill_diarize::cluster::ClusterOptions;
pub use quill_diarize::voices::{MatchOptions, Voice};
use quill_diarize::{Analysis, Diarizer, DiarizerConfig};
use quill_stt::Segment;
use quill_stt::models::{self, ModelInfo};
use serde::{Deserialize, Serialize};

use crate::meeting::{self, MeetingError, OTHERS};

/// The transcript as the two tracks gave it ("Me" / "Others").
pub const TRANSCRIPT_FILE: &str = "transcript.json";
/// The cached diarization analysis of the system track.
pub const ANALYSIS_FILE: &str = "speakers.json";
/// Bumped when the cached analysis format or method changes.
const ANALYSIS_VERSION: u32 = 1;

/// The voice model used for telling speakers apart.
pub const VOICE_MODEL: &str = quill_diarize::DEFAULT_VOICE_MODEL;

/// The models [`diarizer`] needs; download them first.
pub fn required_models() -> Vec<&'static ModelInfo> {
    let mut list = vec![&models::SEGMENTATION_MODEL];
    list.extend(models::find(VOICE_MODEL));
    list
}

/// Loads the diarization models from `models_dir` (well under a second).
pub fn diarizer(models_dir: &Path) -> Result<Diarizer, MeetingError> {
    let voice = models::find(VOICE_MODEL).ok_or_else(|| {
        MeetingError::Diarize(quill_diarize::DiarizeError::ModelNotFound(
            VOICE_MODEL.into(),
        ))
    })?;
    let config = DiarizerConfig::new(
        models::SEGMENTATION_MODEL.path_in(models_dir),
        voice.path_in(models_dir),
    );
    Ok(Diarizer::load(&config)?)
}

/// Label for the `i`th speaker on the call (0-based).
pub fn speaker_label(i: usize) -> String {
    format!("Speaker {}", i + 1)
}

#[derive(Serialize, Deserialize)]
struct Cached {
    version: u32,
    /// Embeddings from different voice models aren't comparable.
    voice_model: String,
    analysis: Analysis,
}

/// Saves the two-track transcript that [`label`] starts from.
pub fn save_track_transcript(dir: &Path, segments: &[Segment]) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(segments).map_err(std::io::Error::other)?;
    std::fs::write(dir.join(TRANSCRIPT_FILE), json)
}

/// The saved two-track transcript, if there is one.
pub fn load_track_transcript(dir: &Path) -> Option<Vec<Segment>> {
    let json = std::fs::read_to_string(dir.join(TRANSCRIPT_FILE)).ok()?;
    serde_json::from_str(&json).ok()
}

/// The cached analysis, if it was made with `voice_model`.
pub fn cached_analysis(dir: &Path, voice_model: &str) -> Option<Analysis> {
    let json = std::fs::read_to_string(dir.join(ANALYSIS_FILE)).ok()?;
    let cached: Cached = serde_json::from_str(&json).ok()?;
    (cached.version == ANALYSIS_VERSION && cached.voice_model == voice_model)
        .then_some(cached.analysis)
}

/// Analyses the recording's system track with `diarizer` and caches the
/// result. `None` if there's no system track. Slow: about 1/35 of the
/// audio's length.
pub fn analyze(
    dir: &Path,
    voice_model: &str,
    diarizer: &Diarizer,
) -> Result<Option<Analysis>, MeetingError> {
    let Some(pcm) = meeting::load_system_track(dir)? else {
        return Ok(None);
    };
    let analysis = diarizer.analyze(&pcm)?;
    let cached = Cached {
        version: ANALYSIS_VERSION,
        voice_model: voice_model.to_string(),
        analysis,
    };
    // The cache only saves time later; failing to write it isn't an error.
    match serde_json::to_string(&cached) {
        Ok(json) => {
            if let Err(e) = std::fs::write(dir.join(ANALYSIS_FILE), json) {
                tracing::warn!("couldn't cache the speaker analysis: {e}");
            }
        }
        Err(e) => tracing::warn!("couldn't serialize the speaker analysis: {e}"),
    }
    Ok(Some(cached.analysis))
}

/// Relabels the "Others" segments as "Speaker 1", "Speaker 2"…, numbered by
/// who speaks first. `speakers` is how many people were on the call, if
/// known. Returns the labelled transcript and how many speakers it has.
pub fn label(
    segments: &[Segment],
    analysis: &Analysis,
    speakers: Option<usize>,
) -> (Vec<Segment>, usize) {
    let labelled = label_and_recognize(segments, analysis, speakers, &[], &MatchOptions::default());
    (labelled.segments, labelled.speakers)
}

/// A person the user named, remembered by voice to recognise in other
/// meetings (see `Library::known_voices_to_match`).
#[derive(Debug, Clone, PartialEq)]
pub struct KnownVoice {
    pub id: i64,
    pub name: String,
    pub voice: Voice,
}

/// One person on the call, as labelled in the transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct CallVoice {
    /// Their label in the transcript: "Speaker 2", or a known voice's name.
    pub label: String,
    pub voice: Voice,
    /// Set when the label is a known voice's name, given automatically.
    pub recognized: Option<Recognized>,
}

/// A speaker given a known voice's name automatically.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Recognized {
    /// [`KnownVoice::id`].
    pub voice_id: i64,
    /// Cosine similarity of the two voices.
    pub similarity: f32,
}

/// A transcript with the people on the call told apart.
#[derive(Debug, Clone, PartialEq)]
pub struct Labelled {
    pub segments: Vec<Segment>,
    /// How many people on the call (not counting "Me").
    pub speakers: usize,
    /// One per label given, in order of first appearance.
    pub voices: Vec<CallVoice>,
}

impl Labelled {
    /// A transcript left as it was: nobody told apart.
    pub fn unchanged(segments: &[Segment]) -> Self {
        Self {
            segments: segments.to_vec(),
            speakers: 0,
            voices: Vec::new(),
        }
    }

    /// How many people were recognised by voice.
    pub fn recognized(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.recognized.is_some())
            .count()
    }
}

/// Like [`label`], and gives anyone whose voice matches one of `known` that
/// person's name (see [`quill_diarize::voices::match_voices`]). Speakers
/// who aren't recognised are numbered "Speaker 1", "Speaker 2"… among
/// themselves, by who speaks first.
pub fn label_and_recognize(
    segments: &[Segment],
    analysis: &Analysis,
    speakers: Option<usize>,
    known: &[KnownVoice],
    options: &MatchOptions,
) -> Labelled {
    let (turns, voices) = analysis.speakers_with_voices(&ClusterOptions::new(speakers));
    let is_others = |s: &Segment| s.speaker.as_deref() == Some(OTHERS);
    let mut theirs: Vec<Segment> = segments.iter().filter(|s| is_others(s)).cloned().collect();
    // Label with the raw speaker index first; named below.
    quill_diarize::label_segments(&mut theirs, &turns, |i| i.to_string());

    // Speakers in order of first appearance in the transcript, so a voice
    // with no transcribed words doesn't leave a gap ("Speaker 1, 2, 4").
    let raw = |s: &Segment| s.speaker.as_deref().and_then(|l| l.parse::<usize>().ok());
    let mut order: Vec<usize> = Vec::new();
    for s in &theirs {
        if let Some(i) = raw(s)
            && !order.contains(&i)
        {
            order.push(i);
        }
    }
    let heard: Vec<Voice> = order
        .iter()
        .map(|&i| voices.get(i).cloned().unwrap_or_default())
        .collect();
    let known_embeddings: Vec<Vec<f32>> = known.iter().map(|k| k.voice.embedding.clone()).collect();
    let matches = quill_diarize::voices::match_voices(&heard, &known_embeddings, options);
    let mut unknown = 0;
    let call: Vec<CallVoice> = heard
        .into_iter()
        .zip(matches)
        .map(|(voice, m)| match m {
            Some(m) => CallVoice {
                label: known[m.known].name.clone(),
                voice,
                recognized: Some(Recognized {
                    voice_id: known[m.known].id,
                    similarity: m.similarity,
                }),
            },
            None => {
                unknown += 1;
                CallVoice {
                    label: speaker_label(unknown - 1),
                    voice,
                    recognized: None,
                }
            }
        })
        .collect();
    for s in &mut theirs {
        if let Some(i) = raw(s)
            && let Some(pos) = order.iter().position(|&o| o == i)
        {
            s.speaker = Some(call[pos].label.clone());
        }
    }
    let mut theirs = theirs.into_iter();
    let out = segments
        .iter()
        .map(|s| {
            if is_others(s) {
                theirs.next().unwrap_or_else(|| s.clone())
            } else {
                s.clone()
            }
        })
        .collect();
    Labelled {
        segments: out,
        speakers: order.len(),
        voices: call,
    }
}

/// Whether a speaker's name can be remembered as a known voice: not the
/// mic track ("Me", "You"), not "Others", and not an automatic "Speaker N"
/// label.
pub fn rememberable(name: &str) -> bool {
    let name = name.trim();
    let numbered = name
        .strip_prefix("Speaker ")
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    !name.is_empty()
        && !numbered
        && [meeting::ME, OTHERS, "You"]
            .iter()
            .all(|reserved| !name.eq_ignore_ascii_case(reserved))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start_ms: u64, end_ms: u64, speaker: &str) -> Segment {
        Segment {
            start_ms,
            end_ms,
            text: "words".into(),
            speaker: Some(speaker.into()),
        }
    }

    #[test]
    fn track_transcript_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_track_transcript(dir.path()).is_none());
        let segments = vec![seg(0, 1000, "Me"), seg(1000, 2000, OTHERS)];
        save_track_transcript(dir.path(), &segments).unwrap();
        assert_eq!(load_track_transcript(dir.path()).unwrap(), segments);
    }

    /// Two voices alternating: turns 0 and 2 are one person, turn 1 another.
    fn analysis() -> Analysis {
        serde_json::from_value(serde_json::json!({
            "turns": [
                {"start_ms": 0, "end_ms": 5000, "speaker": 0},
                {"start_ms": 6000, "end_ms": 10000, "speaker": 1},
                {"start_ms": 11000, "end_ms": 15000, "speaker": 0},
            ],
            "pieces": [
                {"start_ms": 0, "end_ms": 5000, "turn": 0, "clean": true, "label": null},
                {"start_ms": 6000, "end_ms": 10000, "turn": 1, "clean": true, "label": null},
                {"start_ms": 11000, "end_ms": 15000, "turn": 2, "clean": true, "label": null},
            ],
            "embedded": [0, 1, 2],
            "points": [
                {"duration_ms": 5000, "embedding": [1.0, 0.0]},
                {"duration_ms": 4000, "embedding": [0.0, 1.0]},
                {"duration_ms": 4000, "embedding": [1.0, 0.0]},
            ],
        }))
        .unwrap()
    }

    #[test]
    fn others_become_numbered_speakers() {
        let segments = [
            seg(0, 5000, OTHERS),
            seg(5000, 6000, "Me"),
            seg(6000, 10_000, OTHERS),
            seg(11_000, 15_000, OTHERS),
        ];
        let (labelled, count) = label(&segments, &analysis(), None);
        let names: Vec<_> = labelled
            .iter()
            .map(|s| s.speaker.as_deref().unwrap())
            .collect();
        assert_eq!(names, ["Speaker 1", "Me", "Speaker 2", "Speaker 1"]);
        assert_eq!(count, 2);

        // Told there was one other person, everyone is Speaker 1.
        let (labelled, count) = label(&segments, &analysis(), Some(1));
        assert_eq!(count, 1);
        assert!(
            labelled
                .iter()
                .all(|s| s.speaker.as_deref() != Some("Speaker 2"))
        );
    }

    #[test]
    fn known_voices_get_their_names_and_the_rest_are_numbered() {
        let segments = [
            seg(0, 5000, OTHERS),
            seg(6000, 10_000, OTHERS),
            seg(11_000, 15_000, OTHERS),
        ];
        let priya = KnownVoice {
            id: 7,
            name: "Priya".into(),
            voice: Voice {
                embedding: vec![0.0, 1.0],
                speech_ms: 60_000,
            },
        };
        let options = MatchOptions {
            min_similarity: 0.6,
            min_speech_ms: 1000,
        };
        let labelled = label_and_recognize(&segments, &analysis(), None, &[priya], &options);
        let names: Vec<_> = labelled
            .segments
            .iter()
            .map(|s| s.speaker.as_deref().unwrap())
            .collect();
        // The second voice is Priya's; the first is still "Speaker 1".
        assert_eq!(names, ["Speaker 1", "Priya", "Speaker 1"]);
        assert_eq!(labelled.speakers, 2);
        assert_eq!(labelled.recognized(), 1);
        assert_eq!(labelled.voices[0].label, "Speaker 1");
        assert_eq!(labelled.voices[0].voice.speech_ms, 9000);
        let recognized = labelled.voices[1].recognized.unwrap();
        assert_eq!(recognized.voice_id, 7);
        assert!(recognized.similarity > 0.99);

        // Not enough speech behind the voice: nobody is named.
        let cautious = MatchOptions {
            min_speech_ms: 10_000,
            ..options
        };
        let priya = KnownVoice {
            id: 7,
            name: "Priya".into(),
            voice: Voice {
                embedding: vec![0.0, 1.0],
                speech_ms: 60_000,
            },
        };
        let labelled = label_and_recognize(&segments, &analysis(), None, &[priya], &cautious);
        assert_eq!(labelled.recognized(), 0);
        assert_eq!(labelled.voices[1].label, "Speaker 2");
    }

    #[test]
    fn only_real_names_are_remembered() {
        for name in ["Priya", "Dan Smith", "Speaker", "Speaker one"] {
            assert!(rememberable(name), "{name}");
        }
        for name in [
            "",
            "  ",
            "Me",
            "me",
            "You",
            "Others",
            "Speaker 3",
            "Speaker 12",
        ] {
            assert!(!rememberable(name), "{name}");
        }
    }

    #[test]
    fn stale_or_missing_cache_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        assert!(cached_analysis(dir.path(), "voice-a").is_none());
        std::fs::write(dir.path().join(ANALYSIS_FILE), "{\"version\":0}").unwrap();
        assert!(cached_analysis(dir.path(), "voice-a").is_none());
    }
}
