//! `quill-cli bench`: transcribe one file with several models and print a
//! Markdown table of speeds, ready to paste into `eval/benchmarks.md`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context;
use quill_audio::STT_SAMPLE_RATE;
use quill_stt::{TranscribeOptions, Transcriber, WhisperConfig, WhisperTranscriber};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// WAV file to transcribe
    file: PathBuf,
    /// Comma-separated model names; models not yet downloaded are skipped
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "tiny,base,small,medium,turbo-q5,turbo,large-v3"
    )]
    models: Vec<String>,
    /// Spoken language code [default: auto-detect]
    #[arg(long, short)]
    language: Option<String>,
    /// Also run every model on the CPU only, for comparison
    #[arg(long)]
    with_cpu: bool,
}

pub fn run(args: Args, models_dir: &Path) -> anyhow::Result<()> {
    let pcm = quill_audio::load_for_stt(&args.file)
        .with_context(|| format!("reading {}", args.file.display()))?;
    let audio_secs = pcm.len() as f64 / f64::from(STT_SAMPLE_RATE);
    let opts = TranscribeOptions {
        language: args.language,
        ..Default::default()
    };
    // A few seconds of audio to warm up each model before timing, so one-off
    // GPU setup (Metal pipeline compilation, buffer allocation) isn't counted.
    let warmup = &pcm[..pcm.len().min(STT_SAMPLE_RATE as usize * 3)];

    let backends: &[bool] = if args.with_cpu {
        &[true, false]
    } else {
        &[true]
    };

    println!("Audio: {} ({audio_secs:.1}s)\n", args.file.display());
    println!("| Model | Backend | Load (s) | Transcribe (s) | × real time | Text (start) |");
    println!("|---|---|---:|---:|---:|---|");
    for name in &args.models {
        let model_path = match crate::models::resolve(name, models_dir) {
            Ok(path) => path,
            Err(e) => {
                eprintln!("skipping {name}: {e}");
                continue;
            }
        };
        for &use_gpu in backends {
            let config = WhisperConfig {
                use_gpu,
                ..WhisperConfig::default()
            };
            let started = Instant::now();
            let transcriber = WhisperTranscriber::load(&model_path, &config)?;
            let load_secs = started.elapsed().as_secs_f64();

            transcriber.transcribe(warmup, &opts)?;
            let started = Instant::now();
            let segments = transcriber.transcribe(&pcm, &opts)?;
            let stt_secs = started.elapsed().as_secs_f64();

            let text = crate::transcribe::plain_text(&segments);
            let preview: String = text.chars().take(60).collect();
            println!(
                "| {name} | {} | {load_secs:.2} | {stt_secs:.2} | {:.1} | {preview}… |",
                if use_gpu { gpu_label() } else { "CPU" },
                audio_secs / stt_secs.max(f64::EPSILON),
            );
        }
    }
    Ok(())
}

fn gpu_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Metal"
    } else {
        "GPU"
    }
}
