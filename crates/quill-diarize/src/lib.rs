//! Speaker diarization for Quill: splitting one audio track into "who spoke
//! when", so the call side of a meeting can become Speaker 1, 2, 3… instead
//! of one "Others".
//!
//! The work is done by sherpa-onnx's offline pipeline:
//!
//! 1. **Segmentation** (pyannote segmentation-3.0) slides a 10 s window over
//!    the audio and marks where up to three local speakers talk, overlaps
//!    included.
//! 2. **Voice embeddings** turn each local speaker's stretch of each window
//!    into a fingerprint vector.
//! 3. **Clustering** groups the fingerprints into speakers, either into a
//!    known number of speakers or by a distance threshold.
//!
//! Input is 16 kHz mono `f32` PCM, the same as transcription.

pub mod cluster;
pub mod metrics;
pub mod rttm;
pub mod voices;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use quill_stt::{Segment, Word};
use serde::{Deserialize, Serialize};
use sherpa_onnx::{
    FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
    OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
    SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig,
};

use crate::cluster::{ClusterOptions, Piece};
use crate::voices::Voice;

/// The sample rate both models expect.
pub const SAMPLE_RATE: u32 = 16_000;

/// Default clustering threshold for [`Clustering::Quill`]: the cosine
/// distance between two groups' average voices above which they're
/// different people, when the number of speakers isn't known. Lower finds
/// more speakers. 0.45 suits real meetings (AMI); synthetic voices need
/// about 0.3. See eval/diarization/README.md.
pub const DEFAULT_THRESHOLD: f32 = 0.45;

/// The voice model that did best in eval/diarization (with ERes2Net, at
/// half its cost).
pub const DEFAULT_VOICE_MODEL: &str = "voice-titanet-small";

/// Default for [`DiarizerConfig::min_speaker_ms`].
pub const DEFAULT_MIN_SPEAKER_MS: u64 = 4000;

/// Default for [`DiarizerConfig::min_gap_ms`]. sherpa-onnx's own, 0.5 s,
/// joined one person's turns across someone else's quick answer ("Monday,
/// right?" "Yes." "And…"), which then went to the wrong person. 0.2 s with
/// [`DEFAULT_MIN_TURN_MS`] placed 21 of 23 changes of speaker on a scripted
/// call (15 before) and didn't hurt AMI (DER 21.1% → 20.7%). See
/// eval/diarization/README.md.
pub const DEFAULT_MIN_GAP_MS: u64 = 200;

/// Default for [`DiarizerConfig::min_turn_ms`]. sherpa-onnx's own, 0.3 s,
/// dropped one-word answers ("No.").
pub const DEFAULT_MIN_TURN_MS: u64 = 100;

/// How the voice embeddings are grouped into speakers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clustering {
    /// sherpa-onnx's own: complete linkage over every (10 s window,
    /// speaker) embedding.
    Sherpa,
    /// sherpa-onnx's turns, re-clustered from embeddings of their clean
    /// (non-overlapping) speech; see [`cluster`].
    Quill,
}

/// Pieces of a turn longer than this are embedded separately, so one
/// mislabelled turn can be split between speakers.
const MAX_PIECE_MS: u64 = 6000;
/// Shorter pieces give unreliable embeddings; they take their turn's label.
const MIN_PIECE_MS: u64 = 1000;
/// The first pass's threshold under [`Clustering::Quill`]: its labels only
/// fill in turns too short to embed, so err on the side of more speakers.
const FIRST_PASS_THRESHOLD: f32 = 0.5;

/// Default segmentation window step; see [`DiarizerConfig::window_shift`].
pub const DEFAULT_WINDOW_SHIFT: f32 = 0.25;

/// Errors from loading the models or diarizing.
#[derive(Debug, thiserror::Error)]
pub enum DiarizeError {
    #[error("model file not found: {0}")]
    ModelNotFound(PathBuf),
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(PathBuf),
    #[error("sherpa-onnx could not load the diarization models (see the log for details)")]
    Load,
    #[error("the diarization models expect {expected} Hz audio, not {actual} Hz")]
    SampleRate { expected: u32, actual: i32 },
    #[error("diarization failed")]
    Process,
    #[error("could not compute a voice embedding")]
    Embedding,
}

/// What to load and how to cluster.
#[derive(Debug, Clone)]
pub struct DiarizerConfig {
    /// pyannote segmentation-3.0 ONNX model.
    pub segmentation_model: PathBuf,
    /// Speaker-embedding ONNX model (see `quill_stt::models::VOICE_MODELS`).
    pub voice_model: PathBuf,
    /// Who groups the voice embeddings into speakers.
    pub clustering: Clustering,
    /// Clustering threshold when the speaker count isn't given.
    pub threshold: f32,
    /// [`Clustering::Quill`]: a group of speech shorter than this, in total,
    /// is treated as an outlier rather than a person (unless the speaker
    /// count says otherwise).
    pub min_speaker_ms: u64,
    /// Step of the 10 s segmentation window, as a fraction of it. Each step
    /// computes a voice embedding per local speaker, so this sets the cost:
    /// 0.1 (pyannote's default) embeds every second of audio ~10 times.
    /// 0.25 was as accurate in eval/diarization at under half the time.
    pub window_shift: f32,
    /// One speaker's turns closer together than this are joined into one,
    /// across whatever was said in between.
    pub min_gap_ms: u64,
    /// Turns shorter than this are dropped.
    pub min_turn_ms: u64,
    /// CPU threads for each model.
    pub threads: usize,
}

impl DiarizerConfig {
    pub fn new(segmentation_model: PathBuf, voice_model: PathBuf) -> Self {
        Self {
            segmentation_model,
            voice_model,
            clustering: Clustering::Quill,
            threshold: DEFAULT_THRESHOLD,
            min_speaker_ms: DEFAULT_MIN_SPEAKER_MS,
            window_shift: DEFAULT_WINDOW_SHIFT,
            min_gap_ms: DEFAULT_MIN_GAP_MS,
            min_turn_ms: DEFAULT_MIN_TURN_MS,
            threads: std::thread::available_parallelism().map_or(4, |n| n.get().min(8)),
        }
    }
}

/// One stretch of speech by one speaker. Speakers are numbered from 0 in
/// order of first appearance. Turns by different speakers can overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub start_ms: u64,
    pub end_ms: u64,
    pub speaker: usize,
}

impl Turn {
    pub fn duration_ms(&self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }
}

/// A loaded diarization pipeline. Loading takes well under a second; the
/// models stay resident until it's dropped.
pub struct Diarizer {
    // `process` and `set_config` aren't documented as safe to call
    // concurrently on one object, so calls are serialised.
    inner: Mutex<OfflineSpeakerDiarization>,
    config: OfflineSpeakerDiarizationConfig,
    extractor: SpeakerEmbeddingExtractor,
    clustering: Clustering,
    threshold: f32,
    min_speaker_ms: u64,
}

impl Diarizer {
    pub fn load(config: &DiarizerConfig) -> Result<Self, DiarizeError> {
        let threads = i32::try_from(config.threads.max(1)).unwrap_or(4);
        let sherpa = OfflineSpeakerDiarizationConfig {
            segmentation: OfflineSpeakerSegmentationModelConfig {
                pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
                    model: Some(model_path(&config.segmentation_model)?),
                    window_shift_ratio: config.window_shift,
                },
                num_threads: threads,
                ..Default::default()
            },
            embedding: SpeakerEmbeddingExtractorConfig {
                model: Some(model_path(&config.voice_model)?),
                num_threads: threads,
                ..Default::default()
            },
            clustering: FastClusteringConfig {
                threshold: match config.clustering {
                    Clustering::Sherpa => config.threshold,
                    Clustering::Quill => FIRST_PASS_THRESHOLD,
                },
                ..Default::default()
            },
            min_duration_off: config.min_gap_ms as f32 / 1000.0,
            min_duration_on: config.min_turn_ms as f32 / 1000.0,
        };
        let inner = OfflineSpeakerDiarization::create(&sherpa).ok_or(DiarizeError::Load)?;
        let rate = inner.sample_rate();
        if u32::try_from(rate).ok() != Some(SAMPLE_RATE) {
            return Err(DiarizeError::SampleRate {
                expected: SAMPLE_RATE,
                actual: rate,
            });
        }
        let extractor =
            SpeakerEmbeddingExtractor::create(&sherpa.embedding).ok_or(DiarizeError::Load)?;
        Ok(Self {
            inner: Mutex::new(inner),
            config: sherpa,
            extractor,
            clustering: config.clustering,
            threshold: config.threshold,
            min_speaker_ms: config.min_speaker_ms,
        })
    }

    /// Diarizes 16 kHz mono audio. With `speakers`, clusters into exactly
    /// that many speakers; otherwise the threshold decides how many there
    /// are.
    pub fn diarize(&self, pcm: &[f32], speakers: Option<usize>) -> Result<Vec<Turn>, DiarizeError> {
        let speakers = speakers.filter(|&n| n > 0);
        match self.clustering {
            Clustering::Sherpa => self.first_pass(pcm, speakers).map(renumber),
            Clustering::Quill => Ok(self.analyze(pcm)?.speakers(&ClusterOptions {
                threshold: self.threshold,
                speakers,
                min_speaker_ms: self.min_speaker_ms,
            })),
        }
    }

    /// Runs the models (the slow part) under [`Clustering::Quill`], leaving
    /// the clustering to [`Analysis::speakers`], which is instant and can be
    /// repeated with other settings, e.g. once the user says how many people
    /// there were.
    pub fn analyze(&self, pcm: &[f32]) -> Result<Analysis, DiarizeError> {
        let turns = self.first_pass(pcm, None)?;
        let pieces = split_turns(&turns);
        let mut embedded = Vec::new();
        let mut points = Vec::new();
        for (i, p) in pieces.iter().enumerate() {
            if !p.clean || p.duration_ms() < MIN_PIECE_MS {
                continue;
            }
            let samples = sample_range(pcm, p.start_ms, p.end_ms);
            // A piece the model can't embed just takes its turn's label.
            let Ok(embedding) = self.embed(samples) else {
                continue;
            };
            embedded.push(i);
            points.push(Piece {
                duration_ms: p.duration_ms(),
                embedding: cluster::normalized(&embedding),
            });
        }
        Ok(Analysis {
            turns,
            pieces,
            embedded,
            points,
        })
    }

    /// sherpa-onnx's own diarization.
    fn first_pass(&self, pcm: &[f32], speakers: Option<usize>) -> Result<Vec<Turn>, DiarizeError> {
        let mut config = self.config.clone();
        config.clustering.num_clusters = speakers.and_then(|n| i32::try_from(n).ok()).unwrap_or(-1);
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.set_config(&config);
        let result = inner.process(pcm).ok_or(DiarizeError::Process)?;
        Ok(result
            .sort_by_start_time()
            .into_iter()
            .map(|s| Turn {
                start_ms: seconds_to_ms(s.start),
                end_ms: seconds_to_ms(s.end),
                speaker: usize::try_from(s.speaker).unwrap_or(0),
            })
            .collect())
    }

    fn embed(&self, samples: &[f32]) -> Result<Vec<f32>, DiarizeError> {
        let stream = self
            .extractor
            .create_stream()
            .ok_or(DiarizeError::Embedding)?;
        stream.accept_waveform(SAMPLE_RATE as i32, samples);
        stream.input_finished();
        if !self.extractor.is_ready(&stream) {
            return Err(DiarizeError::Embedding);
        }
        let embedding = self
            .extractor
            .compute(&stream)
            .ok_or(DiarizeError::Embedding)?;
        if embedding.iter().any(|x| !x.is_finite()) {
            return Err(DiarizeError::Embedding);
        }
        Ok(embedding)
    }
}

/// The output of the models for one recording, ready to cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Analysis {
    /// sherpa-onnx's turns and labels.
    turns: Vec<Turn>,
    pieces: Vec<TurnPiece>,
    /// Indexes into `pieces` of those with an embedding in `points`.
    embedded: Vec<usize>,
    points: Vec<Piece>,
}

impl Analysis {
    /// Clusters into speakers and returns the turns.
    pub fn speakers(&self, options: &ClusterOptions) -> Vec<Turn> {
        self.speakers_with_voices(options).0
    }

    /// Clusters into speakers and returns the turns, and each speaker's
    /// voice (indexed by [`Turn::speaker`]), for recognising them in other
    /// recordings.
    pub fn speakers_with_voices(&self, options: &ClusterOptions) -> (Vec<Turn>, Vec<Voice>) {
        let labels = cluster::cluster(&self.points, options);
        let mut pieces = self.pieces.clone();
        for (&i, &label) in self.embedded.iter().zip(&labels) {
            pieces[i].label = Some(label);
        }
        let (turns, order) = renumber_with_order(label_pieces(&self.turns, pieces));
        let voices = order
            .iter()
            .map(|&label| {
                let mine = self
                    .points
                    .iter()
                    .zip(&labels)
                    .filter(|&(_, &l)| l == label)
                    .map(|(p, _)| Voice {
                        embedding: p.embedding.clone(),
                        speech_ms: p.duration_ms,
                    })
                    .collect::<Vec<_>>();
                voices::average(&mine).unwrap_or(Voice {
                    embedding: Vec::new(),
                    speech_ms: 0,
                })
            })
            .collect();
        (turns, voices)
    }
}

/// Part of a turn: either clean (only this speaker) or overlapped.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct TurnPiece {
    start_ms: u64,
    end_ms: u64,
    turn: usize,
    clean: bool,
    label: Option<usize>,
}

impl TurnPiece {
    fn duration_ms(&self) -> u64 {
        self.end_ms - self.start_ms
    }
}

/// Splits each turn where other turns overlap it, and long clean stretches
/// into pieces of at most [`MAX_PIECE_MS`].
fn split_turns(turns: &[Turn]) -> Vec<TurnPiece> {
    let mut pieces = Vec::new();
    for (i, turn) in turns.iter().enumerate() {
        // Boundaries where overlap with another turn starts or stops.
        let mut cuts = vec![turn.start_ms, turn.end_ms];
        for (j, other) in turns.iter().enumerate() {
            if j != i && other.start_ms < turn.end_ms && other.end_ms > turn.start_ms {
                cuts.push(other.start_ms.max(turn.start_ms));
                cuts.push(other.end_ms.min(turn.end_ms));
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for w in cuts.windows(2) {
            let (start, end) = (w[0], w[1]);
            let mid = start + (end - start) / 2;
            let clean = !turns
                .iter()
                .enumerate()
                .any(|(j, o)| j != i && o.start_ms <= mid && mid < o.end_ms);
            let parts = if clean {
                (end - start).div_ceil(MAX_PIECE_MS).max(1)
            } else {
                1
            };
            let step = (end - start) / parts;
            for k in 0..parts {
                let s = start + k * step;
                let e = if k + 1 == parts { end } else { s + step };
                pieces.push(TurnPiece {
                    start_ms: s,
                    end_ms: e,
                    turn: i,
                    clean,
                    label: None,
                });
            }
        }
    }
    pieces
}

/// Fills in unlabelled pieces and joins them back into turns. A piece
/// without its own label takes its turn's majority label; a turn with no
/// labelled piece takes the label most of its first-pass speaker's speech
/// got.
fn label_pieces(turns: &[Turn], mut pieces: Vec<TurnPiece>) -> Vec<Turn> {
    let speakers = pieces
        .iter()
        .filter_map(|p| p.label)
        .max()
        .map_or(0, |m| m + 1);
    let first_pass = speaker_count(turns);
    let mut by_turn = vec![vec![0_u64; speakers]; turns.len()];
    let mut by_first_pass = vec![vec![0_u64; speakers]; first_pass];
    for p in &pieces {
        if let Some(label) = p.label {
            by_turn[p.turn][label] += p.duration_ms();
            by_first_pass[turns[p.turn].speaker][label] += p.duration_ms();
        }
    }
    let majority = |votes: &[u64]| {
        votes
            .iter()
            .enumerate()
            .filter(|&(_, &v)| v > 0)
            .max_by_key(|&(_, &v)| v)
            .map(|(i, _)| i)
    };
    for p in &mut pieces {
        if p.label.is_none() {
            p.label = majority(&by_turn[p.turn])
                .or_else(|| majority(&by_first_pass[turns[p.turn].speaker]));
        }
    }
    pieces.sort_by_key(|p| (p.start_ms, p.turn));
    let mut out: Vec<Turn> = Vec::new();
    let mut last_of_turn: Vec<Option<usize>> = vec![None; turns.len()];
    for p in pieces {
        let speaker = p.label.unwrap_or(0);
        // Rejoin consecutive pieces of the same turn that kept one speaker.
        if let Some(k) = last_of_turn[p.turn]
            && out[k].speaker == speaker
            && out[k].end_ms == p.start_ms
        {
            out[k].end_ms = p.end_ms;
            continue;
        }
        last_of_turn[p.turn] = Some(out.len());
        out.push(Turn {
            start_ms: p.start_ms,
            end_ms: p.end_ms,
            speaker,
        });
    }
    out
}

fn sample_range(pcm: &[f32], start_ms: u64, end_ms: u64) -> &[f32] {
    let index = |ms: u64| {
        usize::try_from(ms * u64::from(SAMPLE_RATE) / 1000)
            .unwrap_or(usize::MAX)
            .min(pcm.len())
    };
    &pcm[index(start_ms)..index(end_ms)]
}

fn model_path(path: &Path) -> Result<String, DiarizeError> {
    if !path.is_file() {
        return Err(DiarizeError::ModelNotFound(path.to_path_buf()));
    }
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| DiarizeError::NonUtf8Path(path.to_path_buf()))
}

fn seconds_to_ms(seconds: f32) -> u64 {
    // Saturating float-to-int cast: negative or NaN becomes 0.
    (f64::from(seconds) * 1000.0).round() as u64
}

/// Numbers speakers 0, 1, 2… in order of first appearance, so "Speaker 1"
/// is whoever talks first.
fn renumber(turns: Vec<Turn>) -> Vec<Turn> {
    renumber_with_order(turns).0
}

/// [`renumber`], also returning each new speaker's old number.
fn renumber_with_order(mut turns: Vec<Turn>) -> (Vec<Turn>, Vec<usize>) {
    turns.sort_by_key(|t| (t.start_ms, t.end_ms));
    let mut order: Vec<usize> = Vec::new();
    for turn in &mut turns {
        let new = match order.iter().position(|&s| s == turn.speaker) {
            Some(i) => i,
            None => {
                order.push(turn.speaker);
                order.len() - 1
            }
        };
        turn.speaker = new;
    }
    (turns, order)
}

/// Number of distinct speakers in `turns`.
pub fn speaker_count(turns: &[Turn]) -> usize {
    turns.iter().map(|t| t.speaker + 1).max().unwrap_or(0)
}

/// Labels transcript segments with who was speaking, using
/// `name(speaker)` for the label, and splits a segment where the speaker
/// changes.
///
/// Whisper's segments often straddle a change of speaker: a quick answer,
/// or a reply that starts as the question ends. A segment with
/// [`Segment::words`] is cut between words: each word goes to whoever
/// talks most while it's said, and a word nobody is heard saying (or only
/// punctuation) stays with the word before it. Without word times, whoever
/// talks most during the segment gets all of it. A segment no turn overlaps
/// takes the nearest turn within a second; otherwise its label is left
/// alone.
pub fn label_segments(
    segments: &[Segment],
    turns: &[Turn],
    name: impl Fn(usize) -> String,
) -> Vec<Segment> {
    const NEAREST_MS: u64 = 1000;
    let mut out = Vec::with_capacity(segments.len());
    for segment in segments {
        let near: Vec<&Turn> = turns
            .iter()
            .filter(|t| gap(t, segment) <= NEAREST_MS)
            .collect();
        let whole = most_talk(&near, segment.start_ms, segment.end_ms, None).or_else(|| {
            near.iter()
                .min_by_key(|t| gap(t, segment))
                .map(|t| t.speaker)
        });
        let Some(whole) = whole else {
            out.push(segment.clone());
            continue;
        };
        // Runs of words by one speaker.
        let mut runs: Vec<(usize, Vec<&Word>)> = Vec::new();
        let mut current = whole;
        for word in &segment.words {
            let heard = word.text.chars().any(char::is_alphanumeric);
            // A word timed to an instant still counts where it falls.
            let end_ms = word.end_ms.max(word.start_ms + 1);
            if heard && let Some(s) = most_talk(&near, word.start_ms, end_ms, Some(current)) {
                current = s;
            }
            match runs.last_mut() {
                Some((s, run)) if *s == current => run.push(word),
                _ => runs.push((current, vec![word])),
            }
        }
        if runs.len() < 2 {
            let speaker = runs.first().map_or(whole, |(s, _)| *s);
            out.push(Segment {
                speaker: Some(name(speaker)),
                ..segment.clone()
            });
            continue;
        }
        let last = runs.len() - 1;
        for (i, (speaker, run)) in runs.into_iter().enumerate() {
            let text: Vec<&str> = run.iter().map(|w| w.text.as_str()).collect();
            out.push(Segment {
                start_ms: if i == 0 {
                    segment.start_ms
                } else {
                    run[0].start_ms
                },
                end_ms: if i == last {
                    segment.end_ms
                } else {
                    run[run.len() - 1].end_ms
                },
                text: text.join(" "),
                speaker: Some(name(speaker)),
                words: run.into_iter().cloned().collect(),
            });
        }
    }
    out
}

/// Who talks most between `start_ms` and `end_ms`, if anyone does; on a
/// tie (two turns covering it all, say), `prefer` if it's one of them.
fn most_talk(turns: &[&Turn], start_ms: u64, end_ms: u64, prefer: Option<usize>) -> Option<usize> {
    let mut talk: Vec<(usize, u64)> = Vec::new();
    for turn in turns {
        let overlap = turn
            .end_ms
            .min(end_ms)
            .saturating_sub(turn.start_ms.max(start_ms));
        if overlap == 0 {
            continue;
        }
        match talk.iter_mut().find(|(s, _)| *s == turn.speaker) {
            Some((_, ms)) => *ms += overlap,
            None => talk.push((turn.speaker, overlap)),
        }
    }
    let most = talk.iter().map(|&(_, ms)| ms).max()?;
    match prefer {
        Some(p) if talk.contains(&(p, most)) => Some(p),
        _ => talk.iter().find(|&&(_, ms)| ms == most).map(|&(s, _)| s),
    }
}

/// Milliseconds between a turn and a segment that don't overlap.
fn gap(turn: &Turn, segment: &Segment) -> u64 {
    if turn.end_ms <= segment.start_ms {
        segment.start_ms - turn.end_ms
    } else {
        turn.start_ms.saturating_sub(segment.end_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(start_ms: u64, end_ms: u64, speaker: usize) -> Turn {
        Turn {
            start_ms,
            end_ms,
            speaker,
        }
    }

    fn segment(start_ms: u64, end_ms: u64) -> Segment {
        Segment {
            start_ms,
            end_ms,
            text: String::new(),
            speaker: None,
            words: Vec::new(),
        }
    }

    #[test]
    fn speakers_are_numbered_by_first_appearance() {
        let turns = renumber(vec![
            turn(5000, 6000, 3),
            turn(0, 1000, 7),
            turn(2000, 3000, 3),
        ]);
        assert_eq!(
            turns,
            vec![turn(0, 1000, 0), turn(2000, 3000, 1), turn(5000, 6000, 1)]
        );
        assert_eq!(speaker_count(&turns), 2);
    }

    #[test]
    fn turns_split_at_overlaps_and_into_bounded_pieces() {
        // Turn 0 runs 0–14 s; turn 1 overlaps it from 10 to 12 s.
        let turns = [turn(0, 14_000, 0), turn(10_000, 12_000, 1)];
        let pieces = split_turns(&turns);
        let spans: Vec<_> = pieces
            .iter()
            .map(|p| (p.turn, p.start_ms, p.end_ms, p.clean))
            .collect();
        assert_eq!(
            spans,
            [
                // 10 s of clean speech in two pieces of at most 6 s.
                (0, 0, 5000, true),
                (0, 5000, 10_000, true),
                (0, 10_000, 12_000, false),
                (0, 12_000, 14_000, true),
                (1, 10_000, 12_000, false),
            ]
        );
    }

    #[test]
    fn unlabelled_pieces_follow_their_turn_then_their_first_pass_speaker() {
        let turns = [
            turn(0, 10_000, 0),
            turn(10_000, 10_500, 0),
            turn(11_000, 20_000, 1),
        ];
        let mut pieces = split_turns(&turns);
        // Re-clustering split turn 0 between two speakers and labelled
        // turn 2; turn 1 was too short to embed.
        for p in &mut pieces {
            p.label = match (p.turn, p.start_ms) {
                (0, 0) => Some(1),
                (0, _) => Some(0),
                (2, _) => Some(1),
                _ => None,
            };
        }
        let out = label_pieces(&turns, pieces);
        assert_eq!(
            out,
            [
                turn(0, 5000, 1),
                turn(5000, 10_000, 0),
                // First-pass speaker 0's labelled speech is split evenly;
                // either label is fine, but it must get one.
                out[2],
                turn(11_000, 20_000, 1),
            ]
        );
        assert_eq!((out[2].start_ms, out[2].end_ms), (10_000, 10_500));
    }

    #[test]
    fn each_speaker_gets_the_average_of_their_voice() {
        // Speaker A at 0–4 s and 10–14 s, speaker B at 5–9 s; B's embedding
        // points the other way.
        let turns = vec![
            turn(0, 4000, 0),
            turn(5000, 9000, 1),
            turn(10_000, 14_000, 0),
        ];
        let pieces = split_turns(&turns);
        let a = cluster::normalized(&[1.0, 0.1]);
        let b = cluster::normalized(&[0.0, 1.0]);
        let analysis = Analysis {
            embedded: (0..pieces.len()).collect(),
            points: vec![
                Piece {
                    duration_ms: 4000,
                    embedding: a.clone(),
                },
                Piece {
                    duration_ms: 4000,
                    embedding: b.clone(),
                },
                Piece {
                    duration_ms: 4000,
                    embedding: a.clone(),
                },
            ],
            pieces,
            turns,
        };
        let (turns, voices) = analysis.speakers_with_voices(&ClusterOptions {
            threshold: 0.5,
            speakers: None,
            min_speaker_ms: 1000,
        });
        assert_eq!(speaker_count(&turns), 2);
        assert_eq!(voices.len(), 2);
        // Speaker 0 talks first: that's A, with 8 s of speech.
        assert_eq!(voices[0].speech_ms, 8000);
        assert!(voices[0].similarity(&a) > 0.99);
        assert!(voices[1].similarity(&b) > 0.99);
    }

    #[test]
    fn segments_take_the_majority_speaker() {
        let turns = [turn(0, 4000, 0), turn(4000, 5000, 1), turn(9000, 12_000, 1)];
        let segments = [
            segment(500, 4600),
            segment(3500, 5000),
            segment(7000, 8500),
            segment(20_000, 21_000),
        ];
        let segments = label_segments(&segments, &turns, |s| format!("Speaker {}", s + 1));
        let labels: Vec<_> = segments.iter().map(|s| s.speaker.as_deref()).collect();
        assert_eq!(
            labels,
            [
                Some("Speaker 1"),
                Some("Speaker 2"),
                // No overlap; the turn starting 500 ms later is close enough.
                Some("Speaker 2"),
                None,
            ]
        );
    }

    fn timed(words: &[(&str, u64, u64)]) -> Segment {
        let words: Vec<Word> = words
            .iter()
            .map(|&(text, start_ms, end_ms)| Word {
                text: text.into(),
                start_ms,
                end_ms,
            })
            .collect();
        let text: Vec<&str> = words.iter().map(|w| w.text.as_str()).collect();
        Segment {
            start_ms: words[0].start_ms,
            end_ms: words[words.len() - 1].end_ms,
            text: text.join(" "),
            speaker: None,
            words,
        }
    }

    fn lines(segments: &[Segment]) -> Vec<(u64, u64, &str, &str)> {
        segments
            .iter()
            .map(|s| {
                let speaker = s.speaker.as_deref().unwrap_or("");
                (s.start_ms, s.end_ms, speaker, s.text.as_str())
            })
            .collect()
    }

    #[test]
    fn segments_split_where_the_speaker_changes() {
        // A question, then the answer with no pause, in one segment.
        let segment = timed(&[
            ("Monday,", 0, 400),
            ("right?", 400, 800),
            ("Yes.", 900, 1200),
            ("And", 1300, 1500),
            ("you?", 1500, 1800),
        ]);
        let turns = [turn(0, 850, 0), turn(850, 1250, 1), turn(1250, 1900, 0)];
        let out = label_segments(&[segment], &turns, |s| format!("S{s}"));
        assert_eq!(
            lines(&out),
            [
                (0, 800, "S0", "Monday, right?"),
                (900, 1200, "S1", "Yes."),
                (1300, 1800, "S0", "And you?"),
            ]
        );
        assert_eq!(out[1].words.len(), 1);
    }

    #[test]
    fn a_word_goes_to_whoever_says_most_of_it() {
        let segment = timed(&[("so", 0, 300), ("would", 300, 600), ("you", 600, 1000)]);
        // The change comes 100 ms into "you".
        let turns = [turn(0, 700, 0), turn(700, 1000, 1)];
        let out = label_segments(&[segment], &turns, |s| format!("S{s}"));
        assert_eq!(
            lines(&out),
            [(0, 600, "S0", "so would"), (600, 1000, "S1", "you")]
        );
    }

    #[test]
    fn unheard_words_stay_with_the_speaker_before() {
        // "-" is only punctuation; "um" falls in a gap between turns; and
        // two turns covering "both" equally don't change who's speaking.
        let segment = timed(&[
            ("We", 0, 300),
            ("-", 300, 600),
            ("um", 1000, 1200),
            ("both", 1500, 1800),
            ("agree.", 1800, 2100),
        ]);
        let turns = [turn(0, 900, 0), turn(1300, 2100, 0), turn(1400, 1900, 1)];
        let out = label_segments(&[segment], &turns, |s| format!("S{s}"));
        assert_eq!(lines(&out), [(0, 2100, "S0", "We - um both agree.")]);
    }
}
