//! Recognising people across recordings: each speaker's voice as one
//! embedding, and matching a recording's speakers to voices heard before.
//!
//! A speaker's voice is the duration-weighted average of the embeddings of
//! their clean pieces of speech (the centroid [`crate::cluster`] works with).
//! Averaging many pieces makes it far steadier than any one piece, so two
//! recordings of the same person come out close even when single pieces
//! don't.
//!
//! Matching is deliberately cautious: a wrong name is worse than "Speaker 2".
//! A speaker is matched only with enough speech behind their voice, only
//! above a similarity threshold, and each known voice goes to at most one
//! speaker. The thresholds were tuned on AMI meetings; see
//! eval/diarization/README.md.

use serde::{Deserialize, Serialize};

use crate::cluster;

/// The similarity (cosine, −1 to 1) a speaker's voice must reach to be
/// given a known voice's name. Chosen with `quill-cli voices eval` on 28 AMI
/// meetings (28 people): from 0.65 up, no stranger was named, and recall
/// barely changes between 0.45 and 0.75. 0.7 leaves a margin for voices
/// heard over different microphones and codecs. See
/// eval/diarization/README.md.
pub const DEFAULT_MIN_SIMILARITY: f32 = 0.7;

/// The least clean speech, in total, behind a voice for it to be matched or
/// remembered. Below it, small mixed clusters (overlap, crosstalk) matched
/// each other across meetings in the eval; a voice learned from one became
/// a "person" that showed up everywhere.
pub const DEFAULT_MIN_SPEECH_MS: u64 = 30_000;

/// One speaker's voice.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Voice {
    /// L2-normalised average embedding; empty if nothing was embedded.
    pub embedding: Vec<f32>,
    /// How much clean speech it was averaged from.
    pub speech_ms: u64,
}

impl Voice {
    /// Cosine similarity with another voice's embedding (both normalised).
    /// 0 if either is empty or they come from models of different sizes.
    pub fn similarity(&self, other: &[f32]) -> f32 {
        similarity(&self.embedding, other)
    }
}

/// Cosine similarity of two L2-normalised embeddings; 0 when they can't be
/// compared.
pub fn similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    1.0 - cluster::cosine_distance(a, b)
}

/// The speech-weighted average of several voices of one person, e.g. two
/// speakers the user merged, or one person's voice from several meetings.
/// `None` if there's nothing to average.
pub fn average<'a>(voices: impl IntoIterator<Item = &'a Voice>) -> Option<Voice> {
    let mut sum: Vec<f32> = Vec::new();
    let mut speech_ms = 0;
    for v in voices {
        if v.embedding.is_empty() || (!sum.is_empty() && sum.len() != v.embedding.len()) {
            continue;
        }
        if sum.is_empty() {
            sum = vec![0.0; v.embedding.len()];
        }
        // Weighted by speech, but never so little that a voice counts for
        // nothing.
        let weight = v.speech_ms.max(1) as f32;
        for (s, x) in sum.iter_mut().zip(&v.embedding) {
            *s += x * weight;
        }
        speech_ms += v.speech_ms;
    }
    (!sum.is_empty()).then(|| Voice {
        embedding: cluster::normalized(&sum),
        speech_ms,
    })
}

/// How cautious [`match_voices`] is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchOptions {
    /// Least similarity for a match.
    pub min_similarity: f32,
    /// Least speech behind a speaker's voice for it to be matched.
    pub min_speech_ms: u64,
}

impl Default for MatchOptions {
    fn default() -> Self {
        Self {
            min_similarity: DEFAULT_MIN_SIMILARITY,
            min_speech_ms: DEFAULT_MIN_SPEECH_MS,
        }
    }
}

/// A speaker recognised as a known voice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Match {
    /// Index into the known voices.
    pub known: usize,
    pub similarity: f32,
}

/// Matches each speaker to at most one known voice, and each known voice to
/// at most one speaker: the most similar pair first, then the next, as long
/// as they clear `options`. Returns one entry per speaker.
pub fn match_voices(
    speakers: &[Voice],
    known: &[Vec<f32>],
    options: &MatchOptions,
) -> Vec<Option<Match>> {
    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for (s, voice) in speakers.iter().enumerate() {
        if voice.speech_ms < options.min_speech_ms {
            continue;
        }
        for (k, embedding) in known.iter().enumerate() {
            let sim = voice.similarity(embedding);
            if sim >= options.min_similarity {
                pairs.push((sim, s, k));
            }
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out = vec![None; speakers.len()];
    let mut taken = vec![false; known.len()];
    for (sim, s, k) in pairs {
        if out[s].is_none() && !taken[k] {
            out[s] = Some(Match {
                known: k,
                similarity: sim,
            });
            taken[k] = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: &[f32]) -> Vec<f32> {
        cluster::normalized(v)
    }

    fn voice(v: &[f32], speech_ms: u64) -> Voice {
        Voice {
            embedding: unit(v),
            speech_ms,
        }
    }

    #[test]
    fn average_weights_by_speech() {
        let a = voice(&[1.0, 0.0], 30_000);
        let b = voice(&[0.0, 1.0], 10_000);
        let avg = average([&a, &b]).unwrap();
        assert_eq!(avg.speech_ms, 40_000);
        // Three times as much of `a`: closer to it.
        assert!(avg.similarity(&a.embedding) > avg.similarity(&b.embedding));
        assert!((avg.embedding.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-5);
        assert!(average([]).is_none());
        // Mismatched sizes (another model) are skipped, not mixed in.
        let other = voice(&[1.0, 0.0, 0.0], 5000);
        assert_eq!(average([&a, &other]).unwrap().embedding.len(), 2);
    }

    #[test]
    fn each_known_voice_goes_to_the_closest_speaker_only() {
        let known = vec![unit(&[1.0, 0.0, 0.0]), unit(&[0.0, 1.0, 0.0])];
        let speakers = [
            // Close to known 0.
            voice(&[0.9, 0.1, 0.0], 60_000),
            // Even closer to known 0: takes it.
            voice(&[1.0, 0.02, 0.0], 60_000),
            // Nobody known.
            voice(&[0.0, 0.0, 1.0], 60_000),
            // Close to known 1, but too little speech to trust.
            voice(&[0.0, 1.0, 0.0], 3000),
        ];
        let matches = match_voices(&speakers, &known, &MatchOptions::default());
        assert_eq!(matches[0], None);
        assert_eq!(matches[1].map(|m| m.known), Some(0));
        assert_eq!(matches[2], None);
        assert_eq!(matches[3], None);
    }

    #[test]
    fn below_the_threshold_stays_unnamed() {
        let known = vec![unit(&[1.0, 0.0])];
        // Cosine 0.5 with known 0.
        let speakers = [voice(&[0.5, 0.866], 60_000)];
        let strict = MatchOptions {
            min_similarity: 0.6,
            min_speech_ms: 0,
        };
        assert_eq!(match_voices(&speakers, &known, &strict), [None]);
        let lax = MatchOptions {
            min_similarity: 0.4,
            ..strict
        };
        assert_eq!(match_voices(&speakers, &known, &lax)[0].unwrap().known, 0);
    }
}
