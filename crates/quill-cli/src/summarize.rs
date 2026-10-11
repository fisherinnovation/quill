//! `quill-cli summarize`: transcript in, meeting notes out.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use quill_core::summary::{self, Summary};
use quill_llm::Privacy;
use quill_stt::Segment;
use serde_json::json;

use quill_core::settings::DEFAULT_SUMMARY_MODEL;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Transcript JSON (`transcribe -f json`), or a recording directory
    /// containing transcript.json; summary.md is then saved next to it
    input: PathBuf,
    /// OpenRouter model id, or `ollama:<model>` for a local model
    #[arg(long, short, default_value = DEFAULT_SUMMARY_MODEL)]
    model: String,
    /// Date for the title [default: the recording's date, else today]
    #[arg(long)]
    date: Option<String>,
    /// OpenRouter: only use zero-data-retention endpoints (fewer models)
    #[arg(long)]
    zdr: bool,
    /// OpenRouter: allow providers that may store or train on the transcript
    #[arg(long)]
    allow_data_collection: bool,
}

pub fn run(args: Args) -> anyhow::Result<()> {
    let (transcript_path, out_dir) = if args.input.is_dir() {
        (args.input.join("transcript.json"), Some(args.input.clone()))
    } else {
        (args.input.clone(), None)
    };
    let segments = read_transcript(&transcript_path)?;
    let date = match args.date {
        Some(date) => date,
        None => {
            file_date(&transcript_path).unwrap_or_else(|| jiff::Zoned::now().date().to_string())
        }
    };

    let privacy = Privacy {
        deny_data_collection: !args.allow_data_collection,
        zero_data_retention: args.zdr,
    };
    let (provider, model) = quill_llm::provider_for(&args.model, privacy)?;
    if !args.model.starts_with("ollama:") {
        // Make it obvious when the transcript leaves the machine.
        eprintln!("Sending transcript to OpenRouter ({model})…");
    }

    // The library is async (HTTP); the CLI is not, so run this one command
    // on a Tokio runtime and wait for it here.
    let runtime = tokio::runtime::Runtime::new()?;
    let summary = runtime.block_on(summary::summarize(
        provider.as_ref(),
        &model,
        &quill_llm::prompts::SUMMARY,
        &segments,
        &date,
    ))?;

    println!("{}", summary.markdown);
    eprintln!("\n{}", stats(&summary));
    if let Some(dir) = out_dir {
        save(&dir, &summary, &args.model)?;
        if let Some((library, id)) = crate::library::meeting_for(&dir)? {
            library.save_summary(&id, &summary)?;
        }
        eprintln!("Saved {}", dir.join("summary.md").display());
    }
    Ok(())
}

fn read_transcript(path: &Path) -> anyhow::Result<Vec<Segment>> {
    if !path.is_file() {
        bail!(
            "{} not found; create it with `quill-cli transcribe <recording dir>`",
            path.display()
        );
    }
    let json =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&json).with_context(|| format!("parsing {}", path.display()))
}

/// Local date the file was last modified, e.g. `2026-10-11`.
fn file_date(path: &Path) -> Option<String> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let ts = jiff::Timestamp::try_from(modified).ok()?;
    Some(ts.to_zoned(jiff::tz::TimeZone::system()).date().to_string())
}

fn stats(s: &Summary) -> String {
    let cost = s
        .cost_usd
        .map_or("cost unknown".into(), |c| format!("${c:.4}"));
    format!(
        "{} · {} · {} request{} · {} in / {} out tokens · {} · {:.1}s",
        s.model,
        s.prompt_version,
        s.requests,
        if s.requests == 1 { "" } else { "s" },
        s.prompt_tokens,
        s.completion_tokens,
        cost,
        s.latency_ms as f64 / 1000.0
    )
}

/// Writes summary.md and summary.json (what produced it, for later
/// comparison) into the recording directory.
fn save(dir: &Path, s: &Summary, requested_model: &str) -> anyhow::Result<()> {
    std::fs::write(dir.join("summary.md"), format!("{}\n", s.markdown))?;
    let meta = json!({
        "requested_model": requested_model,
        "model": s.model,
        "prompt_version": s.prompt_version,
        "requests": s.requests,
        "prompt_tokens": s.prompt_tokens,
        "completion_tokens": s.completion_tokens,
        "cost_usd": s.cost_usd,
        "latency_ms": s.latency_ms,
        "created_at": jiff::Timestamp::now().to_string(),
    });
    std::fs::write(
        dir.join("summary.json"),
        serde_json::to_string_pretty(&meta)?,
    )?;
    Ok(())
}
