//! Telling the people on the call apart: turning the "Others" lines of a
//! two-track transcript into "Speaker 1", "Speaker 2"… by voice.
//!
//! The slow part, running the diarization models over the system track, is
//! done once and cached next to the audio as [`ANALYSIS_FILE`]. Splitting
//! the speakers again (say, once the user says how many people were on the
//! call) then takes milliseconds. Re-splitting starts from the two-track
//! transcript saved as [`TRANSCRIPT_FILE`], so names given since are
//! replaced.

use std::collections::HashMap;
use std::path::Path;

use quill_diarize::cluster::ClusterOptions;
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
    let turns = analysis.speakers(&ClusterOptions::new(speakers));
    let is_others = |s: &Segment| s.speaker.as_deref() == Some(OTHERS);
    let mut theirs: Vec<Segment> = segments.iter().filter(|s| is_others(s)).cloned().collect();
    // Label with the raw speaker index first; renumbered below.
    quill_diarize::label_segments(&mut theirs, &turns, |i| i.to_string());

    // Number speakers by first appearance in the transcript, so a voice
    // with no transcribed words doesn't leave a gap ("Speaker 1, 2, 4").
    let mut numbers: HashMap<String, usize> = HashMap::new();
    for s in &mut theirs {
        if let Some(raw) = s.speaker.as_deref().filter(|&l| l != OTHERS) {
            let next = numbers.len();
            let n = *numbers.entry(raw.to_string()).or_insert(next);
            s.speaker = Some(speaker_label(n));
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
    (out, numbers.len())
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
    fn stale_or_missing_cache_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        assert!(cached_analysis(dir.path(), "voice-a").is_none());
        std::fs::write(dir.path().join(ANALYSIS_FILE), "{\"version\":0}").unwrap();
        assert!(cached_analysis(dir.path(), "voice-a").is_none());
    }
}
