//! `Transcriber` backed by whisper.cpp through the `whisper-rs` bindings.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

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

        Ok(Self { ctx, threads, vad })
    }

    /// Where speech ends in 16 kHz mono `pcm`: the end of the last stretch
    /// Silero VAD hears as speech. For measuring how long a dictation took
    /// to arrive after the speaker stopped talking. `None` without a VAD
    /// model, or with no speech.
    pub fn speech_end(&self, pcm: &[f32]) -> Option<Duration> {
        let mut vad = self
            .vad
            .as_ref()?
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (_, end) = *vad::detect_speech(&mut vad, pcm).ok()?.last()?;
        Some(Duration::from_secs_f64(end as f64 / 16_000.0))
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

        // The encoder keeps its full 30 s window even for short clips. Sizing
        // it to the clip (`set_audio_ctx`) was faster, but on 2–4 s clips
        // Whisper then often typed the sentence twice, on CPU and GPU alike,
        // and its timestamps ran past the end of the audio (see "Short clips"
        // in docs/DESIGN_NOTES.md).
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
        Ok(segments)
    }
}

fn centis_to_ms(centis: i64) -> u64 {
    u64::try_from(centis).unwrap_or(0) * 10
}
