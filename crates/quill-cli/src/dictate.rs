//! `quill-cli dictate`: run the dictation pipeline on WAV or FLAC files and
//! show what each step produced and how long it took, to tune latency and
//! the polish prompt (see `eval/dictation/`). A clip with a reference
//! (`clip.txt`: what should have been typed) is also scored by word error
//! rate, and a saved sample's `clip.json` supplies the app it went into.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context;
use quill_audio::STT_SAMPLE_RATE;
use quill_core::settings::{DEFAULT_POLISH_MODEL, DictationSettings, Style};
use quill_core::{dictation, samples};
use quill_eval::wer::{self, Tally};
use quill_llm::Privacy;
use quill_stt::{Transcriber, WhisperConfig};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// WAV or FLAC files, each one utterance
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// Speech model: a Whisper size or a Parakeet model (`parakeet-v2`)
    #[arg(long, default_value = "turbo")]
    stt_model: String,
    /// CPU threads for the speech model [default: up to 8]
    #[arg(long)]
    threads: Option<usize>,
    /// Run Whisper on the CPU only, even if a GPU backend is available
    /// (Parakeet always runs on the CPU)
    #[arg(long)]
    cpu: bool,
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
    /// What the speaker says about themselves, for the clean-up model
    /// (the app's Dictation → About you)
    #[arg(long, default_value = "")]
    about: String,
    /// Pretend the text is going into this app (by bundle id or .exe), to
    /// try its style, e.g. com.tinyspeck.slackmacgap [default: the app in a
    /// saved sample's clip.json]
    #[arg(long)]
    app: Option<String>,
    /// Replay each clip through live dictation (captions while speaking):
    /// passes every --step-ms of audio, then the final pass on release.
    /// Compares the text and the wait after release with a single pass.
    #[arg(long)]
    stream: bool,
    /// With --stream, how much audio arrives between passes
    #[arg(long, default_value_t = 600)]
    step_ms: u64,
}

pub fn run(args: Args, models_dir: &Path) -> anyhow::Result<()> {
    crate::models::require_speech(&args.stt_model, models_dir)?;
    let mut config = WhisperConfig {
        use_gpu: !args.cpu,
        vad_model: crate::models::vad_model(true, models_dir)?,
        ..WhisperConfig::default()
    };
    if let Some(threads) = args.threads {
        config.threads = threads;
    }
    let transcriber = quill_stt::load(&args.stt_model, models_dir, &config)?;
    let transcriber = transcriber.as_ref();
    let settings = DictationSettings {
        vocabulary: args.vocabulary.clone(),
        about: args.about.clone(),
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
        dictation::transcribe(transcriber, &pcm, &settings)?;
    }

    if args.stream {
        return stream(&args, transcriber, &settings);
    }

    let mut whisper_total = Tally::default();
    // (clip seconds, ms to transcribe), for the summary.
    let mut stt_times = Vec::new();
    let mut typed_totals = vec![Tally::default(); providers.len()];
    let mut unscored = 0;
    for file in &args.files {
        let pcm = quill_audio::load_for_stt(file)
            .with_context(|| format!("reading {}", file.display()))?;
        let reference = samples::reference(file)
            .with_context(|| format!("reading the reference for {}", file.display()))?;
        let meta = samples::meta(file)
            .with_context(|| format!("reading {}", file.with_extension("json").display()))?
            .unwrap_or_default();
        // The app it was dictated into, unless --app says otherwise.
        let (app_id, app_name) = match &args.app {
            Some(app) => (Some(app.clone()), app.clone()),
            None => (meta.app_id.clone(), meta.app.clone().unwrap_or_default()),
        };
        let destination = app_id.as_deref().map(|id| dictation::Destination {
            app: if app_name.is_empty() { id } else { &app_name },
            style: dictation::style_for(id, &settings),
        });
        let style = destination.map_or(Style::Default, |d| d.style);
        // Clips don't record the kind of text box they went into.
        let field = None;

        let secs = pcm.len() as f64 / f64::from(STT_SAMPLE_RATE);
        let started = Instant::now();
        let raw = dictation::transcribe(transcriber, &pcm, &settings)?;
        let stt_ms = started.elapsed().as_millis();
        stt_times.push((secs, stt_ms));
        println!("\n## {} ({secs:.1} s)", display_name(file));
        println!("- {} {stt_ms} ms: {raw}", args.stt_model);
        match &reference {
            Some(reference) => whisper_total += score(reference, &raw),
            None => unscored += 1,
        }

        if !args.always_polish && !dictation::needs_polish(&raw) {
            println!("- (nothing to polish)");
            // What's typed is Whisper's text, for every model.
            if let Some(reference) = &reference {
                let tally =
                    wer::align(reference, &dictation::apply_style(&raw, style, field)).tally();
                for total in &mut typed_totals {
                    *total += tally;
                }
            }
            continue;
        }
        for ((provider, model), total) in providers.iter().zip(&mut typed_totals) {
            let result = runtime.block_on(dictation::polish(
                provider.as_ref(),
                model,
                &quill_llm::prompts::DICTATION,
                &raw,
                &settings,
                destination,
            ));
            // The app types the transcript when polishing fails.
            let typed = match result {
                Ok(p) => {
                    let text = dictation::apply_style(&p.text, style, field);
                    let cost = p.cost_usd.map_or(String::new(), |c| format!(", ${c:.5}"));
                    let rejected = p
                        .rejected
                        .map_or(String::new(), |r| format!(" [REJECTED: {r}]"));
                    println!(
                        "- {model} {} ms{cost}{rejected}:\n\n{}\n",
                        p.latency_ms,
                        indent(&text)
                    );
                    text
                }
                Err(e) => {
                    println!("- {model}: error: {e}");
                    dictation::apply_style(&raw, style, field)
                }
            };
            if let Some(reference) = &reference {
                *total += score(reference, &typed);
            }
        }
    }

    let scored = args.files.len() - unscored;
    if scored > 0 {
        println!(
            "\n## Accuracy: {scored} clips, {} reference words",
            whisper_total.reference_words
        );
        println!("\n| Typed by | WER | Errors (sub / del / ins) |\n|---|---:|---:|");
        print_row(&format!("{} only", args.stt_model), whisper_total);
        for ((_, model), total) in providers.iter().zip(&typed_totals) {
            print_row(&format!("{model} when needed"), *total);
        }
    }
    print_times(&args.stt_model, &mut stt_times);
    if unscored > 0 {
        println!(
            "\n{unscored} of {} clips have no reference (clip.txt beside clip.wav), so weren't scored.",
            args.files.len()
        );
    }
    Ok(())
}

/// `--stream`: each clip through live dictation and through a single pass.
fn stream(
    args: &Args,
    transcriber: &dyn Transcriber,
    settings: &DictationSettings,
) -> anyhow::Result<()> {
    let step = (args.step_ms as usize * STT_SAMPLE_RATE as usize / 1000).max(1);
    let (mut single_total, mut live_total) = (Tally::default(), Tally::default());
    let (mut same, mut single_waits, mut live_waits, mut pass_times) =
        (0, Vec::new(), Vec::new(), Vec::new());
    for file in &args.files {
        let pcm = quill_audio::load_for_stt(file)
            .with_context(|| format!("reading {}", file.display()))?;
        let reference = samples::reference(file)?;
        println!(
            "\n## {} ({:.1} s)",
            display_name(file),
            pcm.len() as f64 / f64::from(STT_SAMPLE_RATE)
        );

        let started = Instant::now();
        let single = dictation::transcribe(transcriber, &pcm, settings)?;
        single_waits.push(started.elapsed().as_millis());

        let mut live = quill_core::live::LiveTranscript::new();
        let mut end = step;
        while end < pcm.len() {
            let started = Instant::now();
            live.update(transcriber, &pcm[..end], settings)?;
            pass_times.push(started.elapsed().as_millis());
            end += step;
        }
        let started = Instant::now();
        let streamed = live.finish(transcriber, &pcm, settings)?;
        let wait = started.elapsed().as_millis();
        live_waits.push(wait);
        let (passes, locked_ms) = live.progress();

        println!(
            "- single pass {} ms: {single}",
            single_waits.last().unwrap_or(&0)
        );
        println!(
            "- live, {passes} passes, {:.1} s locked in, {wait} ms after release: {streamed}",
            locked_ms as f64 / 1000.0
        );
        if streamed == single {
            same += 1;
            println!("  - same text");
        } else {
            println!(
                "  - differs: {}",
                wer::align(&single, &streamed).errors().join(" ")
            );
        }
        if let Some(reference) = &reference {
            single_total += wer::align(reference, &single).tally();
            live_total += wer::align(reference, &streamed).tally();
        }
    }

    let median = |v: &mut Vec<u128>| {
        v.sort_unstable();
        v.get(v.len() / 2).copied().unwrap_or(0)
    };
    let max = |v: &[u128]| v.iter().max().copied().unwrap_or(0);
    println!(
        "\n## Live dictation: {} clips, a pass every {} ms of audio",
        args.files.len(),
        args.step_ms
    );
    println!(
        "\n- Same text as a single pass: {same} of {}",
        args.files.len()
    );
    println!(
        "- Wait after release: single pass median {} ms (max {}), live median {} ms (max {})",
        median(&mut single_waits.clone()),
        max(&single_waits),
        median(&mut live_waits.clone()),
        max(&live_waits)
    );
    println!(
        "- Passes while speaking: median {} ms, max {} ms (each must finish within the {} ms step)",
        median(&mut pass_times.clone()),
        max(&pass_times),
        args.step_ms
    );
    if single_total.reference_words > 0 {
        println!(
            "- WER against references ({} words): single pass {:.1}%, live {:.1}%",
            single_total.reference_words,
            single_total.wer() * 100.0,
            live_total.wer() * 100.0
        );
    }
    Ok(())
}

/// Median and slowest transcription times, by clip length.
fn print_times(model: &str, times: &mut [(f64, u128)]) {
    if times.is_empty() {
        return;
    }
    println!("\n## Speed ({model})\n\n| Clips | Count | Median | Max |\n|---|---:|---:|---:|");
    for (label, lo, hi) in [
        ("under 4 s", 0.0, 4.0),
        ("4–15 s", 4.0, 15.0),
        ("over 15 s", 15.0, f64::MAX),
        ("all", 0.0, f64::MAX),
    ] {
        let mut ms: Vec<u128> = times
            .iter()
            .filter(|(secs, _)| *secs >= lo && *secs < hi)
            .map(|(_, ms)| *ms)
            .collect();
        if ms.is_empty() {
            continue;
        }
        ms.sort_unstable();
        println!(
            "| {label} | {} | {} ms | {} ms |",
            ms.len(),
            ms[ms.len() / 2],
            ms[ms.len() - 1]
        );
    }
}

/// Prints how far `text` is from `reference` under the step it scores.
fn score(reference: &str, text: &str) -> Tally {
    let alignment = wer::align(reference, text);
    let tally = alignment.tally();
    if tally.errors() == 0 {
        println!("  - matches the reference");
    } else {
        let errors = tally.errors();
        println!(
            "  - {errors} error{} in {} words ({:.0}%): {}",
            if errors == 1 { "" } else { "s" },
            tally.reference_words,
            tally.wer() * 100.0,
            alignment.errors().join(" ")
        );
    }
    tally
}

fn print_row(step: &str, t: Tally) {
    println!(
        "| {step} | {:.1}% | {} ({} / {} / {}) |",
        t.wer() * 100.0,
        t.errors(),
        t.substitutions,
        t.deletions,
        t.insertions
    );
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
