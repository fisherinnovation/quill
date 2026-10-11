//! `quill-cli eval`: which model makes the best notes for the money?

use std::path::PathBuf;

use anyhow::{Context, bail};
use quill_eval::{judge, report, store};
use quill_llm::Privacy;
use quill_llm::prompts::{self, SummaryPrompts};

/// Strong judge by default; judging quality matters more than its cost
/// (one request per transcript, not per model).
const DEFAULT_JUDGE: &str = "anthropic/claude-opus-5.5";

#[derive(Debug, clap::Args)]
pub struct Args {
    /// Comma-separated models to compare (OpenRouter ids and/or `ollama:<model>`)
    #[arg(long, short, value_delimiter = ',', required = true)]
    models: Vec<String>,
    /// Directory of transcripts (*.json, plus private/*.json)
    #[arg(long, default_value = "eval/transcripts")]
    transcripts: PathBuf,
    /// Comma-separated summary prompt versions
    #[arg(long, value_delimiter = ',', default_value = prompts::SUMMARY.version)]
    prompts: Vec<String>,
    /// Model that scores the summaries
    #[arg(long, default_value = DEFAULT_JUDGE, conflicts_with = "no_judge")]
    judge: String,
    /// Skip judging (just cost/latency and the side-by-side review)
    #[arg(long)]
    no_judge: bool,
    /// Summaries to run at once
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
    /// Where to write results (eval.sqlite accumulates across runs)
    #[arg(long, default_value = "eval/results")]
    out: PathBuf,
    /// OpenRouter: only use zero-data-retention endpoints
    #[arg(long)]
    zdr: bool,
}

pub fn run(args: Args) -> anyhow::Result<()> {
    let cases = quill_eval::load_cases(&args.transcripts)?;
    let prompt_sets = args
        .prompts
        .iter()
        .map(|v| {
            prompts::find_summary(v).with_context(|| {
                let known: Vec<_> = prompts::SUMMARY_VERSIONS
                    .iter()
                    .map(|p| p.version)
                    .collect();
                format!("unknown prompt version `{v}`; known: {}", known.join(", "))
            })
        })
        .collect::<anyhow::Result<Vec<&'static SummaryPrompts>>>()?;
    let privacy = Privacy {
        deny_data_collection: true,
        zero_data_retention: args.zdr,
    };
    let factory = |spec: &str| quill_llm::provider_for(spec, privacy);

    let total = cases.len() * args.models.len() * prompt_sets.len();
    let hours: f64 = cases.iter().map(quill_eval::Case::hours).sum();
    eprintln!(
        "{} transcripts ({hours:.1} h) × {} models × {} prompts = {total} summaries",
        cases.len(),
        args.models.len(),
        prompt_sets.len()
    );

    let started = jiff::Timestamp::now();
    let date = jiff::Zoned::now().date().to_string();
    let runtime = tokio::runtime::Runtime::new()?;
    let mut results = runtime.block_on(quill_eval::run_summaries(
        &cases,
        &args.models,
        &prompt_sets,
        &date,
        &factory,
        args.concurrency,
    ));
    // Stable order for the files: by case, then model, then prompt.
    results.sort_by(|a, b| {
        (a.case.as_str(), a.model.as_str(), a.prompt_version).cmp(&(
            b.case.as_str(),
            b.model.as_str(),
            b.prompt_version,
        ))
    });
    let failed = results.iter().filter(|r| r.outcome.is_err()).count();
    if failed == results.len() {
        for r in &results {
            eprintln!(
                "  {} / {}: {}",
                r.case,
                r.model,
                r.outcome.as_ref().err().map_or("", |e| e)
            );
        }
        bail!("every summary failed");
    }

    let mut judge_usage = None;
    if !args.no_judge {
        let (provider, model) = quill_llm::provider_for(&args.judge, privacy)?;
        eprintln!("Judging with {model}…");
        let seed = started.as_nanosecond().unsigned_abs() as u64;
        judge_usage = Some(runtime.block_on(judge::judge_all(
            provider.as_ref(),
            &model,
            &cases,
            &mut results,
            seed,
        )));
    }

    // Outputs: a folder per run, plus the shared database.
    let run_dir = args.out.join(started.strftime("%Y%m%d-%H%M%S").to_string());
    std::fs::create_dir_all(&run_dir).with_context(|| format!("creating {}", run_dir.display()))?;
    let meta = store::RunMeta {
        created_at: started.to_string(),
        judge_model: (!args.no_judge).then(|| args.judge.clone()),
        judge_prompt: (!args.no_judge).then_some(prompts::JUDGE.version),
        judge_cost_usd: judge_usage.as_ref().and_then(|u| u.cost_usd),
    };
    let run_id = store::save_sqlite(&args.out.join("eval.sqlite"), &meta, &results)?;
    store::save_csv(&run_dir.join("results.csv"), &results)?;
    let rows = report::leaderboard(&results);
    let table = report::leaderboard_markdown(&rows);
    std::fs::write(run_dir.join("leaderboard.md"), &table)?;
    std::fs::write(run_dir.join("review.md"), report::review_markdown(&results))?;

    println!("{table}");
    if failed > 0 {
        eprintln!(
            "{failed} of {} summaries failed (see review.md).",
            results.len()
        );
    }
    if let Some(u) = &judge_usage {
        let cost = u.cost_usd.map_or("?".into(), |c| format!("${c:.4}"));
        eprintln!("Judge: {} requests, {cost}.", u.requests);
        for f in &u.failures {
            eprintln!("  judge failed on {f}");
        }
    }
    eprintln!("Run {run_id} saved to {}", run_dir.display());
    Ok(())
}
