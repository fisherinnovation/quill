//! `Transcriber` backed by NVIDIA Parakeet TDT, through sherpa-onnx's
//! offline recognizer (ONNX Runtime, on the CPU).
//!
//! Parakeet isn't Whisper: it takes no prompt and no language (v2 is
//! English only, v3 detects the language itself), and gives a time for
//! every token rather than segments. The vocabulary works differently too:
//! instead of a prompt, its terms are hotwords, which beam search favours. Quill splits its text into
//! sentences at those times, so meetings, live captions and diarization get
//! segments as they do from Whisper. Silence handling is shared with
//! Whisper: the same Silero VAD, trimming a dictation's outer silence or
//! packing a meeting's speech together.

use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};
use whisper_rs::WhisperVadContext;

use crate::models::ParakeetModel;
use crate::vad::{self, SAMPLES_PER_MS};
use crate::{ProgressEvent, Segment, SttError, TranscribeOptions, Transcriber};

/// Load-time settings for a Parakeet model.
#[derive(Debug, Clone)]
pub struct ParakeetConfig {
    /// CPU threads for ONNX Runtime.
    pub threads: usize,
    /// Silero VAD model, as for Whisper: only the speech is transcribed.
    pub vad_model: Option<PathBuf>,
}

impl Default for ParakeetConfig {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        Self {
            threads: cores.min(MAX_THREADS),
            vad_model: None,
        }
    }
}

/// The most threads worth giving ONNX Runtime. On a 24-core i9-14900KF, 4
/// and 8 were equally fast, and 16 four times slower (its efficiency cores
/// and hyperthreads hold the rest up); see "Parakeet" in
/// docs/DESIGN_NOTES.md.
pub(crate) const MAX_THREADS: usize = 4;

/// The most audio decoded at once. Parakeet's encoder attends over the
/// whole input, so time and memory grow faster than the audio does; long
/// recordings are cut into windows, at pauses between speech where there
/// are any.
const MAX_WINDOW_SAMPLES: usize = 30 * 1000 * SAMPLES_PER_MS;

/// Silence between a meeting's stretches of speech once they're packed
/// together: Whisper's 0.1 s was too little for Parakeet to hear the pause,
/// so it ran sentences, and speakers' turns, together with commas.
const PACK_GAP_SAMPLES: usize = 250 * SAMPLES_PER_MS;

/// A pause this long always ends a segment, punctuated or not.
const LONG_PAUSE_SAMPLES: usize = 1000 * SAMPLES_PER_MS;

/// How much a hotword's tokens are favoured, per token: sherpa-onnx's
/// default.
const HOTWORDS_SCORE: f32 = 1.5;

/// A loaded Parakeet model. Load once and reuse.
pub struct ParakeetTranscriber {
    recognizer: OfflineRecognizer,
    vad: Option<Mutex<WhisperVadContext>>,
    /// The characters the model can write, to leave out vocabulary terms
    /// it can't spell.
    letters: HashSet<char>,
}

impl ParakeetTranscriber {
    /// Loads `model`'s files from `models_dir`.
    pub fn load(
        model: &ParakeetModel,
        models_dir: &Path,
        config: &ParakeetConfig,
    ) -> Result<Self, SttError> {
        let path = |file: &crate::models::ModelInfo| -> Result<String, SttError> {
            let path = file.path_in(models_dir);
            if !path.is_file() {
                return Err(SttError::ModelNotFound(path));
            }
            path.to_str()
                .map(str::to_string)
                .ok_or(SttError::NonUtf8Path(path.clone()))
        };
        let threads = config.threads.max(1);
        let mut sherpa = OfflineRecognizerConfig::default();
        sherpa.model_config.transducer = OfflineTransducerModelConfig {
            encoder: Some(path(&model.encoder)?),
            decoder: Some(path(&model.decoder)?),
            joiner: Some(path(&model.joiner)?),
        };
        sherpa.model_config.tokens = Some(path(&model.tokens)?);
        sherpa.model_config.model_type = Some("nemo_transducer".into());
        sherpa.model_config.num_threads = i32::try_from(threads).unwrap_or(4);
        sherpa.model_config.provider = Some("cpu".into());
        // Beam search, which hotwords need. Without hotwords it gave the
        // same text as greedy decoding, within a few percent of its speed.
        sherpa.decoding_method = Some("modified_beam_search".into());
        sherpa.hotwords_score = HOTWORDS_SCORE;
        // Hotwords are spelled in the model's pieces by sherpa-onnx's own
        // SentencePiece encoder, from a `.vocab` file the export doesn't
        // include: it's made from tokens.txt, and read only while loading.
        // (Without "bpe", sherpa-onnx exits the process on the first
        // hotword.)
        let tokens = std::fs::read_to_string(model.tokens.path_in(models_dir))?;
        let vocab =
            std::env::temp_dir().join(format!("quill-{}-{}.vocab", model.name, std::process::id()));
        std::fs::write(&vocab, bpe_vocab(&tokens))?;
        sherpa.model_config.modeling_unit = Some("bpe".into());
        sherpa.model_config.bpe_vocab = Some(
            vocab
                .to_str()
                .ok_or_else(|| SttError::NonUtf8Path(vocab.clone()))?
                .to_string(),
        );
        tracing::info!(model = model.name, threads, "loading parakeet model");
        let recognizer = OfflineRecognizer::create(&sherpa);
        let _ = std::fs::remove_file(&vocab);
        let recognizer = recognizer.ok_or(SttError::ParakeetLoad)?;
        let vad = vad::load_shared(config.vad_model.as_ref(), threads)?;
        Ok(Self {
            recognizer,
            vad,
            letters: letters(&tokens),
        })
    }

    /// `vocabulary` as sherpa-onnx hotwords, one term a line. `None` if
    /// none are usable.
    fn hotwords(&self, vocabulary: &[String]) -> Option<String> {
        let lines: Vec<String> = vocabulary
            .iter()
            .filter_map(|term| usable_hotword(term, &self.letters))
            .collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    /// Decodes one window of 16 kHz audio into its tokens, with start
    /// times (ms) counted from `offset` samples.
    fn decode(
        &self,
        audio: &[f32],
        offset: usize,
        hotwords: Option<&str>,
    ) -> Result<Vec<Token>, SttError> {
        let stream = match hotwords {
            Some(hotwords) => self.recognizer.create_stream_with_hotwords(hotwords),
            None => self.recognizer.create_stream(),
        };
        stream.accept_waveform(16_000, audio);
        self.recognizer.decode(&stream);
        let Some(result) = stream.get_result() else {
            return Ok(Vec::new());
        };
        let offset_ms = (offset / SAMPLES_PER_MS) as f32;
        let times = result.timestamps.unwrap_or_default();
        let durations = result.durations.unwrap_or_default();
        Ok(result
            .tokens
            .into_iter()
            .enumerate()
            .map(|(i, text)| {
                let start = times.get(i).copied().unwrap_or(0.0) * 1000.0 + offset_ms;
                // TDT predicts each token's duration; 80 ms (one encoder
                // frame) if it didn't.
                let length = durations
                    .get(i)
                    .copied()
                    .filter(|d| *d > 0.0)
                    .unwrap_or(0.08);
                Token {
                    text: text.replace(WORD_START, " "),
                    start_ms: start.max(0.0) as u64,
                    end_ms: (start + length * 1000.0).max(0.0) as u64,
                }
            })
            .collect())
    }
}

impl Transcriber for ParakeetTranscriber {
    fn transcribe(
        &self,
        pcm_16k_mono: &[f32],
        opts: &TranscribeOptions,
    ) -> Result<Vec<Segment>, SttError> {
        // Speech only, as for Whisper (see `WhisperTranscriber`); where a
        // long recording may be cut (the middle of each pause); and where a
        // meeting's pauses are (ms into `audio`, and whether it was long).
        let (audio, regions, cuts, pauses) = match &self.vad {
            Some(vad) => {
                let ranges = vad::detect_shared(vad, pcm_16k_mono)?;
                if ranges.is_empty() {
                    return Ok(Vec::new());
                }
                if opts.short_clip {
                    let (audio, regions) = vad::trim(pcm_16k_mono, &ranges);
                    let start = regions.first().map_or(0, |r| r.orig_start);
                    let cuts = ranges
                        .windows(2)
                        .map(|w| ((w[0].1 + w[1].0) / 2).saturating_sub(start))
                        .collect();
                    (Cow::Owned(audio), Some(regions), cuts, Vec::new())
                } else {
                    let (audio, regions) =
                        vad::pack_with_gap(pcm_16k_mono, &ranges, PACK_GAP_SAMPLES);
                    let cuts = regions
                        .iter()
                        .skip(1)
                        .map(|r| r.packed_start - PACK_GAP_SAMPLES / 2)
                        .collect();
                    let pauses = regions
                        .windows(2)
                        .map(|w| {
                            let long = w[1].orig_start - w[0].orig_end >= LONG_PAUSE_SAMPLES;
                            ((w[1].packed_start / SAMPLES_PER_MS) as u64, long)
                        })
                        .collect();
                    (Cow::Owned(audio), Some(regions), cuts, pauses)
                }
            }
            None => (Cow::Borrowed(pcm_16k_mono), None, Vec::new(), Vec::new()),
        };

        let hotwords = self.hotwords(&opts.vocabulary);
        let windows = windows(audio.len(), &cuts, MAX_WINDOW_SAMPLES);
        let mut segments = Vec::new();
        for (i, &(start, end)) in windows.iter().enumerate() {
            let tokens = self.decode(&audio[start..end], start, hotwords.as_deref())?;
            let found = sentences(&tokens, &pauses);
            if let Some(progress) = &opts.progress {
                for s in &found {
                    (progress.0)(ProgressEvent::Line(s.text.clone()));
                }
                let percent = (i + 1) * 100 / windows.len();
                (progress.0)(ProgressEvent::Percent(u8::try_from(percent).unwrap_or(100)));
            }
            segments.extend(found);
        }
        if let Some(regions) = &regions {
            for s in &mut segments {
                (s.start_ms, s.end_ms) = vad::map_segment(s.start_ms, s.end_ms, regions);
            }
        }
        Ok(segments)
    }

    fn speech_end(&self, pcm: &[f32]) -> Option<Duration> {
        vad::speech_end(self.vad.as_ref()?, pcm)
    }
}

/// SentencePiece marks the start of a word with this.
const WORD_START: char = '\u{2581}';

/// Each line of `tokens.txt` (`<piece> <id>`) as a SentencePiece `.vocab`
/// line (`<piece>\t<score>`). BPE learned the pieces in id order, so a
/// lower id is a higher score: the order sherpa-onnx's encoder merges in.
fn bpe_vocab(tokens: &str) -> String {
    tokens
        .lines()
        .filter_map(|line| {
            let (piece, id) = line.rsplit_once(' ')?;
            let id: i64 = id.trim().parse().ok()?;
            Some(format!("{piece}\t{}\n", -id))
        })
        .collect()
}

/// The single characters among `tokens.txt`'s pieces.
fn letters(tokens: &str) -> HashSet<char> {
    tokens
        .lines()
        .filter_map(|line| {
            let (piece, _) = line.rsplit_once(' ')?;
            let mut chars = piece.chars();
            let c = chars.next()?;
            chars.next().is_none().then_some(c)
        })
        .collect()
}

/// `term` as one line of hotwords, or `None` if sherpa-onnx can't take it:
/// a letter the model doesn't know (which it would log and skip), a word
/// starting with `:`, `#` or `@` (its syntax for scores and thresholds,
/// which it throws on), or a `/` (which it reads as a line break).
fn usable_hotword(term: &str, letters: &HashSet<char>) -> Option<String> {
    let words: Vec<&str> = term.split_whitespace().collect();
    let usable = !words.is_empty()
        && words.iter().all(|w| {
            !w.starts_with([':', '#', '@']) && w.chars().all(|c| c != '/' && letters.contains(&c))
        });
    usable.then(|| words.join(" "))
}

/// One of Parakeet's tokens (a word or part of one), with its time.
#[derive(Debug, Clone, PartialEq)]
struct Token {
    /// Starts with a space when it starts a word.
    text: String,
    start_ms: u64,
    end_ms: u64,
}

/// Tokens grouped into sentences: a segment ends after a token ending in
/// `.`, `?` or `!` that the next token doesn't continue ("3.5" stays one),
/// and at a pause between stretches of speech (`pauses`: where the speech
/// after it starts, ms, and whether it was long) if the text so far ends
/// in punctuation or the pause was long. Meetings need that: one speaker's
/// turn often ends with a comma, and the next speaker's must be a segment
/// of its own to be told apart.
fn sentences(tokens: &[Token], pauses: &[(u64, bool)]) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut start = None;
    let mut last_end = 0;
    // Where the last word started. Not the last token: Parakeet often
    // writes the punctuation ending a turn only once the next one starts.
    let mut word_start = 0;
    for (i, token) in tokens.iter().enumerate() {
        let new_word = token.text.starts_with(' ');
        let paused = pauses
            .iter()
            .find(|(at, _)| word_start < *at && *at <= token.start_ms);
        let punctuated = text.trim_end().ends_with(['.', ',', '?', '!', ';', ':']);
        if new_word
            && let Some(&(_, long)) = paused
            && (long || punctuated)
            && !text.trim().is_empty()
        {
            segments.push(Segment {
                start_ms: start.unwrap_or(token.start_ms),
                end_ms: last_end,
                text: text.trim().to_string(),
                speaker: None,
            });
            text.clear();
            start = None;
        }
        last_end = token.end_ms;
        if token.text.chars().any(char::is_alphanumeric) {
            word_start = token.start_ms;
        }
        start.get_or_insert(token.start_ms);
        text.push_str(&token.text);
        let ends = token.text.trim_end().ends_with(['.', '?', '!']);
        let next_starts_word = tokens.get(i + 1).is_none_or(|t| t.text.starts_with(' '));
        if ends && next_starts_word || i + 1 == tokens.len() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                segments.push(Segment {
                    start_ms: start.unwrap_or(token.start_ms),
                    end_ms: token.end_ms,
                    text: trimmed.to_string(),
                    speaker: None,
                });
            }
            text.clear();
            start = None;
        }
    }
    segments
}

/// Splits `0..len` into windows of at most `max` samples, each ending at
/// the furthest of `cuts` (sorted sample positions) that fits, or at `max`
/// if none does.
fn windows(len: usize, cuts: &[usize], max: usize) -> Vec<(usize, usize)> {
    let mut windows = Vec::new();
    let mut start = 0;
    while len - start > max {
        let end = cuts
            .iter()
            .copied()
            .filter(|&c| c > start && c <= start + max)
            .max()
            .unwrap_or(start + max);
        windows.push((start, end));
        start = end;
    }
    if len > start || windows.is_empty() {
        windows.push((start, len));
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(text: &str, start_ms: u64, end_ms: u64) -> Token {
        Token {
            text: text.into(),
            start_ms,
            end_ms,
        }
    }

    #[test]
    fn tokens_become_sentences() {
        let tokens = [
            token(" Hi", 100, 300),
            token(" there", 300, 600),
            token(".", 600, 680),
            token(" It", 1_000, 1_200),
            token(" costs", 1_200, 1_500),
            token(" 3", 1_500, 1_600),
            token(".", 1_600, 1_650),
            token("5", 1_650, 1_700),
            token(" dollars", 1_700, 2_000),
            token("?", 2_000, 2_050),
            token(" Maybe", 2_500, 2_900),
        ];
        let segments = sentences(&tokens, &[]);
        let texts: Vec<_> = segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["Hi there.", "It costs 3.5 dollars?", "Maybe"]);
        assert_eq!((segments[0].start_ms, segments[0].end_ms), (100, 680));
        assert_eq!((segments[1].start_ms, segments[1].end_ms), (1_000, 2_050));
        assert!(sentences(&[], &[]).is_empty());
    }

    #[test]
    fn pauses_end_segments_after_punctuation_or_when_long() {
        let tokens = [
            token(" What", 0, 200),
            token(" are", 200, 400),
            token(" you", 400, 600),
            token(" hearing", 600, 900),
            // The comma comes late, once the next turn has started.
            token(",", 1_450, 1_500),
            token(" so", 1_500, 1_700),
            token(" we", 1_700, 1_900),
            token(" talked", 2_600, 2_900),
            token(" twice", 3_900, 4_200),
        ];
        // A pause after the comma ends the turn; one mid-phrase doesn't,
        // unless it was long.
        let pauses = [(1_400, false), (2_500, false), (3_800, true)];
        let texts: Vec<_> = sentences(&tokens, &pauses)
            .into_iter()
            .map(|s| s.text)
            .collect();
        assert_eq!(texts, ["What are you hearing,", "so we talked", "twice"]);
    }

    #[test]
    fn vocabulary_becomes_hotwords_sherpa_onnx_can_take() {
        let tokens = "<unk> 0\n\u{2581}t 1\nin 2\n\u{2581} 3\nt 4\ne 5\ns 6\nQ 7\nu 8\ni 9\nl 10\n";
        assert_eq!(bpe_vocab(tokens).lines().nth(1), Some("\u{2581}t\t-1"));
        let letters = letters(tokens);
        assert!(letters.contains(&'t') && !letters.contains(&'n'));
        assert_eq!(
            usable_hotword("  Quill  test ", &letters).as_deref(),
            Some("Quill test")
        );
        // A letter it doesn't know, its own syntax, a line break.
        for term in ["Quinn", ":tl", "#test", "@test", "tl/s", " "] {
            assert_eq!(usable_hotword(term, &letters), None, "{term}");
        }
    }

    #[test]
    fn long_audio_is_cut_at_pauses() {
        // Short audio is one window, whatever the cuts.
        assert_eq!(windows(100, &[50], 200), [(0, 100)]);
        // At the furthest pause that fits.
        assert_eq!(
            windows(250, &[40, 90, 120, 210], 100),
            [(0, 90), (90, 120), (120, 210), (210, 250)]
        );
        // No pause in reach: cut at the limit.
        assert_eq!(windows(250, &[], 100), [(0, 100), (100, 200), (200, 250)]);
        assert_eq!(windows(0, &[], 100), [(0, 0)]);
    }
}
