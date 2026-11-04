//! Voice activity detection: find the speech, drop the silence, and map
//! timestamps back to the original audio.
//!
//! whisper.cpp has VAD built in, but only on its `whisper_full()` entry point;
//! whisper-rs calls `whisper_full_with_state()`, which silently skips it. So we
//! run the same Silero model ourselves through `WhisperVadContext` and stitch
//! the speech together the way whisper.cpp does: speech regions back to back
//! with 0.1 s of silence between them.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use whisper_rs::{WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

use crate::{Segment, SttError, Word};

pub(crate) const SAMPLES_PER_MS: usize = 16;
/// Silence inserted between packed speech regions, so words from separate
/// regions don't run together.
pub(crate) const GAP_SAMPLES: usize = 100 * SAMPLES_PER_MS;
/// A pause between a meeting's stretches of speech this long always ends a
/// segment (see [`split_at_long_pauses`]).
pub(crate) const LONG_PAUSE_SAMPLES: usize = 1000 * SAMPLES_PER_MS;
/// Silence packed in place of a long pause.
const LONG_GAP_SAMPLES: usize = 1000 * SAMPLES_PER_MS;

/// Where one stretch of speech sits in the original and the packed audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Region {
    /// Sample range in the original audio.
    pub orig_start: usize,
    pub orig_end: usize,
    /// Where the region starts in the packed audio.
    pub packed_start: usize,
}

/// Loads the Silero model. Keep it: loading costs far more than running it
/// on a short clip.
pub(crate) fn load(model_path: &str, threads: usize) -> Result<WhisperVadContext, SttError> {
    let mut ctx_params = WhisperVadContextParams::new();
    ctx_params.set_n_threads(i32::try_from(threads).unwrap_or(4));
    // The model is under 1 MB; GPU dispatch overhead would outweigh any gain.
    ctx_params.set_use_gpu(false);
    Ok(WhisperVadContext::new(model_path, ctx_params)?)
}

/// Loads the Silero model at `path`, if one is given, behind a mutex:
/// running it needs `&mut`, and it takes milliseconds, so concurrent
/// callers barely wait.
pub(crate) fn load_shared(
    path: Option<&PathBuf>,
    threads: usize,
) -> Result<Option<Mutex<WhisperVadContext>>, SttError> {
    match path {
        Some(path) if !path.is_file() => Err(SttError::ModelNotFound(path.clone())),
        // whisper.cpp takes the path as a C string, so it must be UTF-8.
        Some(path) => {
            let path = path
                .to_str()
                .ok_or_else(|| SttError::NonUtf8Path(path.clone()))?;
            Ok(Some(Mutex::new(load(path, threads)?)))
        }
        None => Ok(None),
    }
}

/// Speech sample ranges in `pcm`, from a shared Silero model.
pub(crate) fn detect_shared(
    vad: &Mutex<WhisperVadContext>,
    pcm: &[f32],
) -> Result<Vec<(usize, usize)>, SttError> {
    let mut vad = vad.lock().unwrap_or_else(PoisonError::into_inner);
    detect_speech(&mut vad, pcm)
}

/// Where speech ends in 16 kHz mono `pcm`: the end of the last stretch
/// Silero hears as speech, or `None` with no speech.
pub(crate) fn speech_end(vad: &Mutex<WhisperVadContext>, pcm: &[f32]) -> Option<Duration> {
    let (_, end) = *detect_shared(vad, pcm).ok()?.last()?;
    Some(Duration::from_secs_f64(end as f64 / 16_000.0))
}

/// Runs Silero VAD over 16 kHz mono audio and returns speech sample ranges.
pub(crate) fn detect_speech(
    vad: &mut WhisperVadContext,
    pcm: &[f32],
) -> Result<Vec<(usize, usize)>, SttError> {
    let segments = vad.segments_from_samples(WhisperVadParams::default(), pcm)?;
    // VAD timestamps are centiseconds; 1 cs = 160 samples at 16 kHz.
    let to_sample = |cs: f32| ((cs.max(0.0) * 160.0) as usize).min(pcm.len());
    Ok(segments
        .map(|s| (to_sample(s.start), to_sample(s.end)))
        .filter(|(start, end)| end > start)
        .collect())
}

/// Concatenates the speech ranges with short gaps between them, and longer
/// ones where the pause was long.
pub(crate) fn pack(pcm: &[f32], ranges: &[(usize, usize)]) -> (Vec<f32>, Vec<Region>) {
    pack_with_gap(pcm, ranges, GAP_SAMPLES, LONG_GAP_SAMPLES)
}

/// [`pack`] with `gap` samples of silence between the ranges, or `long_gap`
/// after a pause of [`LONG_PAUSE_SAMPLES`] or more.
pub(crate) fn pack_with_gap(
    pcm: &[f32],
    ranges: &[(usize, usize)],
    gap: usize,
    long_gap: usize,
) -> (Vec<f32>, Vec<Region>) {
    let speech: usize = ranges.iter().map(|(s, e)| e - s).sum();
    let mut packed = Vec::with_capacity(speech + ranges.len() * long_gap.max(gap));
    let mut regions: Vec<Region> = Vec::with_capacity(ranges.len());
    for &(start, end) in ranges {
        if let Some(before) = regions.last() {
            let long = start - before.orig_end >= LONG_PAUSE_SAMPLES;
            packed.resize(packed.len() + if long { long_gap } else { gap }, 0.0);
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

/// Kept before the first speech and after the last when trimming, since VAD
/// boundaries can clip soft word edges.
const TRIM_LEAD_SAMPLES: usize = 200 * SAMPLES_PER_MS;
const TRIM_TAIL_SAMPLES: usize = 300 * SAMPLES_PER_MS;
/// Silence appended after a trimmed clip. Audio that stops right after the
/// last word made greedy decoding stumble into slow temperature retries.
const TRIM_SILENCE_SAMPLES: usize = 500 * SAMPLES_PER_MS;

/// For short clips (dictation): keeps everything from just before the first
/// speech to just after the last, pauses included (they carry punctuation),
/// plus trailing silence. Returns one region for [`map_segment`].
///
/// `ranges` must be non-empty.
pub(crate) fn trim(pcm: &[f32], ranges: &[(usize, usize)]) -> (Vec<f32>, Vec<Region>) {
    let first = ranges.first().map_or(0, |r| r.0);
    let last = ranges.last().map_or(pcm.len(), |r| r.1);
    let start = first.saturating_sub(TRIM_LEAD_SAMPLES);
    let end = (last + TRIM_TAIL_SAMPLES).min(pcm.len()).max(start);
    let mut audio = Vec::with_capacity(end - start + TRIM_SILENCE_SAMPLES);
    audio.extend_from_slice(&pcm[start..end]);
    audio.resize(audio.len() + TRIM_SILENCE_SAMPLES, 0.0);
    let region = Region {
        orig_start: start,
        orig_end: end,
        packed_start: 0,
    };
    (audio, vec![region])
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

/// `segment` (mapped back onto the original audio, with timed words) cut
/// between words on either side of a long pause between `regions`.
///
/// Whisper hears the packed speech, where every pause is 0.1 s, so one of
/// its segments can hold speech from both sides of a long silence. Mapped
/// back, it then spans the silence too: on a call, that's often the other
/// person talking. The line sat among theirs as one turn, and its talk time
/// counted the silence (a 40-minute 1:1 came to 63 minutes of talking).
/// Without words, the segment is left whole.
pub(crate) fn split_at_long_pauses(segment: Segment, regions: &[Region]) -> Vec<Segment> {
    // Each long pause: where the speech before it ends and after it starts.
    let pauses: Vec<(u64, u64)> = regions
        .windows(2)
        .filter(|w| w[1].orig_start - w[0].orig_end >= LONG_PAUSE_SAMPLES)
        .map(|w| (ms(w[0].orig_end), ms(w[1].orig_start)))
        .collect();
    let apart = |a: &Word, b: &Word| {
        pauses
            .iter()
            .any(|&(end, start)| a.start_ms <= end && b.start_ms >= start)
    };
    let mut runs: Vec<Vec<Word>> = Vec::new();
    for word in &segment.words {
        match runs.last_mut() {
            Some(run) if !apart(&run[run.len() - 1], word) => run.push(word.clone()),
            _ => runs.push(vec![word.clone()]),
        }
    }
    if runs.len() < 2 {
        return vec![segment];
    }
    let last = runs.len() - 1;
    runs.into_iter()
        .enumerate()
        .map(|(i, words)| Segment {
            start_ms: if i == 0 {
                segment.start_ms
            } else {
                words[0].start_ms
            },
            end_ms: if i == last {
                segment.end_ms
            } else {
                words[words.len() - 1].end_ms
            },
            text: words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            speaker: segment.speaker.clone(),
            words,
        })
        .collect()
}

fn ms(samples: usize) -> u64 {
    (samples / SAMPLES_PER_MS) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_outer_silence_but_keeps_pauses() {
        let pcm = vec![0.5_f32; 10 * 16_000];
        // Speech at 2-3 s and 5-6 s.
        let (audio, regions) = trim(&pcm, &[(32_000, 48_000), (80_000, 96_000)]);
        // 1.8 s ..= 6.3 s, plus 0.5 s of silence.
        assert_eq!(audio.len(), (6_300 - 1_800 + 500) * 16);
        assert_eq!(audio.last(), Some(&0.0));
        assert_eq!(regions[0].orig_start, 1_800 * 16);
        // A segment at 0.2 s in the clip is at 2.0 s in the original.
        assert_eq!(map_segment(200, 1_200, &regions).0, 2_000);
    }

    #[test]
    fn packs_regions_with_gaps() {
        let pcm: Vec<f32> = (0..100_000).map(|i| i as f32).collect();
        // A 0.5 s pause, then a 1 s one.
        let (packed, regions) = pack(
            &pcm,
            &[(16_000, 32_000), (40_000, 56_000), (72_000, 80_000)],
        );

        assert_eq!(
            packed.len(),
            16_000 + GAP_SAMPLES + 16_000 + LONG_GAP_SAMPLES + 8_000
        );
        assert_eq!(packed[0], 16_000.0);
        assert_eq!(packed[16_000..16_000 + GAP_SAMPLES], [0.0; GAP_SAMPLES]);
        assert_eq!(packed[16_000 + GAP_SAMPLES], 40_000.0);
        assert_eq!(regions[1].packed_start, 16_000 + GAP_SAMPLES);
        assert_eq!(
            regions[2].packed_start,
            32_000 + GAP_SAMPLES + LONG_GAP_SAMPLES
        );
    }

    #[test]
    fn maps_segments_back_to_the_original_timeline() {
        // Speech at 1.0–2.0 s and 4.0–4.5 s of the original audio; packed,
        // region 2 starts at 1.1 s (1.0 s of speech + 100 ms gap).
        let (_, regions) = pack_with_gap(
            &vec![0.0; 100_000],
            &[(16_000, 32_000), (64_000, 72_000)],
            GAP_SAMPLES,
            GAP_SAMPLES,
        );
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

    #[test]
    fn segments_are_cut_at_long_pauses_only() {
        let word = |text: &str, start_ms, end_ms| Word {
            text: text.into(),
            start_ms,
            end_ms,
        };
        let segment = |words: Vec<Word>| Segment {
            start_ms: words[0].start_ms,
            end_ms: words[words.len() - 1].end_ms,
            text: words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            speaker: None,
            words,
        };
        // Speech at 1.0–2.0 s, 4.0–4.5 s (after a 2 s pause) and 5.0–6.0 s
        // (after half a second).
        let (_, regions) = pack(
            &vec![0.0; 100_000],
            &[(16_000, 32_000), (64_000, 72_000), (80_000, 96_000)],
        );
        let line = segment(vec![
            word("It", 1000, 1300),
            word("took", 1300, 1700),
            word("three", 4000, 4200),
            word("weeks.", 4200, 4500),
            word("Yeah.", 5000, 5400),
        ]);
        let pieces = split_at_long_pauses(line, &regions);
        let spans: Vec<_> = pieces
            .iter()
            .map(|s| (s.start_ms, s.end_ms, s.text.as_str(), s.words.len()))
            .collect();
        assert_eq!(
            spans,
            [
                (1000, 1700, "It took", 2),
                (4000, 5400, "three weeks. Yeah.", 3)
            ]
        );

        // Without word times there's nowhere to cut.
        let whole = Segment {
            words: Vec::new(),
            ..pieces[0].clone()
        };
        assert_eq!(split_at_long_pauses(whole.clone(), &regions), [whole]);
    }
}
