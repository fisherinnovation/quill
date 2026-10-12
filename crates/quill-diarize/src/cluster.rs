//! Quill's own speaker clustering, run on voice embeddings of the pieces of
//! speech sherpa-onnx's diarization found.
//!
//! sherpa-onnx clusters with complete linkage and no notion of outliers, so
//! a few odd embeddings (overlapping speech, a cough, a codec glitch) each
//! become a "speaker", and when the speaker count is given they use up slots
//! that real people needed, merging them. Here:
//!
//! - every piece's embedding is weighted by how long it is;
//! - clusters merge by centroid distance (as in pyannote), so one stray
//!   embedding can't keep two halves of a speaker apart;
//! - clusters with too little speech to be trusted are folded into the
//!   nearest real speaker afterwards, unless that would leave fewer speakers
//!   than the caller said there are.

/// One stretch of one speaker's speech with its voice embedding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Piece {
    pub duration_ms: u64,
    /// L2-normalised.
    pub embedding: Vec<f32>,
}

/// Clustering settings.
#[derive(Debug, Clone, Copy)]
pub struct ClusterOptions {
    /// Merge clusters while their centroids are closer than this cosine
    /// distance (when `speakers` isn't given).
    pub threshold: f32,
    /// The known number of speakers.
    pub speakers: Option<usize>,
    /// Clusters with less speech than this are treated as outliers and
    /// folded into the nearest larger cluster.
    pub min_speaker_ms: u64,
}

use serde::{Deserialize, Serialize};

impl ClusterOptions {
    /// The defaults, with the speaker count if known.
    pub fn new(speakers: Option<usize>) -> Self {
        Self {
            threshold: crate::DEFAULT_THRESHOLD,
            speakers: speakers.filter(|&n| n > 0),
            min_speaker_ms: crate::DEFAULT_MIN_SPEAKER_MS,
        }
    }
}

struct Cluster {
    /// Duration-weighted sum of member embeddings.
    sum: Vec<f32>,
    weight_ms: u64,
    members: Vec<usize>,
}

impl Cluster {
    fn centroid(&self) -> Vec<f32> {
        normalized(&self.sum)
    }

    fn absorb(&mut self, other: Cluster) {
        for (a, b) in self.sum.iter_mut().zip(&other.sum) {
            *a += b;
        }
        self.weight_ms += other.weight_ms;
        self.members.extend(other.members);
    }
}

/// Clusters pieces into speakers, returning each piece's speaker index
/// (0-based, dense, in no particular order).
pub fn cluster(pieces: &[Piece], options: &ClusterOptions) -> Vec<usize> {
    if pieces.is_empty() {
        return Vec::new();
    }
    let mut clusters: Vec<Cluster> = pieces
        .iter()
        .enumerate()
        .map(|(i, p)| Cluster {
            sum: p
                .embedding
                .iter()
                .map(|x| x * p.duration_ms as f32)
                .collect(),
            weight_ms: p.duration_ms,
            members: vec![i],
        })
        .collect();

    // 1. Merge by threshold. With a known count, also stop once only that
    //    many clusters are left (rarely reached before the threshold).
    let floor = options.speakers.unwrap_or(1).max(1);
    merge_while(&mut clusters, |count, distance| {
        count > floor && distance < options.threshold
    });

    // 2. Separate real speakers from outliers.
    let is_large = |c: &Cluster| c.weight_ms >= options.min_speaker_ms;
    let large = clusters.iter().filter(|c| is_large(c)).count();
    if let Some(want) = options.speakers
        && large > want
    {
        // Too many confident speakers: keep merging the closest ones,
        // considering only the large clusters.
        let (mut big, small): (Vec<_>, Vec<_>) = clusters.into_iter().partition(is_large);
        merge_while(&mut big, |count, _| count > want);
        clusters = big;
        clusters.extend(small);
    }
    // Keep the largest clusters as speakers: all large ones, or as many as
    // the caller says there are (promoting the biggest small clusters, the
    // people who barely spoke).
    clusters.sort_by_key(|c| std::cmp::Reverse(c.weight_ms));
    let keep = match options.speakers {
        Some(want) => want.min(clusters.len()),
        None => large.max(1),
    };
    let outliers = clusters.split_off(keep);
    let centroids: Vec<Vec<f32>> = clusters.iter().map(Cluster::centroid).collect();

    // 3. Label: each kept cluster's members, then each outlier cluster as a
    //    whole goes to the nearest kept centroid.
    let mut labels = vec![0; pieces.len()];
    for (speaker, c) in clusters.iter().enumerate() {
        for &m in &c.members {
            labels[m] = speaker;
        }
    }
    for o in outliers {
        let centroid = o.centroid();
        let speaker = nearest(&centroid, &centroids).unwrap_or(0);
        for m in o.members {
            labels[m] = speaker;
        }
    }
    labels
}

/// Repeatedly merges the two clusters with the closest centroids while
/// `go(cluster_count, distance)` says so.
fn merge_while(clusters: &mut Vec<Cluster>, go: impl Fn(usize, f32) -> bool) {
    let mut centroids: Vec<Vec<f32>> = clusters.iter().map(Cluster::centroid).collect();
    // Upper-triangle distances, recomputed only for the merged cluster.
    let n = clusters.len();
    let mut distance = vec![vec![f32::INFINITY; n]; n];
    for i in 0..n {
        for j in i + 1..n {
            distance[i][j] = cosine_distance(&centroids[i], &centroids[j]);
        }
    }
    let mut alive: Vec<bool> = vec![true; n];
    let mut count = n;
    let mut slots: Vec<Option<Cluster>> = std::mem::take(clusters).into_iter().map(Some).collect();
    while count > 1 {
        let mut best = (f32::INFINITY, 0, 0);
        for i in (0..n).filter(|&i| alive[i]) {
            for j in (i + 1..n).filter(|&j| alive[j]) {
                if distance[i][j] < best.0 {
                    best = (distance[i][j], i, j);
                }
            }
        }
        let (d, i, j) = best;
        if !go(count, d) {
            break;
        }
        // Merge j into i.
        let Some(other) = slots[j].take() else { break };
        let Some(target) = slots[i].as_mut() else {
            break;
        };
        target.absorb(other);
        centroids[i] = target.centroid();
        alive[j] = false;
        count -= 1;
        for k in (0..n).filter(|&k| alive[k] && k != i) {
            let d = cosine_distance(&centroids[i], &centroids[k]);
            let (a, b) = if k < i { (k, i) } else { (i, k) };
            distance[a][b] = d;
        }
    }
    *clusters = slots.into_iter().flatten().collect();
}

fn nearest(v: &[f32], centroids: &[Vec<f32>]) -> Option<usize> {
    centroids
        .iter()
        .enumerate()
        .map(|(i, c)| (i, cosine_distance(v, c)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// 1 − cosine similarity of two unit vectors.
pub fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    1.0 - a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>()
}

pub fn normalized(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter().map(|x| x / norm).collect()
    } else {
        v.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit vector near axis `axis`, nudged by `noise` along the next one.
    fn voice(axis: usize, noise: f32, duration_ms: u64) -> Piece {
        let mut v = vec![0.0; 8];
        v[axis] = 1.0;
        v[(axis + 1) % 8] = noise;
        Piece {
            duration_ms,
            embedding: normalized(&v),
        }
    }

    fn groups(labels: &[usize]) -> usize {
        labels.iter().max().map_or(0, |m| m + 1)
    }

    #[test]
    fn separates_distinct_voices_by_threshold() {
        let pieces = [
            voice(0, 0.1, 3000),
            voice(1, 0.1, 3000),
            voice(0, 0.2, 3000),
            voice(1, 0.0, 3000),
        ];
        let options = ClusterOptions {
            threshold: 0.5,
            speakers: None,
            min_speaker_ms: 2000,
        };
        let labels = cluster(&pieces, &options);
        assert_eq!(groups(&labels), 2);
        assert_eq!(labels[0], labels[2]);
        assert_eq!(labels[1], labels[3]);
        assert_ne!(labels[0], labels[1]);
    }

    #[test]
    fn short_outliers_join_the_nearest_speaker() {
        // A 0.5 s odd embedding shouldn't become its own speaker.
        let pieces = [voice(0, 0.0, 5000), voice(1, 0.0, 5000), voice(2, 0.9, 500)];
        let options = ClusterOptions {
            threshold: 0.5,
            speakers: None,
            min_speaker_ms: 2000,
        };
        let labels = cluster(&pieces, &options);
        assert_eq!(groups(&labels), 2);
        // Axis 2 nudged towards 3 is equally far from 0 and 1; it just has
        // to land on one of them.
        assert!(labels[2] == labels[0] || labels[2] == labels[1]);
    }

    #[test]
    fn known_count_keeps_a_quiet_speaker() {
        // Three people, one who barely spoke: with the count given, the
        // quiet one survives instead of being treated as an outlier.
        let pieces = [voice(0, 0.0, 9000), voice(1, 0.0, 9000), voice(2, 0.0, 800)];
        let options = ClusterOptions {
            threshold: 0.5,
            speakers: Some(3),
            min_speaker_ms: 2000,
        };
        assert_eq!(groups(&cluster(&pieces, &options)), 3);
    }

    #[test]
    fn known_count_merges_down() {
        // Four distinct clusters but only two speakers: the closest merge.
        let pieces = [
            voice(0, 0.0, 5000),
            voice(0, 0.6, 5000),
            voice(4, 0.0, 5000),
            voice(4, 0.6, 5000),
        ];
        let options = ClusterOptions {
            threshold: 0.1,
            speakers: Some(2),
            min_speaker_ms: 2000,
        };
        let labels = cluster(&pieces, &options);
        assert_eq!(groups(&labels), 2);
        assert_eq!(labels[0], labels[1]);
        assert_eq!(labels[2], labels[3]);
    }
}
