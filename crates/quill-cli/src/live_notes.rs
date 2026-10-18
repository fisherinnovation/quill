//! `quill-cli live-notes`: replays a recording through live notes (what the
//! panel shows during a meeting), to see what the notes say and when, and
//! what they cost. A recording folder plays as You (mic) and Others (call);
//! a single audio file plays as Others.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Context;
use quill_audio::STT_SAMPLE_RATE;
use quill_core::live_meeting::{self, LiveTranscript, Notes};
use quill_core::meeting::{MIC_FILE, SYSTEM_FILE};
use quill_core::settings::DEFAULT_SUMMARY_MODEL;
use quill_llm::Privacy;
use quill_stt::{TranscribeOptions, WhisperConfig, WhisperTranscriber};

#[derive(Debug, clap::Args)]
pub struct Args {
    /// A recording folder (mic and system tracks) or one audio file (the call)
    path: PathBuf,
    /// Notes model (OpenRouter id or `ollama:<model>`)
    #[arg(long, short, default_value = DEFAULT_SUMMARY_MODEL)]
    model: String,
    /// Whisper model
    #[arg(long, default_value = "turbo")]
    stt_model: String,
    /// Update the notes every this many seconds of audio
    #[arg(long, default_value_t = 60)]
    every_secs: u64,
    /// Spoken language code [default: detect]
    #[arg(long, short)]
    language: Option<String>,
}

pub fn run(args: Args, models_dir: &Path) -> anyhow::Result<()> {
    let (mic, system) = tracks(&args.path)?;
    let model_path = crate::models::resolve(&args.stt_model, models_dir)?;
    let config = WhisperConfig {
        vad_model: crate::models::vad_model(true, models_dir)?,
        ..WhisperConfig::default()
    };
    let transcriber = WhisperTranscriber::load(&model_path, &config)?;
    let (provider, model) = quill_llm::provider_for(&args.model, Privacy::default())?;
    let runtime = tokio::runtime::Runtime::new()?;
    let opts = TranscribeOptions {
        language: args.language.clone(),
        ..TranscribeOptions::default()
    };

    let rate = STT_SAMPLE_RATE as usize;
    let slice = 2 * rate;
    let len = mic.len().max(system.len());
    let at = |v: &[f32], i: usize| {
        v.get(i..(i + slice).min(len))
            .map_or_else(|| vec![0.0; (i + slice).min(len) - i], <[f32]>::to_vec)
    };
    let mut live = LiveTranscript::new();
    let mut notes = Notes::default();
    let (mut transcribe_ms, mut cost) = (0_u128, 0.0_f64);
    let mut next_update = args.every_secs as usize * rate;
    let mut i = 0;
    while i < len {
        live.push(&at(&mic, i), &at(&system, i));
        i = (i + slice).min(len);
        let end = i >= len;
        let started = Instant::now();
        live.step(&transcriber, &opts, end)?;
        transcribe_ms += started.elapsed().as_millis();
        if i >= next_update || end {
            next_update += args.every_secs as usize * rate;
            let text = live.recent_text();
            let started = Instant::now();
            let (updated, call_cost) = runtime.block_on(live_meeting::update(
                provider.as_ref(),
                &model,
                &quill_llm::prompts::LIVE,
                live_meeting::Update {
                    previous: &notes,
                    dismissed: &[],
                    ticked: &[],
                    transcript: &text,
                    minutes: (i / rate / 60) as u64,
                },
            ))?;
            cost += call_cost.unwrap_or(0.0);
            notes = updated;
            print_notes(i / rate, started.elapsed().as_millis(), &notes);
        }
    }
    println!(
        "\n{} lines transcribed in {:.1} s for {:.1} min of audio; notes cost ${cost:.4}",
        live.lines().len(),
        transcribe_ms as f64 / 1000.0,
        len as f64 / rate as f64 / 60.0
    );
    Ok(())
}

/// (mic, system) as 16 kHz mono: both from a recording folder, or a file as
/// the call with a silent mic.
fn tracks(path: &Path) -> anyhow::Result<(Vec<f32>, Vec<f32>)> {
    let load = |p: PathBuf| {
        quill_audio::load_for_stt(&p).with_context(|| format!("reading {}", p.display()))
    };
    if path.is_dir() {
        let find = |wav: &str| {
            let flac = Path::new(wav).with_extension("flac");
            [path.join(flac), path.join(wav)]
                .into_iter()
                .find(|p| p.exists())
        };
        let mic = find(MIC_FILE).map(load).transpose()?.unwrap_or_default();
        let system = find(SYSTEM_FILE).map(load).transpose()?.unwrap_or_default();
        return Ok((mic, system));
    }
    let system = load(path.to_path_buf())?;
    Ok((vec![0.0; system.len()], system))
}

fn print_notes(secs: usize, latency_ms: u128, notes: &Notes) {
    println!(
        "\n## At {}:{:02} ({latency_ms} ms)\n\n{}",
        secs / 60,
        secs % 60,
        notes.gist
    );
    for (kind, title) in [
        (live_meeting::Kind::Ask, "Ask before the end"),
        (live_meeting::Kind::FollowUp, "Follow-ups"),
        (live_meeting::Kind::Decision, "Decisions"),
    ] {
        let items: Vec<_> = notes.items.iter().filter(|i| i.kind == kind).collect();
        if items.is_empty() {
            continue;
        }
        println!("\n{title}:");
        for item in items {
            let who = item
                .who
                .as_deref()
                .map_or(String::new(), |w| format!("{w}: "));
            println!(
                "- [{}] {} {who}{}",
                if item.done { "x" } else { " " },
                item.id,
                item.text
            );
        }
    }
}
