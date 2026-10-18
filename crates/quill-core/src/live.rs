//! Live dictation: transcribing while the user speaks, for captions, so
//! that by the time they let go most of it is already done.
//!
//! Whisper isn't a streaming model, so [`LiveTranscript::update`]
//! re-transcribes the audio that isn't locked in yet, every half second or
//! so. A segment (Whisper's unit, about a sentence) locks in once two passes
//! in a row agree on it and it isn't the last one, which may still be
//! growing, and only where there's a real pause after it to cut at. Its
//! audio is then dropped from later passes and its text primes the next
//! pass. Passes don't retry when unsure (audio that ends mid-word makes
//! Whisper unsure, and retries take seconds).
//! [`LiveTranscript::finish`] transcribes only what's left.

use quill_stt::{Segment, SttError, Transcriber};
use serde::Serialize;

use crate::dictation::{has_loop, join, options, tidy};
use crate::settings::DictationSettings;

const SAMPLES_PER_MS: usize = 16;
/// A segment ending this close to the end of the audio may still change.
const SETTLE_MS: u64 = 1000;
/// Less unlocked audio than this isn't worth a pass.
const MIN_PASS_MS: usize = 500;
/// Each pass is primed with the end of what's locked in.
const CONTEXT_CHARS: usize = 200;
/// Where to look for the pause after a segment: from a little before
/// Whisper's end time to well after it (short-clip timestamps run early).
/// Not bounded by the next segment's start: Whisper makes segments
/// contiguous, so that's the same time and says nothing about the pause.
const PAUSE_BEFORE_MS: u64 = 200;
const PAUSE_AFTER_MS: u64 = 800;
/// A pause: this long, and this much quieter than the segment's speech. A
/// stop consonant's silence ("t" in "fifty") is far shorter.
const PAUSE_MS: usize = 150;
const PAUSE_RATIO: f32 = 0.2;
/// Faster than anyone speaks (270 words a minute): locking more words than
/// this per second of audio means Whisper's timestamps are off (short-clip
/// mode can put a long stretch's end seconds early), and cutting there would
/// transcribe the same words again.
const MAX_WORDS_PER_SEC: f32 = 4.5;
/// How many words to check for a pass repeating the end of the locked text.
const MAX_OVERLAP_WORDS: usize = 12;
const FRAME: usize = 10 * SAMPLES_PER_MS;

/// What to show while the user speaks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Caption {
    /// Locked in: won't change.
    pub committed: String,
    /// The latest pass's guess at the rest: may still change.
    pub tentative: String,
}

#[derive(Debug, Default)]
pub struct LiveTranscript {
    /// Locked-in segments' text.
    committed: Vec<String>,
    /// Samples of audio the locked-in text covers.
    offset: usize,
    /// The unlocked segments from the last pass, to compare with the next.
    previous: Vec<String>,
    passes: usize,
}

impl LiveTranscript {
    pub fn new() -> Self {
        Self::default()
    }

    /// Passes run so far, and how much of the audio is locked in (ms), for
    /// timings in the log.
    pub fn progress(&self) -> (usize, u64) {
        (self.passes, (self.offset / SAMPLES_PER_MS) as u64)
    }

    pub fn caption(&self, settings: &DictationSettings) -> Caption {
        Caption {
            committed: tidy(&self.committed.join(" "), settings),
            tentative: tidy(&self.previous.join(" "), settings),
        }
    }

    /// One pass over `pcm`, all the audio so far (16 kHz mono).
    pub fn update(
        &mut self,
        transcriber: &dyn Transcriber,
        pcm: &[f32],
        settings: &DictationSettings,
    ) -> Result<Caption, SttError> {
        let tail = &pcm[self.offset.min(pcm.len())..];
        if tail.len() / SAMPLES_PER_MS < MIN_PASS_MS {
            return Ok(self.caption(settings));
        }
        let context = self.context();
        let mut opts = options(settings, context.as_deref());
        opts.no_fallback = true;
        let segments: Vec<Segment> = transcriber
            .transcribe(tail, &opts)?
            .into_iter()
            .filter(|s| !s.text.trim().is_empty())
            .collect();
        self.passes += 1;
        let mut texts: Vec<String> = segments.iter().map(|s| s.text.trim().to_string()).collect();
        if let Some(first) = texts.first_mut() {
            *first = without_overlap(&self.committed.join(" "), first);
        }
        let tail_ms = (tail.len() / SAMPLES_PER_MS) as u64;

        // Lock in the leading segments this pass and the last agree on,
        // except the last one, and only once they're clear of the end.
        let mut lock = 0;
        while lock + 1 < segments.len()
            && self.previous.get(lock) == Some(&texts[lock])
            && segments[lock].end_ms + SETTLE_MS <= tail_ms
        {
            lock += 1;
        }
        // Cut in the pause after the last of them; without one, a cut could
        // split a word, so lock nothing yet.
        let cut = (lock > 0)
            .then(|| {
                let last = &segments[lock - 1];
                let ms = |ms: u64| ms as usize * SAMPLES_PER_MS;
                let speech =
                    rms(&tail[ms(last.start_ms).min(tail.len())..ms(last.end_ms).min(tail.len())]);
                pause(
                    tail,
                    ms(last.end_ms.saturating_sub(PAUSE_BEFORE_MS)),
                    ms(last.end_ms + PAUSE_AFTER_MS),
                    speech * PAUSE_RATIO,
                )
            })
            .flatten();
        // Counts only: dictated text stays out of the logs.
        tracing::debug!(
            tail_ms,
            segments = segments.len(),
            agreed = lock,
            paused = cut.is_some(),
            "live pass"
        );
        // More words than could have been said in the audio being cut: the
        // timestamps are wrong, so lock nothing.
        let cut = cut.filter(|cut| {
            let words: usize = texts[..lock]
                .iter()
                .map(|t| t.split_whitespace().count())
                .sum();
            words as f32 <= MAX_WORDS_PER_SEC * (*cut as f32 / (SAMPLES_PER_MS * 1000) as f32) + 2.0
        });
        let lock = if cut.is_some() { lock } else { 0 };
        if let Some(cut) = cut {
            self.offset += cut;
            self.committed.extend_from_slice(&texts[..lock]);
        }
        self.previous = texts[lock..].to_vec();
        Ok(self.caption(settings))
    }

    /// The final text once the user lets go: what's locked in, plus the
    /// rest of `pcm` (all the audio) transcribed now. Tidied like
    /// [`crate::dictation::transcribe`]'s. If Whisper looped (repeated a
    /// passage), one pass over all of it instead, if that one didn't.
    pub fn finish(
        &self,
        transcriber: &dyn Transcriber,
        pcm: &[f32],
        settings: &DictationSettings,
    ) -> Result<String, SttError> {
        let tail = &pcm[self.offset.min(pcm.len())..];
        let mut text = self.committed.join(" ");
        if !tail.is_empty() {
            let context = self.context();
            let rest = without_overlap(
                &text,
                &join(&transcriber.transcribe(tail, &options(settings, context.as_deref()))?),
            );
            if !rest.is_empty() {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(&rest);
            }
        }
        if has_loop(&text) {
            let whole = join(&transcriber.transcribe(pcm, &options(settings, None))?);
            if !has_loop(&whole) {
                return Ok(tidy(&whole, settings));
            }
        }
        Ok(tidy(&text, settings))
    }

    /// The end of the locked-in text, to prime the next pass with.
    fn context(&self) -> Option<String> {
        let all = self.committed.join(" ");
        if all.is_empty() {
            return None;
        }
        let start = all
            .char_indices()
            .rev()
            .nth(CONTEXT_CHARS)
            .map_or(0, |(i, _)| i);
        // Start at a word.
        let from = all[start..]
            .find(' ')
            .filter(|_| start > 0)
            .map_or(start, |i| start + i + 1);
        Some(all[from..].to_string())
    }
}

/// The middle of the longest run of 10 ms frames in `pcm[lo..hi]` (clamped)
/// quieter than `threshold`, if it's at least [`PAUSE_MS`] long: a sample
/// index to cut at without splitting a word.
fn pause(pcm: &[f32], lo: usize, hi: usize, threshold: f32) -> Option<usize> {
    let hi = hi.min(pcm.len());
    let lo = lo.min(hi);
    let (mut best, mut run_start) = (None::<(usize, usize)>, None);
    let mut i = lo;
    while i + FRAME <= hi {
        let quiet = rms(&pcm[i..i + FRAME]) < threshold;
        match (quiet, run_start) {
            (true, None) => run_start = Some(i),
            (false, Some(start)) => {
                best = longer(best, (start, i));
                run_start = None;
            }
            _ => {}
        }
        i += FRAME;
    }
    if let Some(start) = run_start {
        best = longer(best, (start, i));
    }
    best.filter(|(start, end)| end - start >= PAUSE_MS * SAMPLES_PER_MS)
        .map(|(start, end)| (start + end) / 2)
}

fn longer(best: Option<(usize, usize)>, run: (usize, usize)) -> Option<(usize, usize)> {
    match best {
        Some(b) if b.1 - b.0 >= run.1 - run.0 => Some(b),
        _ => Some(run),
    }
}

/// `next` without any words at its start that repeat the end of `locked`
/// (a pass re-hearing the end of a locked sentence), compared ignoring case
/// and punctuation.
fn without_overlap(locked: &str, next: &str) -> String {
    let key = |w: &str| {
        w.trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase()
    };
    let tail: Vec<String> = locked.split_whitespace().map(key).collect();
    let words: Vec<&str> = next.split_whitespace().collect();
    let longest = (1..=MAX_OVERLAP_WORDS.min(tail.len()).min(words.len()))
        .rev()
        .find(|&n| {
            tail[tail.len() - n..]
                .iter()
                .zip(&words[..n])
                .all(|(a, b)| *a == key(b) && !a.is_empty())
        })
        .unwrap_or(0);
    // A single shared word is often just a common word ("the"), not a repeat.
    if longest < 2 {
        return next.to_string();
    }
    words[longest..].join(" ")
}

fn rms(pcm: &[f32]) -> f32 {
    if pcm.is_empty() {
        return 0.0;
    }
    (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quill_stt::TranscribeOptions;
    use std::sync::Mutex;

    /// Plays a script of words at fixed times, as speech-loud audio with
    /// silent pauses after sentences. It finds where the audio it's given
    /// starts from where that slice sits in the whole recording, and returns
    /// the sentences spoken in it, one still being spoken cut to the words
    /// said so far.
    struct Script {
        /// (start ms, end ms, word, ends a sentence)
        words: Vec<(u64, u64, String, bool)>,
        recording: Vec<f32>,
        prompts: Mutex<Vec<Option<String>>>,
        retries_off: Mutex<Vec<bool>>,
    }

    impl Script {
        fn new(text: &str) -> Self {
            // Each word takes 400 ms; a sentence ends with a 600 ms pause.
            let mut t = 0;
            let mut words = Vec::new();
            for word in text.split(' ') {
                let end_of_sentence = word.ends_with('.');
                words.push((t, t + 400, word.to_string(), end_of_sentence));
                t += if end_of_sentence { 1000 } else { 400 };
            }
            let length = words.last().map_or(0, |w| w.1 + 200) as usize;
            let mut recording = vec![0.0_f32; length * SAMPLES_PER_MS];
            for (start, end, ..) in &words {
                for (i, s) in recording
                    [*start as usize * SAMPLES_PER_MS..*end as usize * SAMPLES_PER_MS]
                    .iter_mut()
                    .enumerate()
                {
                    *s = if i % 2 == 0 { 0.3 } else { -0.3 };
                }
            }
            Self {
                words,
                recording,
                prompts: Mutex::new(Vec::new()),
                retries_off: Mutex::new(Vec::new()),
            }
        }
    }

    impl Transcriber for Script {
        fn transcribe(
            &self,
            pcm: &[f32],
            opts: &TranscribeOptions,
        ) -> Result<Vec<Segment>, SttError> {
            self.prompts
                .lock()
                .unwrap()
                .push(opts.initial_prompt.clone());
            self.retries_off.lock().unwrap().push(opts.no_fallback);
            let offset =
                (pcm.as_ptr() as usize - self.recording.as_ptr() as usize) / size_of::<f32>();
            let from = (offset / SAMPLES_PER_MS) as u64;
            let to = from + (pcm.len() / SAMPLES_PER_MS) as u64;
            let mut segments: Vec<Segment> = Vec::new();
            let mut current: Option<Segment> = None;
            for (start, end, word, last) in &self.words {
                // Words before this audio are already locked in; words not
                // finished by its end haven't been heard yet.
                if *start < from || *end > to {
                    continue;
                }
                let seg = current.get_or_insert(Segment {
                    start_ms: start - from,
                    end_ms: 0,
                    text: String::new(),
                    speaker: None,
                });
                seg.end_ms = end - from;
                seg.text = format!("{} {word}", seg.text).trim().to_string();
                if *last {
                    segments.extend(current.take());
                }
            }
            segments.extend(current);
            Ok(segments)
        }
    }

    fn run(script: &Script, step_ms: usize) -> (String, Vec<Caption>, LiveTranscript) {
        let settings = DictationSettings::default();
        let mut live = LiveTranscript::new();
        let mut captions = Vec::new();
        let all = &script.recording;
        let mut end = step_ms * SAMPLES_PER_MS;
        while end < all.len() {
            captions.push(live.update(script, &all[..end], &settings).unwrap());
            end += step_ms * SAMPLES_PER_MS;
        }
        let text = live.finish(script, all, &settings).unwrap();
        (text, captions, live)
    }

    #[test]
    fn locks_sentences_in_as_it_goes_and_finishes_with_the_rest() {
        let script = Script::new(
            "First we ship the redesign. Then we fix the events. And last the pricing tests.",
        );
        let (text, captions, live) = run(&script, 600);
        assert_eq!(
            text,
            "First we ship the redesign. Then we fix the events. And last the pricing tests."
        );
        // The first sentences were locked in before the end, so only the last
        // one was left to transcribe.
        let last = captions.last().unwrap();
        assert_eq!(
            last.committed,
            "First we ship the redesign. Then we fix the events."
        );
        assert!(last.tentative.starts_with("And last"), "{last:?}");
        let (passes, locked_ms) = live.progress();
        assert!(passes >= 10);
        // Cut inside the pause after "events." (4.6–5.2 s).
        assert!((4_600..5_200).contains(&locked_ms), "{locked_ms}");
        // Locked text never changes once shown.
        for pair in captions.windows(2) {
            assert!(
                pair[1].committed.starts_with(&pair[0].committed),
                "{pair:?}"
            );
        }
        // Later passes are primed with what's locked in; passes while
        // speaking don't retry, the final one may.
        assert!(
            script
                .prompts
                .lock()
                .unwrap()
                .iter()
                .any(|p| p.as_deref() == Some("First we ship the redesign."))
        );
        let retries_off = script.retries_off.lock().unwrap();
        assert!(retries_off[..retries_off.len() - 1].iter().all(|off| *off));
        assert_eq!(retries_off.last(), Some(&false));
    }

    #[test]
    fn short_dictation_locks_nothing_and_matches_a_single_pass() {
        let script = Script::new("Sounds good, see you then.");
        let (text, captions, live) = run(&script, 600);
        assert_eq!(text, "Sounds good, see you then.");
        assert!(captions.iter().all(|c| c.committed.is_empty()));
        assert_eq!(live.progress().1, 0);
    }

    #[test]
    fn cuts_only_in_a_real_pause() {
        let speech = |n: usize| {
            (0..n)
                .map(|i| if i % 2 == 0 { 0.3 } else { -0.3 })
                .collect::<Vec<f32>>()
        };
        // 1 s of speech, a 300 ms pause, 1 s of speech.
        let mut pcm = speech(16_000);
        pcm.extend(vec![0.0; 4_800]);
        pcm.extend(speech(16_000));
        let cut = pause(&pcm, 12_000, 30_000, 0.06).unwrap();
        assert!((18_000..19_600).contains(&cut), "{cut}");
        // A 60 ms gap, like a stop consonant, isn't a pause.
        let mut pcm = speech(16_000);
        pcm.extend(vec![0.0; 960]);
        pcm.extend(speech(16_000));
        assert_eq!(pause(&pcm, 12_000, 30_000, 0.06), None);
        // Out-of-range bounds are clamped.
        assert_eq!(pause(&pcm, 40_000, 50_000, 0.06), None);
    }

    #[test]
    fn a_pass_repeating_the_locked_end_is_trimmed() {
        assert_eq!(
            without_overlap(
                "We ship on Friday. Then we fix it.",
                "then we fix it. And the pricing."
            ),
            "And the pricing."
        );
        // One shared word isn't a repeat; nothing locked, nothing to trim.
        assert_eq!(
            without_overlap("Ship the redesign.", "The pricing next."),
            "The pricing next."
        );
        assert_eq!(without_overlap("", "Hello there."), "Hello there.");
        assert_eq!(without_overlap("Hello there.", ""), "");
    }

    /// Whisper with timestamps far too early: one long segment said to end
    /// at 1 s, then another.
    struct EarlyTimestamps;

    impl Transcriber for EarlyTimestamps {
        fn transcribe(&self, _: &[f32], _: &TranscribeOptions) -> Result<Vec<Segment>, SttError> {
            let segment = |start_ms, end_ms, text: &str| Segment {
                start_ms,
                end_ms,
                text: text.into(),
                speaker: None,
            };
            Ok(vec![
                segment(
                    0,
                    1_000,
                    "one two three four five six seven eight nine ten eleven twelve thirteen fourteen.",
                ),
                segment(1_000, 1_500, "fifteen"),
            ])
        }
    }

    #[test]
    fn implausibly_many_words_for_the_audio_dont_lock() {
        // A second of speech, then a long pause, then more: the cut would be
        // at about 1.3 s, far too soon for fourteen words.
        let speech = |n: usize| {
            (0..n)
                .map(|i| if i % 2 == 0 { 0.3 } else { -0.3 })
                .collect::<Vec<f32>>()
        };
        let mut pcm = speech(16_000);
        pcm.extend(vec![0.0; 16_000]);
        pcm.extend(speech(64_000));
        let settings = DictationSettings::default();
        let mut live = LiveTranscript::new();
        for _ in 0..3 {
            live.update(&EarlyTimestamps, &pcm, &settings).unwrap();
        }
        assert_eq!(live.progress().1, 0, "nothing locked");
    }

    #[test]
    fn context_starts_at_a_word() {
        let mut live = LiveTranscript::new();
        live.committed = vec!["word ".repeat(80).trim().to_string()];
        let context = live.context().unwrap();
        assert!(context.starts_with("word"));
        assert!(context.len() <= CONTEXT_CHARS);
    }
}
