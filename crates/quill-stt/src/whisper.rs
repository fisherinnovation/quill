//! `Transcriber` backed by whisper.cpp through the `whisper-rs` bindings.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperVadContext,
};

use crate::{
    GpuChoice, ProgressEvent, Segment, SttError, TranscribeOptions, Transcriber, gpu, vad,
};

/// Load-time settings for a Whisper model.
#[derive(Debug, Clone)]
pub struct WhisperConfig {
    /// Run on the GPU (Metal on macOS, Vulkan on Windows) when the build
    /// includes a GPU backend; see [`gpu_backend`].
    /// Set to `false` to force CPU, e.g. for benchmarking.
    pub use_gpu: bool,
    /// Which graphics card, when there's a choice (see [`gpu`]).
    pub gpu: GpuChoice,
    /// CPU threads for the parts that run on the CPU.
    pub threads: usize,
    /// Silero VAD model (`ggml-silero-*.bin`). When set, voice activity
    /// detection runs first and only the speech is transcribed, which stops
    /// Whisper inventing text during silence and noise.
    pub vad_model: Option<PathBuf>,
}

impl Default for WhisperConfig {
    fn default() -> Self {
        // Use the cores the OS says we have, but cap it: whisper.cpp stops
        // getting faster past ~8 threads and the extra ones just contend.
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        Self {
            use_gpu: true,
            gpu: GpuChoice::Automatic,
            threads: cores.min(8),
            vad_model: None,
        }
    }
}

/// The GPU backend whisper.cpp was built with, if any: Metal on macOS and
/// Vulkan on Windows always; CUDA or Vulkan elsewhere when the feature is on.
pub fn gpu_backend() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("Metal")
    } else if cfg!(feature = "cuda") {
        Some("CUDA")
    } else if cfg!(any(windows, feature = "vulkan")) {
        Some("Vulkan")
    } else {
        None
    }
}

/// A loaded Whisper model. Load once and reuse; loading takes seconds.
pub struct WhisperTranscriber {
    ctx: WhisperContext,
    threads: usize,
    /// Silero VAD, loaded once. Behind a mutex because running it needs
    /// `&mut`; it takes milliseconds, so concurrent callers barely wait.
    vad: Option<Mutex<WhisperVadContext>>,
    /// Size the encoder to short clips (see `transcribe`): only on Metal,
    /// where the full window costs ~0.5 s more per dictation.
    fit_encoder: bool,
}

impl WhisperTranscriber {
    /// Loads a ggml model file (e.g. `ggml-large-v3-turbo.bin`).
    pub fn load(model_path: &Path, config: &WhisperConfig) -> Result<Self, SttError> {
        if !model_path.is_file() {
            return Err(SttError::ModelNotFound(model_path.to_path_buf()));
        }
        // Send whisper.cpp's chatty logging through `tracing` instead of
        // straight to stderr. Safe to call repeatedly.
        whisper_rs::install_logging_hooks();

        let backend = gpu_backend().filter(|_| config.use_gpu);
        // The card, by its position among whisper.cpp's GPUs; none means the
        // CPU (no backend, no usable card, or the user chose the CPU).
        let gpus = if backend.is_some() {
            gpu::gpus()
        } else {
            Vec::new()
        };
        let device = gpu::choose(&gpus, &config.gpu);
        let use_gpu = device.is_some();
        tracing::info!(
            backend = if use_gpu {
                backend.unwrap_or("CPU")
            } else {
                "CPU"
            },
            gpu = device.map_or("none", |i| gpus[i].name.as_str()),
            gpus = gpus.len(),
            "loading whisper model"
        );
        let mut params = WhisperContextParameters::default();
        params.use_gpu(use_gpu);
        if let Some(device) = device {
            params.gpu_device(i32::try_from(device).unwrap_or(0));
        }
        // Flash attention is a faster attention kernel; a clear win on Metal.
        params.flash_attn(use_gpu);
        let ctx = WhisperContext::new_with_params(model_path, params)?;

        let threads = config.threads.max(1);
        let vad = match &config.vad_model {
            Some(path) if !path.is_file() => return Err(SttError::ModelNotFound(path.clone())),
            // whisper.cpp takes the path as a C string, so it must be UTF-8.
            Some(path) => {
                let path = path
                    .to_str()
                    .ok_or_else(|| SttError::NonUtf8Path(path.clone()))?;
                Some(Mutex::new(vad::load(path, threads)?))
            }
            None => None,
        };

        Ok(Self {
            ctx,
            threads,
            vad,
            fit_encoder: use_gpu && backend == Some("Metal"),
        })
    }
}

impl Transcriber for WhisperTranscriber {
    fn transcribe(
        &self,
        pcm_16k_mono: &[f32],
        opts: &TranscribeOptions,
    ) -> Result<Vec<Segment>, SttError> {
        // A state holds the scratch buffers for one transcription. Creating one
        // per call keeps `&self` shareable across threads; if this shows up in
        // dictation latency we can pool states instead.
        let mut state = self.ctx.create_state()?;

        // `FullParams<'a, 'b>` borrows the language and prompt strings rather
        // than copying them, so `opts` must outlive `params`. It does: both
        // live until the end of this function.
        //
        // Beam search rather than greedy decoding: with greedy, large-v3 fell
        // into a repetition loop ("I think that's a good thing." x110) 90 s
        // into the benchmark meeting and lost the rest of it. Beam search fixed
        // that at ~10% speed cost for turbo (see eval/benchmarks.md). Short
        // clips (dictation) use greedy: too short to loop, and faster.
        let mut params = if opts.short_clip {
            FullParams::new(SamplingStrategy::Greedy { best_of: 1 })
        } else {
            FullParams::new(SamplingStrategy::BeamSearch {
                beam_size: 5,
                patience: -1.0, // unused by whisper.cpp; -1 is its default
            })
        };
        params.set_n_threads(i32::try_from(self.threads).unwrap_or(8));
        if opts.no_fallback {
            params.set_temperature_inc(0.0);
        }
        params.set_language(Some(opts.language.as_deref().unwrap_or("auto")));
        if let Some(prompt) = &opts.initial_prompt {
            params.set_initial_prompt(prompt);
        }
        // whisper.cpp prints progress and results to stdout by default; we
        // return segments instead, so keep stdout clean for our own output.
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        if let Some(progress) = &opts.progress {
            let percent = progress.clone();
            params.set_progress_callback_safe(move |p: i32| {
                (percent.0)(ProgressEvent::Percent(
                    u8::try_from(p.clamp(0, 100)).unwrap_or(0),
                ));
            });
            let line = progress.clone();
            params.set_segment_callback_safe_lossy(
                move |segment: whisper_rs::SegmentCallbackData| {
                    let text = segment.text.trim();
                    if !text.is_empty() {
                        (line.0)(ProgressEvent::Line(text.to_string()));
                    }
                },
            );
        }

        // With VAD, transcribe only the speech, packed together; `regions`
        // remembers where each piece came from. `Cow` lets the no-VAD path
        // borrow the caller's samples instead of copying them.
        let (audio, regions) = match &self.vad {
            Some(vad) => {
                let ranges = {
                    let mut vad = vad
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    vad::detect_speech(&mut vad, pcm_16k_mono)?
                };
                if ranges.is_empty() {
                    return Ok(Vec::new());
                }
                // Meetings drop long silences outright; dictation keeps its
                // pauses (see `vad::trim`).
                let (audio, regions) = if opts.short_clip {
                    vad::trim(pcm_16k_mono, &ranges)
                } else {
                    vad::pack(pcm_16k_mono, &ranges)
                };
                (Cow::Owned(audio), Some(regions))
            }
            None => (Cow::Borrowed(pcm_16k_mono), None),
        };

        // The encoder processes a 30 s window (1500 frames of 20 ms) unless
        // told otherwise. On Metal that's ~0.6 s for any clip, so for short
        // ones it's sized to the clip plus 3 s of margin: a 3 s dictation
        // takes ~0.15 s. Sized that small, Whisper often hears the clip twice,
        // ending a segment and starting another with the same words ("Can you
        // send it over? Can you send it over?"), which `without_repeats`
        // drops below; with only ~1.3 s of margin it also misheard more short
        // words. Elsewhere the full window costs only ~50 ms (Vulkan), so it's
        // kept. (Not for long audio either: quality drops when it's cut
        // short.) See "Short clips" in docs/DESIGN_NOTES.md.
        let frames = audio.len().div_ceil(320) + FIT_MARGIN_FRAMES;
        let fit = opts.short_clip && self.fit_encoder && frames < 1500;
        if fit {
            if opts.language.is_none() {
                // Language detection on the shortened window picks nonsense
                // ("Tex Tex" for English), so detect it first with the full
                // one, in a separate state: reusing this one left its encoder
                // sized for 30 s and the decode came out garbled.
                let mut detect = self.ctx.create_state()?;
                detect.pcm_to_mel(&audio, self.threads)?;
                let (id, _) = detect.lang_detect(0, self.threads)?;
                tracing::debug!(language = ?whisper_rs::get_lang_str(id), "detected language");
                if let Some(language) = whisper_rs::get_lang_str(id) {
                    params.set_language(Some(language));
                }
            }
            params.set_audio_ctx(i32::try_from(frames).unwrap_or(1500));
        }
        state.full(params, &audio)?;

        let mut segments = Vec::new();
        for segment in state.as_iter() {
            let text = segment.to_str_lossy()?.trim().to_string();
            if text.is_empty() {
                continue;
            }
            // whisper.cpp timestamps are in centiseconds (10 ms units).
            let start = centis_to_ms(segment.start_timestamp());
            let end = centis_to_ms(segment.end_timestamp());
            let (start_ms, end_ms) = match &regions {
                Some(regions) => vad::map_segment(start, end, regions),
                None => (start, end),
            };
            segments.push(Segment {
                start_ms,
                end_ms,
                text,
                speaker: None,
            });
        }
        if fit {
            let before = segments.len();
            segments = without_repeats(segments);
            if segments.len() < before {
                // Counts only: dictated text stays out of the logs.
                tracing::debug!(
                    dropped = before - segments.len(),
                    "dropped repeated segments"
                );
            }
        }
        Ok(segments)
    }
}

/// `segments` without the ones Whisper didn't hear. With the encoder sized
/// to a short clip, it often ends a segment and then transcribes the same
/// audio again as the next: all of it ("Can you send it over? Can you send
/// it over?"), its last sentence ("That makes sense. Thanks. Thanks."), or
/// an earlier sentence; and once, at the end of a long pass, it added a
/// sentence of its own in half a second. Dropped: a segment that ends the
/// one before it word for word, one of 3 or more words found in what came
/// before (compared on letters and digits only, so "follow-up" and "follow
/// up" match), and one after the first with more words than its time
/// allows.
fn without_repeats(segments: Vec<Segment>) -> Vec<Segment> {
    let words = |text: &str| -> Vec<String> {
        text.split_whitespace()
            .map(|w| {
                w.chars()
                    .filter(|c| c.is_alphanumeric())
                    .flat_map(char::to_lowercase)
                    .collect::<String>()
            })
            .filter(|w| !w.is_empty())
            .collect()
    };
    let mut kept: Vec<Segment> = Vec::with_capacity(segments.len());
    // What's kept so far, as one run of letters and digits.
    let mut before = String::new();
    let mut previous: Vec<String> = Vec::new();
    for segment in segments {
        let these = words(&segment.text);
        let letters = these.concat();
        let repeat = !these.is_empty()
            && (previous.ends_with(&these) || (these.len() >= 3 && before.contains(&letters)));
        let secs = segment.end_ms.saturating_sub(segment.start_ms) as f32 / 1000.0;
        let too_fast = !kept.is_empty() && these.len() as f32 > 1.0 + MAX_WORDS_PER_SEC * secs;
        if !repeat && !too_fast {
            before.push_str(&letters);
            previous = these;
            kept.push(segment);
        }
    }
    kept
}

/// Far faster than anyone speaks. In 334 clips, segments Whisper heard came
/// at most 4.3 words a second (not counting the first, whose end time is
/// often seconds out); the ones it invented, 13 and up.
const MAX_WORDS_PER_SEC: f32 = 8.0;

/// The margin after a short clip when the encoder is sized to it: 3 s. With
/// 1.3 s (64 frames), 230 short TTS clips had a WER of 8.0%, against 4.2%
/// with this and 3.0% with the full window.
const FIT_MARGIN_FRAMES: usize = 150;

fn centis_to_ms(centis: i64) -> u64 {
    u64::try_from(centis).unwrap_or(0) * 10
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(start_ms: u64, end_ms: u64, text: &str) -> Segment {
        Segment {
            start_ms,
            end_ms,
            text: text.into(),
            speaker: None,
        }
    }

    #[test]
    fn a_clip_heard_twice_is_typed_once() {
        // As Whisper produced it with the encoder sized to a 2.7 s clip.
        let kept = without_repeats(vec![
            segment(0, 1_600, "I'll be there in five minutes."),
            segment(1_600, 14_600, "I'll be there in five minutes."),
        ]);
        assert_eq!(
            kept,
            vec![segment(0, 1_600, "I'll be there in five minutes.")]
        );
        // Several times over, with case and punctuation differing.
        let kept = without_repeats(vec![
            segment(0, 1_300, "The build is green again."),
            segment(1_300, 1_300, "The build is green again."),
            segment(1_300, 1_300, "the build is green again"),
            segment(1_300, 14_300, "The build is green again!"),
        ]);
        assert_eq!(kept.len(), 1);
        // A repeat of the last of several segments.
        let kept = without_repeats(vec![
            segment(0, 6_900, "First we ship it."),
            segment(6_900, 19_300, "Then we fix the events."),
            segment(19_300, 23_300, "Then we fix the events."),
        ]);
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn a_repeated_last_sentence_or_earlier_sentence_is_dropped() {
        // The end of the segment before, however short.
        let kept = without_repeats(vec![
            segment(0, 1_700, "That makes sense. Thanks."),
            segment(1_700, 14_000, "Thanks."),
        ]);
        assert_eq!(kept.len(), 1);
        // An earlier sentence, spelled a little differently.
        let kept = without_repeats(vec![
            segment(0, 5_700, "Can we move the launch to next week?"),
            segment(5_700, 11_900, "The copy still needs a review."),
            segment(11_900, 18_800, "Can we move the launch to next week?"),
        ]);
        assert_eq!(kept.len(), 2);
        let kept = without_repeats(vec![
            segment(0, 2_000, "We set up the follow-up call."),
            segment(2_000, 9_000, "We set up the follow up call."),
        ]);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn words_faster_than_speech_are_dropped() {
        // Seven words in half a second at the end of a 17 s pass.
        let kept = without_repeats(vec![
            segment(
                0,
                16_040,
                "Then we fix the events, and after that the pricing tests.",
            ),
            segment(16_040, 16_560, "And then the door was open again."),
        ]);
        assert_eq!(kept.len(), 1);
        // The first segment's end time is often far off, so it's never judged.
        let first = vec![segment(0, 500, "Okay, I just gave it a run and it works.")];
        assert_eq!(without_repeats(first.clone()), first);
    }

    #[test]
    fn new_words_are_kept() {
        let segments = vec![
            segment(0, 1_000, "Yes."),
            segment(1_000, 2_000, "I think so."),
            // Short, and not the end of the one before.
            segment(2_000, 3_000, "Yes."),
            segment(3_000, 4_000, "Yes, I'll be there."),
            // Its words are all earlier, but not in this order.
            segment(4_000, 5_000, "There I'll be."),
        ];
        assert_eq!(without_repeats(segments.clone()), segments);
    }
}
