//! Voice activity detection: find the speech, drop the silence, and map
//! timestamps back to the original audio.
//!
//! whisper.cpp has VAD built in, but only on its `whisper_full()` entry point;
//! whisper-rs calls `whisper_full_with_state()`, which silently skips it. So we
//! run the same Silero model ourselves through `WhisperVadContext` and stitch
//! the speech together the way whisper.cpp does: speech regions back to back
//! with 0.1 s of silence between them.

use whisper_rs::{WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

use crate::SttError;

const SAMPLES_PER_MS: usize = 16;
/// Silence inserted between packed speech regions, so words from separate
/// regions don't run together.
const GAP_SAMPLES: usize = 100 * SAMPLES_PER_MS;

/// Where one stretch of speech sits in the original and the packed audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Region {
    /// Sample range in the original audio.
    pub orig_start: usize,
    pub orig_end: usize,
    /// Where the region starts in the packed audio.
    pub packed_start: usize,
}

/// Runs Silero VAD over 16 kHz mono audio and returns speech sample ranges.
pub(crate) fn detect_speech(
    model_path: &str,
    pcm: &[f32],
    threads: usize,
) -> Result<Vec<(usize, usize)>, SttError> {
    let mut ctx_params = WhisperVadContextParams::new();
    ctx_params.set_n_threads(i32::try_from(threads).unwrap_or(4));
    // The model is under 1 MB; GPU dispatch overhead would outweigh any gain.
    ctx_params.set_use_gpu(false);
    let mut vad = WhisperVadContext::new(model_path, ctx_params)?;

    let segments = vad.segments_from_samples(WhisperVadParams::default(), pcm)?;
    // VAD timestamps are centiseconds; 1 cs = 160 samples at 16 kHz.
    let to_sample = |cs: f32| ((cs.max(0.0) * 160.0) as usize).min(pcm.len());
    Ok(segments
        .map(|s| (to_sample(s.start), to_sample(s.end)))
        .filter(|(start, end)| end > start)
        .collect())
}

/// Concatenates the speech ranges with short gaps between them.
pub(crate) fn pack(pcm: &[f32], ranges: &[(usize, usize)]) -> (Vec<f32>, Vec<Region>) {
    let speech: usize = ranges.iter().map(|(s, e)| e - s).sum();
    let mut packed = Vec::with_capacity(speech + ranges.len() * GAP_SAMPLES);
    let mut regions = Vec::with_capacity(ranges.len());
    for (i, &(start, end)) in ranges.iter().enumerate() {
        if i > 0 {
            packed.resize(packed.len() + GAP_SAMPLES, 0.0);
        }
        regions.push(Region {
            orig_start: start,
            orig_end: end,
            packed_start: packed.len(),
        });
        packed.extend_from_slice(&pcm[start..end]);
    }
    (packed, regions)
}

/// A segment's overlap with a region is "substantial" (that region holds
/// real speech of the segment, not just a sliver of its edge) if it covers at
/// least this much of the region…
const SUBSTANTIAL_OVERLAP: usize = 1000 * SAMPLES_PER_MS;

impl Region {
    fn packed_end(&self) -> usize {
        self.packed_start + (self.orig_end - self.orig_start)
    }

    /// Samples of `start..end` (packed time) that fall inside this region.
    fn overlap(&self, start: usize, end: usize) -> usize {
        end.min(self.packed_end())
            .saturating_sub(start.max(self.packed_start))
    }

    /// …or at least half of a short region.
    fn substantially_overlaps(&self, start: usize, end: usize) -> bool {
        let overlap = self.overlap(start, end);
        let len = self.orig_end - self.orig_start;
        overlap >= SUBSTANTIAL_OVERLAP || overlap * 2 >= len
    }

    /// Packed time → original time, clamped to this region.
    fn original_time(&self, packed: usize) -> usize {
        let offset = packed.clamp(self.packed_start, self.packed_end()) - self.packed_start;
        self.orig_start + offset
    }
}

/// Maps a segment's packed start/end (ms) back onto the original audio.
///
/// Whisper's segment boundaries are approximate: a segment often "starts"
/// where the previous one ended, in the trailing silence of an earlier
/// speech region. Mapping that time directly would place the segment too
/// early, even before another speaker's turn. So the start snaps to the first
/// region the segment substantially overlaps, and the end to the last.
pub(crate) fn map_segment(start_ms: u64, end_ms: u64, regions: &[Region]) -> (u64, u64) {
    let to_samples = |ms: u64| usize::try_from(ms).unwrap_or(usize::MAX / 32) * SAMPLES_PER_MS;
    let (start, end) = (
        to_samples(start_ms),
        to_samples(end_ms).max(to_samples(start_ms)),
    );

    let overlapping: Vec<&Region> = regions
        .iter()
        .filter(|r| r.overlap(start, end) > 0)
        .collect();
    let substantial: Vec<&Region> = overlapping
        .iter()
        .copied()
        .filter(|r| r.substantially_overlaps(start, end))
        .collect();
    // Prefer regions with real overlap; a short segment may only have small
    // ones, so fall back to the one it overlaps most.
    let (first, last) = match (substantial.first(), substantial.last()) {
        (Some(first), Some(last)) => (*first, *last),
        _ => match overlapping.iter().max_by_key(|r| r.overlap(start, end)) {
            Some(best) => (*best, *best),
            // Entirely inside a gap: use the nearest region before it.
            None => match regions
                .iter()
                .rev()
                .find(|r| r.packed_start <= start)
                .or(regions.first())
            {
                Some(r) => (r, r),
                None => return (start_ms, end_ms),
            },
        },
    };
    (ms(first.original_time(start)), ms(last.original_time(end)))
}

fn ms(samples: usize) -> u64 {
    (samples / SAMPLES_PER_MS) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_regions_with_gaps() {
        let pcm: Vec<f32> = (0..100_000).map(|i| i as f32).collect();
        let (packed, regions) = pack(&pcm, &[(16_000, 32_000), (64_000, 72_000)]);

        assert_eq!(packed.len(), 16_000 + GAP_SAMPLES + 8_000);
        assert_eq!(packed[0], 16_000.0);
        assert_eq!(packed[16_000..16_000 + GAP_SAMPLES], [0.0; GAP_SAMPLES]);
        assert_eq!(packed[16_000 + GAP_SAMPLES], 64_000.0);
        assert_eq!(regions[1].packed_start, 16_000 + GAP_SAMPLES);
    }

    #[test]
    fn maps_segments_back_to_the_original_timeline() {
        // Speech at 1.0–2.0 s and 4.0–4.5 s of the original audio; packed,
        // region 2 starts at 1.1 s (1.0 s of speech + 100 ms gap).
        let (_, regions) = pack(&vec![0.0; 100_000], &[(16_000, 32_000), (64_000, 72_000)]);
        let map = |s, e| map_segment(s, e, &regions);

        // Inside one region: just shifted.
        assert_eq!(map(0, 500), (1000, 1500));
        assert_eq!(map(1200, 1350), (4100, 4250));
        // Starts in region 1's last 100 ms (its trailing silence) but the
        // speech is in region 2: the start snaps to region 2.
        assert_eq!(map(900, 1500), (4000, 4400));
        // Spans both regions substantially: start in 1, end in 2.
        assert_eq!(map(300, 1400), (1300, 4300));
        // Short segment with only small overlaps: the bigger one wins.
        assert_eq!(map(950, 1150), (4000, 4050));
        // Past the end: clamped to the last region.
        assert_eq!(map(1500, 9999), (4400, 4500));
    }
}
