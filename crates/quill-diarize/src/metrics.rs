//! Scoring a diarization against a reference: diarization error rate (DER)
//! and how each real speaker fared.
//!
//! DER is the share of reference speech that's missed, attributed to someone
//! who isn't talking (false alarm), or attributed to the wrong person
//! (confusion). Hypothesis speakers are matched one-to-one to reference
//! speakers so that the most speech lines up, as standard scoring tools do.
//! Overlapping speech counts once per speaker.

use std::collections::HashMap;

use crate::Turn;
use crate::rttm::NamedTurn;

/// Scoring resolution.
const FRAME_MS: u64 = 10;
/// Speakers beyond this many on either side are ignored (bit masks).
const MAX_SPEAKERS: usize = 64;

/// The result of [`score`].
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Reference speech scored, counting overlaps once per speaker.
    pub scored_ms: u64,
    pub missed_ms: u64,
    pub false_alarm_ms: u64,
    pub confusion_ms: u64,
    pub reference_speakers: usize,
    pub hypothesis_speakers: usize,
    /// Each reference speaker, in order of first appearance.
    pub speakers: Vec<SpeakerScore>,
}

impl Report {
    /// Diarization error rate, 0.0–1.0 (can exceed 1 with many false alarms).
    pub fn der(&self) -> f64 {
        ratio(
            self.missed_ms + self.false_alarm_ms + self.confusion_ms,
            self.scored_ms,
        )
    }

    pub fn missed(&self) -> f64 {
        ratio(self.missed_ms, self.scored_ms)
    }

    pub fn false_alarm(&self) -> f64 {
        ratio(self.false_alarm_ms, self.scored_ms)
    }

    pub fn confusion(&self) -> f64 {
        ratio(self.confusion_ms, self.scored_ms)
    }
}

/// How one reference speaker was diarized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeakerScore {
    pub name: String,
    /// How long they talk (scored frames only).
    pub talk_ms: u64,
    /// The hypothesis speaker matched to them, if any.
    pub matched: Option<usize>,
    /// How much of their talk the matched speaker covers.
    pub matched_ms: u64,
    /// The hypothesis speaker covering most of their talk, which differs
    /// from `matched` when they were merged into someone else.
    pub mostly: Option<usize>,
    pub mostly_ms: u64,
}

impl SpeakerScore {
    /// Share of their talk attributed to their matched speaker.
    pub fn coverage(&self) -> f64 {
        ratio(self.matched_ms, self.talk_ms)
    }
}

fn ratio(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

/// Scores `hypothesis` against `reference`. Within `collar_ms` of a
/// reference turn's start or end nothing is scored, forgiving small boundary
/// differences (0 is the strictest; 250 ms is common).
pub fn score(reference: &[NamedTurn], hypothesis: &[Turn], collar_ms: u64) -> Report {
    // Reference speakers by first appearance.
    let mut sorted: Vec<&NamedTurn> = reference.iter().collect();
    sorted.sort_by_key(|t| (t.start_ms, t.end_ms));
    let mut names: Vec<String> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for t in &sorted {
        if !index.contains_key(t.speaker.as_str()) {
            index.insert(&t.speaker, names.len());
            names.push(t.speaker.clone());
        }
    }
    let n_ref = names.len().min(MAX_SPEAKERS);
    let n_hyp = crate::speaker_count(hypothesis).min(MAX_SPEAKERS);

    let end = reference
        .iter()
        .map(|t| t.end_ms)
        .chain(hypothesis.iter().map(|t| t.end_ms))
        .max()
        .unwrap_or(0);
    let frames = usize::try_from(end.div_ceil(FRAME_MS)).unwrap_or(0);
    let frame_range = |start_ms: u64, end_ms: u64| {
        let to_frame = |ms: u64| usize::try_from(ms / FRAME_MS).unwrap_or(frames).min(frames);
        to_frame(start_ms)..to_frame(end_ms)
    };

    let mut ref_mask = vec![0_u64; frames];
    let mut hyp_mask = vec![0_u64; frames];
    let mut scored = vec![true; frames];
    for t in reference {
        let speaker = index[t.speaker.as_str()];
        if speaker < MAX_SPEAKERS {
            for f in frame_range(t.start_ms, t.end_ms) {
                ref_mask[f] |= 1 << speaker;
            }
        }
        if collar_ms > 0 {
            for boundary in [t.start_ms, t.end_ms] {
                for f in frame_range(boundary.saturating_sub(collar_ms), boundary + collar_ms) {
                    scored[f] = false;
                }
            }
        }
    }
    for t in hypothesis {
        if t.speaker < MAX_SPEAKERS {
            for f in frame_range(t.start_ms, t.end_ms) {
                hyp_mask[f] |= 1 << t.speaker;
            }
        }
    }

    // Frames each reference speaker shares with each hypothesis speaker.
    let mut overlap = vec![vec![0_u64; n_hyp]; n_ref];
    let mut talk = vec![0_u64; n_ref];
    for f in (0..frames).filter(|&f| scored[f]) {
        for (r, row) in overlap.iter_mut().enumerate() {
            if ref_mask[f] & (1 << r) == 0 {
                continue;
            }
            talk[r] += 1;
            for (h, cell) in row.iter_mut().enumerate() {
                if hyp_mask[f] & (1 << h) != 0 {
                    *cell += 1;
                }
            }
        }
    }
    let mapping = best_mapping(&overlap);

    let mut report = Report {
        scored_ms: 0,
        missed_ms: 0,
        false_alarm_ms: 0,
        confusion_ms: 0,
        reference_speakers: names.len(),
        hypothesis_speakers: crate::speaker_count(hypothesis),
        speakers: Vec::new(),
    };
    for f in (0..frames).filter(|&f| scored[f]) {
        let (r, h) = (ref_mask[f], hyp_mask[f]);
        let n_r = u64::from(r.count_ones());
        let n_h = u64::from(h.count_ones());
        let correct = (0..n_ref)
            .filter(|&s| r & (1 << s) != 0)
            .filter(|&s| mapping[s].is_some_and(|m| h & (1 << m) != 0))
            .count() as u64;
        report.scored_ms += n_r * FRAME_MS;
        report.missed_ms += n_r.saturating_sub(n_h) * FRAME_MS;
        report.false_alarm_ms += n_h.saturating_sub(n_r) * FRAME_MS;
        report.confusion_ms += (n_r.min(n_h) - correct) * FRAME_MS;
    }
    report.speakers = (0..n_ref)
        .map(|r| {
            let mostly = (0..n_hyp)
                .filter(|&h| overlap[r][h] > 0)
                .max_by_key(|&h| overlap[r][h]);
            SpeakerScore {
                name: names[r].clone(),
                talk_ms: talk[r] * FRAME_MS,
                matched: mapping[r],
                matched_ms: mapping[r].map_or(0, |h| overlap[r][h] * FRAME_MS),
                mostly,
                mostly_ms: mostly.map_or(0, |h| overlap[r][h] * FRAME_MS),
            }
        })
        .collect();
    report
}

/// One-to-one assignment of rows to columns maximising the total weight
/// (Hungarian algorithm), as `row -> Some(column)`. Rows left without a
/// column (more rows than columns), or matched only through zero weight,
/// get `None`.
fn best_mapping(weights: &[Vec<u64>]) -> Vec<Option<usize>> {
    let rows = weights.len();
    let cols = weights.first().map_or(0, Vec::len);
    let n = rows.max(cols);
    if n == 0 {
        return vec![None; rows];
    }
    // Minimise cost = -weight on an n×n matrix padded with zeros.
    let cost = |i: usize, j: usize| -> i64 {
        if i < rows && j < cols {
            -i64::try_from(weights[i][j]).unwrap_or(i64::MAX)
        } else {
            0
        }
    };
    // Potentials-based O(n³) Hungarian algorithm, 1-indexed internally.
    let mut u = vec![0_i64; n + 1];
    let mut v = vec![0_i64; n + 1];
    let mut p = vec![0_usize; n + 1]; // column -> row
    let mut way = vec![0_usize; n + 1];
    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0;
        let mut minv = vec![i64::MAX; n + 1];
        let mut used = vec![false; n + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = i64::MAX;
            let mut j1 = 0;
            for j in 1..=n {
                if used[j] {
                    continue;
                }
                let cur = cost(i0 - 1, j - 1) - u[i0] - v[j];
                if cur < minv[j] {
                    minv[j] = cur;
                    way[j] = j0;
                }
                if minv[j] < delta {
                    delta = minv[j];
                    j1 = j;
                }
            }
            for j in 0..=n {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    let mut mapping = vec![None; rows];
    for (j, &row) in p.iter().enumerate().skip(1) {
        let (i, col) = (row - 1, j - 1);
        if i < rows && col < cols && weights[i][col] > 0 {
            mapping[i] = Some(col);
        }
    }
    mapping
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(start_ms: u64, end_ms: u64, speaker: &str) -> NamedTurn {
        NamedTurn {
            start_ms,
            end_ms,
            speaker: speaker.into(),
        }
    }

    fn turn(start_ms: u64, end_ms: u64, speaker: usize) -> Turn {
        Turn {
            start_ms,
            end_ms,
            speaker,
        }
    }

    #[test]
    fn perfect_diarization_with_different_labels_scores_zero() {
        let reference = [
            named(0, 1000, "bob"),
            named(1000, 3000, "amy"),
            named(3000, 4000, "bob"),
        ];
        let hypothesis = [turn(0, 1000, 1), turn(1000, 3000, 0), turn(3000, 4000, 1)];
        let report = score(&reference, &hypothesis, 0);
        assert_eq!(report.der(), 0.0);
        assert_eq!(report.scored_ms, 4000);
        assert_eq!(report.speakers[0].name, "bob");
        assert_eq!(report.speakers[0].matched, Some(1));
        assert_eq!(report.speakers[0].coverage(), 1.0);
    }

    #[test]
    fn merged_speakers_count_as_confusion() {
        // Two people, found as one: the shorter one is all confusion.
        let reference = [named(0, 3000, "a"), named(3000, 4000, "b")];
        let hypothesis = [turn(0, 4000, 0)];
        let report = score(&reference, &hypothesis, 0);
        assert_eq!(report.confusion_ms, 1000);
        assert_eq!((report.missed_ms, report.false_alarm_ms), (0, 0));
        assert!((report.der() - 0.25).abs() < 1e-9);
        let b = &report.speakers[1];
        assert_eq!((b.matched, b.mostly), (None, Some(0)));
    }

    #[test]
    fn missed_and_false_alarm_speech() {
        let reference = [named(0, 2000, "a")];
        let hypothesis = [turn(1000, 3000, 0)];
        let report = score(&reference, &hypothesis, 0);
        assert_eq!(report.missed_ms, 1000);
        assert_eq!(report.false_alarm_ms, 1000);
        assert_eq!(report.confusion_ms, 0);
        assert!((report.der() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn overlap_counts_once_per_speaker() {
        let reference = [named(0, 2000, "a"), named(1000, 2000, "b")];
        // Only one speaker found during the overlap.
        let hypothesis = [turn(0, 2000, 0)];
        let report = score(&reference, &hypothesis, 0);
        assert_eq!(report.scored_ms, 3000);
        assert_eq!(report.missed_ms, 1000);
    }

    #[test]
    fn collar_forgives_boundaries() {
        let reference = [named(0, 2000, "a"), named(2000, 4000, "b")];
        let hypothesis = [turn(0, 2200, 0), turn(2200, 4000, 1)];
        assert!(score(&reference, &hypothesis, 0).der() > 0.0);
        assert_eq!(score(&reference, &hypothesis, 250).der(), 0.0);
    }

    #[test]
    fn mapping_maximises_total_overlap() {
        // Greedy would pair row 0 with column 0 (9) and leave row 1 with 1;
        // the best total pairs 0→1 and 1→0 (8 + 8).
        let weights = vec![vec![9, 8], vec![8, 1]];
        assert_eq!(best_mapping(&weights), vec![Some(1), Some(0)]);
        // More rows than columns: someone goes unmatched.
        let weights = vec![vec![5], vec![7], vec![1]];
        assert_eq!(best_mapping(&weights), vec![None, Some(0), None]);
    }
}
