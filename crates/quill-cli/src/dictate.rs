//! `quill-cli dictate`: run the dictation pipeline on WAV files and show what
//! each step produced and how long it took, to tune latency and the polish
//! prompt (see `eval/dictation/`).

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context;
use quill_audio::STT_SAMPLE_RATE;
use quill_core::dictation;
use quill_core::settings::{DEFAULT_POLISH_MODEL, DictationSettings, Style};
use quill_llm::Privacy;
use quill_stt::{WhisperConfig, WhisperTranscriber};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// WAV files, each one utterance
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// Whisper model
    #[arg(long, default_value = "turbo")]
    stt_model: String,
    /// Spoken language code, or "auto" [default: the system language]
    #[arg(long, short)]
    language: Option<String>,
    /// Polish models to compare, comma-separated (OpenRouter ids or
    /// `ollama:<model>`)
    #[arg(long, value_delimiter = ',', default_value = DEFAULT_POLISH_MODEL)]
    polish_models: Vec<String>,
    /// Skip the polish step
    #[arg(long)]
    no_polish: bool,
    /// Polish even utterances below the minimum length
    #[arg(long)]
    always_polish: bool,
    /// Terms to spell exactly, comma-separated
    #[arg(long, value_delimiter = ',')]
    vocabulary: Vec<String>,
    /// Pretend the text is going into this app (by bundle id or .exe), to
    /// try its style, e.g. com.tinyspeck.slackmacgap
    #[arg(long)]
    app: Option<String>,
}

pub fn run(args: Args, models_dir: &Path) -> anyhow::Result<()> {
    let model_path = crate::models::resolve(&args.stt_model, models_dir)?;
    let config = WhisperConfig {
        vad_model: crate::models::vad_model(true, models_dir)?,
        ..WhisperConfig::default()
    };
    let transcriber = WhisperTranscriber::load(&model_path, &config)?;
    let settings = DictationSettings {
        vocabulary: args.vocabulary.clone(),
        language: args.language.clone(),
        ..DictationSettings::default()
    };
    let providers = if args.no_polish {
        Vec::new()
    } else {
        args.polish_models
            .iter()
            .map(|spec| {
                let (provider, model) = quill_llm::provider_for(spec, Privacy::default())?;
                Ok((provider, model))
            })
            .collect::<anyhow::Result<Vec<_>>>()?
    };
    let runtime = tokio::runtime::Runtime::new()?;

    // The first transcription pays for GPU setup; a resident model in the
    // app pays it once at startup, so don't count it.
    if let Some(first) = args.files.first() {
        let pcm = quill_audio::load_for_stt(first)?;
        dictation::transcribe(&transcriber, &pcm, &settings)?;
    }

    let destination = args.app.as_deref().map(|app| dictation::Destination {
        app,
        style: dictation::style_for(app, &settings),
    });
    for file in &args.files {
        let pcm = quill_audio::load_for_stt(file)
            .with_context(|| format!("reading {}", file.display()))?;
        let secs = pcm.len() as f64 / f64::from(STT_SAMPLE_RATE);
        let started = Instant::now();
        let raw = dictation::transcribe(&transcriber, &pcm, &settings)?;
        let stt_ms = started.elapsed().as_millis();
        println!("\n## {} ({secs:.1} s)", display_name(file));
        println!("- whisper {stt_ms} ms: {raw}");

        if !args.always_polish && !dictation::needs_polish(&raw) {
            println!("- (nothing to polish)");
            continue;
        }
        for (provider, model) in &providers {
            let result = runtime.block_on(dictation::polish(
                provider.as_ref(),
                model,
                &quill_llm::prompts::DICTATION,
                &raw,
                &settings,
                destination,
            ));
            match result {
                Ok(p) => {
                    let p = dictation::Polish {
                        text: dictation::apply_style(
                            &p.text,
                            destination.map_or(Style::Default, |d| d.style),
                        ),
                        ..p
                    };
                    let cost = p.cost_usd.map_or(String::new(), |c| format!(", ${c:.5}"));
                    let rejected = p
                        .rejected
                        .map_or(String::new(), |r| format!(" [REJECTED: {r}]"));
                    println!(
                        "- {model} {} ms{cost}{rejected}:\n\n{}\n",
                        p.latency_ms,
                        indent(&p.text)
                    );
                }
                Err(e) => println!("- {model}: error: {e}"),
            }
        }
    }
    Ok(())
}

fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Indents every line so multi-line (list) output stays inside its bullet.
fn indent(text: &str) -> String {
    text.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}
