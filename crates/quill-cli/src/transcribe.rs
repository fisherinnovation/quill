//! `quill-cli transcribe`: WAV file (or a `record` directory) in,
//! timestamped transcript out.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context;
use clap::ValueEnum;
use quill_audio::STT_SAMPLE_RATE;
use quill_stt::{Segment, TranscribeOptions, Transcriber, WhisperConfig, WhisperTranscriber};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// WAV file (any sample rate / channel count), or a directory from
    /// `quill-cli record` to get a merged "Me" / "Others" transcript
    file: PathBuf,
    /// Model name (see `quill-cli models list`)
    #[arg(long, short, default_value = "turbo")]
    model: String,
    /// Use this model file instead of a named model
    #[arg(long, conflicts_with = "model")]
    model_path: Option<PathBuf>,
    /// Spoken language code, e.g. `en` [default: auto-detect]
    #[arg(long, short)]
    language: Option<String>,
    /// Prime the decoder with vocabulary or names, e.g. "Quill, OpenRouter"
    #[arg(long)]
    prompt: Option<String>,
    /// Output format
    #[arg(long, short, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// Run on the CPU only, even if a GPU backend is available
    #[arg(long)]
    cpu: bool,
    /// Transcribe silence too, instead of skipping it with Silero VAD
    #[arg(long)]
    no_vad: bool,
    /// Recording directories: don't remove speaker echo from the mic track
    #[arg(long)]
    no_aec: bool,
    /// Recording directories: split "Others" into Speaker 1, 2, … by voice
    #[arg(long)]
    diarize: bool,
    /// With --diarize: how many other people were on the call, if known
    #[arg(long, requires = "diarize")]
    speakers: Option<usize>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    /// `[mm:ss.mmm --> mm:ss.mmm]  text`, one segment per line
    Text,
    /// JSON array of segments (input for `summarize` in M3)
    Json,
}

pub fn run(args: Args, models_dir: &Path) -> anyhow::Result<()> {
    let model_path = match args.model_path {
        Some(path) => path,
        None => crate::models::resolve(&args.model, models_dir)?,
    };

    let config = WhisperConfig {
        use_gpu: !args.cpu,
        vad_model: crate::models::vad_model(!args.no_vad, models_dir)?,
        ..WhisperConfig::default()
    };
    let started = Instant::now();
    let transcriber = WhisperTranscriber::load(&model_path, &config)
        .with_context(|| format!("loading {}", model_path.display()))?;
    let load_secs = started.elapsed().as_secs_f64();

    let opts = TranscribeOptions {
        language: args.language,
        initial_prompt: args.prompt,
        ..Default::default()
    };
    let started = Instant::now();
    let (segments, audio_secs) = if args.file.is_dir() {
        let segments = quill_core::meeting::transcribe_recording(
            &args.file,
            &transcriber,
            &opts,
            !args.no_aec,
        )
        .with_context(|| format!("transcribing recording {}", args.file.display()))?;
        // Kept as the two-track ("Me" / "Others") transcript: `summarize
        // <dir>` reads it, and speakers are split again from it.
        quill_core::speakers::save_track_transcript(&args.file, &segments)
            .context("saving transcript.json")?;
        let mut segments = segments;
        if args.diarize {
            let found =
                crate::diarize::label_others(&args.file, &mut segments, args.speakers, models_dir)?;
            eprintln!("diarization: {found} speakers on the call");
        }
        (segments, None)
    } else {
        let pcm = quill_audio::load_for_stt(&args.file)
            .with_context(|| format!("reading {}", args.file.display()))?;
        let audio_secs = pcm.len() as f64 / f64::from(STT_SAMPLE_RATE);
        (transcriber.transcribe(&pcm, &opts)?, Some(audio_secs))
    };
    let stt_secs = started.elapsed().as_secs_f64();

    // A recording directory in the library gets the transcript too.
    if args.file.is_dir()
        && let Some((library, id)) = crate::library::meeting_for(&args.file)?
    {
        library.save_transcript(&id, &segments)?;
    }

    match args.format {
        Format::Text => {
            for s in &segments {
                let speaker = s.speaker.as_deref().map(|sp| format!("{sp}: "));
                println!(
                    "[{} --> {}]  {}{}",
                    timestamp(s.start_ms),
                    timestamp(s.end_ms),
                    speaker.unwrap_or_default(),
                    s.text
                );
            }
        }
        Format::Json => println!("{}", serde_json::to_string_pretty(&segments)?),
    }

    // Stats go to stderr so `> transcript.json` captures only the transcript.
    let cpu = if args.cpu { " (CPU)" } else { "" };
    match audio_secs {
        Some(audio_secs) => eprintln!(
            "\n{audio_secs:.1}s audio · model load {load_secs:.2}s · transcribe {stt_secs:.2}s · \
             {:.1}× real time{cpu}",
            audio_secs / stt_secs.max(f64::EPSILON),
        ),
        None => eprintln!("\nmodel load {load_secs:.2}s · transcribe {stt_secs:.2}s{cpu}"),
    }
    Ok(())
}

/// Formats milliseconds as `mm:ss.mmm`, or `h:mm:ss.mmm` past an hour.
fn timestamp(ms: u64) -> String {
    let (h, m, s, ms) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{ms:03}")
    } else {
        format!("{m:02}:{s:02}.{ms:03}")
    }
}

/// Joins segment text into one string (used by `bench` for a preview).
pub fn plain_text(segments: &[Segment]) -> String {
    segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::timestamp;

    #[test]
    fn formats_timestamps() {
        assert_eq!(timestamp(0), "00:00.000");
        assert_eq!(timestamp(61_234), "01:01.234");
        assert_eq!(timestamp(3_723_004), "1:02:03.004");
    }
}
