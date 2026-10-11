//! `Transcriber` backed by whisper.cpp through the `whisper-rs` bindings.

use std::path::Path;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::{Segment, SttError, TranscribeOptions, Transcriber};

/// Load-time settings for a Whisper model.
#[derive(Debug, Clone)]
pub struct WhisperConfig {
    /// Run on the GPU (Metal on macOS) when the build includes a GPU backend.
    /// Set to `false` to force CPU, e.g. for benchmarking.
    pub use_gpu: bool,
    /// CPU threads for the parts that run on the CPU.
    pub threads: usize,
}

impl Default for WhisperConfig {
    fn default() -> Self {
        // Use the cores the OS says we have, but cap it: whisper.cpp stops
        // getting faster past ~8 threads and the extra ones just contend.
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        Self {
            use_gpu: true,
            threads: cores.min(8),
        }
    }
}

/// A loaded Whisper model. Load once and reuse; loading takes seconds.
pub struct WhisperTranscriber {
    ctx: WhisperContext,
    threads: usize,
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

        let mut params = WhisperContextParameters::default();
        params.use_gpu(config.use_gpu);
        // Flash attention is a faster attention kernel; a clear win on Metal.
        params.flash_attn(config.use_gpu);
        let ctx = WhisperContext::new_with_params(model_path, params)?;

        Ok(Self {
            ctx,
            threads: config.threads.max(1),
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
        // dictation latency (M6) we can pool states instead.
        let mut state = self.ctx.create_state()?;

        // `FullParams<'a, 'b>` borrows the language and prompt strings rather
        // than copying them, so `opts` must outlive `params`. It does: both
        // live until the end of this function.
        //
        // Beam search rather than greedy decoding: with greedy, large-v3 fell
        // into a repetition loop ("I think that's a good thing." x110) 90 s
        // into the benchmark meeting and lost the rest of it. Beam search fixed
        // that at ~10% speed cost for turbo (see eval/benchmarks.md). Dictation
        // (M6) may want greedy for latency on short clips.
        let mut params = FullParams::new(SamplingStrategy::BeamSearch {
            beam_size: 5,
            patience: -1.0, // unused by whisper.cpp; -1 is its default
        });
        params.set_n_threads(i32::try_from(self.threads).unwrap_or(8));
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

        state.full(params, pcm_16k_mono)?;

        let mut segments = Vec::new();
        for segment in state.as_iter() {
            let text = segment.to_str_lossy()?.trim().to_string();
            if text.is_empty() {
                continue;
            }
            // whisper.cpp timestamps are in centiseconds (10 ms units).
            segments.push(Segment {
                start_ms: centis_to_ms(segment.start_timestamp()),
                end_ms: centis_to_ms(segment.end_timestamp()),
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
