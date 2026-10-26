//! `quill-cli search`: search the meeting library by words and meaning
//! (transcripts, or `--everything`: titles, notes, action items and
//! dictations too), index it for search by meaning, or measure search on
//! labelled transcripts (`--eval eval/search/queries.json`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, bail};
use quill_core::library::{Library, SearchHit};
use quill_core::search::{self, Scope};
use quill_core::semantic::{self, LabelledQuery, Matched};
use quill_embed::Embedder;
use quill_stt::Segment;

#[derive(Debug, clap::Args)]
pub struct Args {
    /// What to search for
    query: Option<String>,
    /// Search by words only, as Quill does without the search model
    #[arg(long)]
    words_only: bool,
    /// Search everything, as the app's search window does: meeting titles,
    /// moments in transcripts, notes, action items and dictations (up to
    /// --limit of each)
    #[arg(long)]
    everything: bool,
    /// First index what isn't yet indexed for search by meaning. Writes to
    /// the library, as the app does in the background
    #[arg(long)]
    index: bool,
    /// Measure search quality and speed on labelled transcripts (e.g.
    /// eval/search/queries.json), in a scratch library
    #[arg(long, value_name = "QUERIES")]
    eval: Option<PathBuf>,
    /// With --eval: also time queries with the scratch library padded to
    /// this many hours of transcript (copies of the same meetings)
    #[arg(long, default_value_t = 0.0)]
    scale_hours: f64,
    /// With --eval: also time searching everything with the scratch library
    /// holding this many dictations (copies of one, embedded once)
    #[arg(long, default_value_t = 0)]
    scale_dictations: usize,
    /// Most results to show
    #[arg(long, default_value_t = 20)]
    limit: usize,
    /// With --eval: print each query's closest chunks and their similarity,
    /// for tuning the cut-offs
    #[arg(long)]
    scores: bool,
    /// With --eval: the least similarity that counts as a match by meaning
    #[arg(long)]
    min_score: Option<f32>,
    /// With --eval: how far below the best match by meaning others may be
    #[arg(long)]
    spread: Option<f32>,
    /// CPU cores for the embedding model (default: half, as in the app)
    #[arg(long)]
    threads: Option<usize>,
}

pub fn run(args: Args, models_dir: &Path) -> anyhow::Result<()> {
    if let Some(queries) = &args.eval {
        return eval(&args, queries, models_dir);
    }
    let library = crate::library::open()?;
    let embedder = if args.words_only {
        None
    } else {
        Some(load(models_dir, args.threads)?)
    };
    if args.index
        && let Some(e) = &embedder
    {
        index(&library, e)?;
    }
    let Some(query) = &args.query else {
        if !args.index {
            bail!("give a query to search for, or --index or --eval");
        }
        return Ok(());
    };
    if let Some(e) = &embedder {
        let coverage = library.semantic_coverage(e.id())?;
        if coverage.indexed < coverage.total {
            eprintln!(
                "{} of {} meetings are indexed for search by meaning; --index does the rest",
                coverage.indexed, coverage.total
            );
        }
        let items = library.search_items_coverage(e.id())?;
        if args.everything && items.indexed < items.total {
            eprintln!(
                "{} of {} notes, action items and dictations are indexed for search by meaning; --index does the rest",
                items.indexed, items.total
            );
        }
    }
    if args.everything {
        let started = Instant::now();
        let results = library.search_everything(
            query,
            Scope::All,
            embedder.as_ref().map(|e| e as &dyn Embedder),
            args.limit,
        )?;
        let took = started.elapsed();
        print_results(&results);
        eprintln!(
            "{} results in {:.0} ms",
            results.len(),
            took.as_secs_f64() * 1000.0
        );
        return Ok(());
    }
    let started = Instant::now();
    let hits = library.hybrid_search(
        query,
        embedder.as_ref().map(|e| e as &dyn Embedder),
        args.limit,
    )?;
    let took = started.elapsed();
    for h in &hits {
        print_hit(h);
    }
    eprintln!(
        "{} results in {:.0} ms",
        hits.len(),
        took.as_secs_f64() * 1000.0
    );
    Ok(())
}

fn load(models_dir: &Path, threads: Option<usize>) -> anyhow::Result<quill_embed::E5Embedder> {
    if !semantic::model_ready(models_dir) {
        bail!(
            "the search model isn't downloaded ({} MB); run: quill-cli models download search && quill-cli models download search-tokenizer",
            semantic::required_models()
                .iter()
                .map(|m| m.size_bytes)
                .sum::<u64>()
                / 1_000_000
        );
    }
    let started = Instant::now();
    let e = semantic::load_embedder(
        models_dir,
        threads.unwrap_or_else(semantic::default_threads),
    )
    .context("loading the search model")?;
    eprintln!(
        "loaded the search model in {:.0} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    Ok(e)
}

/// Indexes every pending meeting, then the notes, action items and
/// dictations, printing progress. Returns the time spent on meetings.
fn index(library: &Library, embedder: &dyn Embedder) -> anyhow::Result<std::time::Duration> {
    let took = index_meetings(library, embedder)?;
    let started = Instant::now();
    let mut items = 0;
    while let Some(n) = library.index_search_items(embedder, &|| true)?
        && n > 0
    {
        items += n;
    }
    if items > 0 {
        eprintln!(
            "indexed {items} notes, action items and dictations in {:.1} s",
            started.elapsed().as_secs_f64()
        );
    }
    Ok(took)
}

fn index_meetings(
    library: &Library,
    embedder: &dyn Embedder,
) -> anyhow::Result<std::time::Duration> {
    let pending = library.semantic_pending(embedder.id())?;
    let started = Instant::now();
    let mut chunks = 0;
    for (i, id) in pending.iter().enumerate() {
        let n = library
            .index_meeting(id, embedder, &|| true)?
            .unwrap_or_default();
        chunks += n;
        eprintln!("indexed {id} ({n} chunks) [{}/{}]", i + 1, pending.len());
    }
    let took = started.elapsed();
    if !pending.is_empty() {
        eprintln!(
            "indexed {} meetings, {chunks} chunks, in {:.1} s",
            pending.len(),
            took.as_secs_f64()
        );
    }
    Ok(took)
}

fn print_hit(h: &SearchHit) {
    let how = match h.matched {
        Matched::Words => "words  ",
        Matched::Meaning => "meaning",
        Matched::Both => "both   ",
    };
    let at = h.start_ms / 1000;
    println!(
        "{how} {:>2}:{:02}  {}  {}{}",
        at / 60,
        at % 60,
        h.title,
        h.speaker
            .as_deref()
            .map(|s| format!("{s}: "))
            .unwrap_or_default(),
        h.snippet
    );
}

/// Results of searching everything, by kind.
fn print_results(r: &search::Results) {
    let how = |m: Matched| match m {
        Matched::Words => "words  ",
        Matched::Meaning => "meaning",
        Matched::Both => "both   ",
    };
    let more = |more: bool| if more { " (more)" } else { "" };
    if !r.meetings.hits.is_empty() {
        println!("Meetings{}", more(r.meetings.more));
        for h in &r.meetings.hits {
            println!("  words   {}", h.snippet);
        }
    }
    if !r.moments.hits.is_empty() {
        println!("Moments{}", more(r.moments.more));
        for h in &r.moments.hits {
            print!("  ");
            print_hit(h);
        }
    }
    if !r.notes.hits.is_empty() {
        println!("Notes{}", more(r.notes.more));
        for h in &r.notes.hits {
            println!(
                "  {} {} · {}  {}",
                how(h.matched),
                h.title,
                h.heading.as_deref().unwrap_or("Notes"),
                h.snippet.replace('\n', " ")
            );
        }
    }
    if !r.actions.hits.is_empty() {
        println!("Action items{}", more(r.actions.more));
        for h in &r.actions.hits {
            println!(
                "  {} {}{}  ({})",
                how(h.matched),
                if h.done { "✓ " } else { "" },
                h.snippet,
                h.title
            );
        }
    }
    if !r.dictations.hits.is_empty() {
        println!("Dictations{}", more(r.dictations.more));
        for h in &r.dictations.hits {
            println!(
                "  {} {} {}  {}",
                how(h.matched),
                h.created_at.get(..10).unwrap_or(&h.created_at),
                h.app.as_deref().unwrap_or(""),
                h.snippet
            );
        }
    }
}

#[derive(serde::Deserialize)]
struct EvalFile {
    /// Meeting name → transcript JSON, relative to this file.
    transcripts: HashMap<String, PathBuf>,
    queries: Vec<LabelledQuery>,
}

fn eval(args: &Args, path: &Path, models_dir: &Path) -> anyhow::Result<()> {
    let file: EvalFile = serde_json::from_str(
        &std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    let scratch = tempfile_dir()?;
    let library = Library::open(&scratch, &scratch.join("recordings"))?;

    let mut names: Vec<_> = file.transcripts.iter().collect();
    names.sort();
    let mut ids = HashMap::new();
    let mut audio_ms = 0_u64;
    let mut words = 0_usize;
    for (name, transcript) in names {
        let segments: Vec<Segment> = serde_json::from_str(
            &std::fs::read_to_string(base.join(transcript))
                .with_context(|| format!("reading {}", transcript.display()))?,
        )?;
        audio_ms += segments.iter().map(|s| s.end_ms).max().unwrap_or(0);
        words += segments
            .iter()
            .map(|s| s.text.split_whitespace().count())
            .sum::<usize>();
        let meeting = library.create_meeting()?;
        library.rename(&meeting.id, name)?;
        library.save_transcript(&meeting.id, &segments)?;
        ids.insert(name.clone(), meeting.id);
    }
    let db = scratch.join("quill.sqlite");
    let size_before = std::fs::metadata(&db)?.len();

    let embedder = load(models_dir, args.threads)?;
    let took = index(&library, &embedder)?;
    let (chunks, vector_bytes) = semantic::index_size(&library)?;
    let size_after = std::fs::metadata(&db)?.len();
    let hours = audio_ms as f64 / 3_600_000.0;
    println!(
        "Indexed {:.1} min of transcript ({words} words) into {chunks} chunks in {:.2} s: {:.1} s per hour of transcript.",
        audio_ms as f64 / 60_000.0,
        took.as_secs_f64(),
        took.as_secs_f64() / hours.max(1e-9)
    );
    println!(
        "Vectors: {vector_bytes} bytes; the database grew {} bytes ({:.0} KB per hour of transcript).",
        size_after.saturating_sub(size_before),
        size_after.saturating_sub(size_before) as f64 / 1000.0 / hours.max(1e-9)
    );

    let queries: Vec<LabelledQuery> = file
        .queries
        .iter()
        .map(|q| LabelledQuery {
            query: q.query.clone(),
            expect: q
                .expect
                .iter()
                .map(|(name, from, to)| (ids.get(name).cloned().unwrap_or_default(), *from, *to))
                .collect(),
        })
        .collect();
    // Warm up (first query pays for allocations and caches).
    library.hybrid_search("warm up", Some(&embedder), args.limit)?;
    if args.scores {
        let names: HashMap<&String, &String> = ids.iter().map(|(name, id)| (id, name)).collect();
        for q in &queries {
            println!("{}", q.query);
            for (id, start_ms, score) in library.meaning_scores(&q.query, &embedder, 5)? {
                // A chunk starting up to a minute before an expected stretch
                // may well hold it.
                let expected = q
                    .expect
                    .iter()
                    .any(|(e, from, to)| *e == id && start_ms <= *to && start_ms + 60_000 >= *from);
                println!(
                    "  {score:.3} {} {}:{:02}{}",
                    names.get(&id).map_or("?", |n| n.as_str()),
                    start_ms / 60_000,
                    start_ms / 1000 % 60,
                    if expected { "  <- expected" } else { "" }
                );
            }
            for h in library.hybrid_search(&q.query, Some(&embedder), 3)?.iter() {
                print!("  = ");
                print_hit(h);
            }
        }
    }
    let defaults = semantic::Cutoffs::default();
    let cutoffs = semantic::Cutoffs {
        min_score: args.min_score.unwrap_or(defaults.min_score),
        spread: args.spread.unwrap_or(defaults.spread),
    };
    let outcomes = semantic::evaluate(&library, &embedder, &queries, args.limit, cutoffs)?;
    println!(
        "\n| Query | Words only | Words + meaning | Only by meaning | ms |\n|---|---:|---:|---:|---:|"
    );
    let rank = |r: Option<usize>| r.map_or("–".to_string(), |r| r.to_string());
    for o in &outcomes {
        println!(
            "| {} | {} | {} | {} | {:.0} |",
            o.query,
            rank(o.words_rank),
            rank(o.hybrid_rank),
            o.meaning_only,
            o.hybrid_ms
        );
    }
    let at = |k: usize, f: fn(&semantic::QueryOutcome) -> Option<usize>| {
        outcomes
            .iter()
            .filter(|o| f(o).is_some_and(|r| r <= k))
            .count()
    };
    println!(
        "\nFound first: words {}/{n}, words + meaning {}/{n}. In the top 3: words {}/{n}, words + meaning {}/{n}.",
        at(1, |o| o.words_rank),
        at(1, |o| o.hybrid_rank),
        at(3, |o| o.words_rank),
        at(3, |o| o.hybrid_rank),
        n = outcomes.len()
    );
    println!("Query time: median {:.0} ms.", median(&outcomes));

    if args.scale_hours > 0.0 {
        semantic::pad_for_benchmark(&library, args.scale_hours)?;
        let (chunks, vector_bytes) = semantic::index_size(&library)?;
        library.hybrid_search("warm up", Some(&embedder), args.limit)?;
        let outcomes = semantic::evaluate(&library, &embedder, &queries, args.limit, cutoffs)?;
        let words_ms = {
            let started = Instant::now();
            for q in &queries {
                library.search(&q.query, args.limit)?;
            }
            started.elapsed().as_secs_f64() * 1000.0 / queries.len().max(1) as f64
        };
        println!(
            "Padded to {:.0} hours ({chunks} chunks, {:.1} MB of vectors, database {:.1} MB): words + meaning median {:.0} ms per query (words alone {:.1} ms).",
            args.scale_hours,
            vector_bytes as f64 / 1e6,
            std::fs::metadata(&db)?.len() as f64 / 1e6,
            median(&outcomes),
            words_ms
        );
    }
    if args.scale_dictations > 0 {
        search::pad_dictations_for_benchmark(&library, args.scale_dictations, &embedder)?;
    }
    // Searching everything, as the app's search window does (five of each).
    library.search_everything("warm up", Scope::All, Some(&embedder), 5)?;
    let mut everything_ms: Vec<f64> = Vec::new();
    for q in &queries {
        let started = Instant::now();
        library.search_everything(&q.query, Scope::All, Some(&embedder), 5)?;
        everything_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    everything_ms.sort_by(f64::total_cmp);
    println!(
        "Searching everything{}: median {:.0} ms per query.",
        if args.scale_dictations > 0 {
            format!(" with {} dictations", args.scale_dictations)
        } else {
            String::new()
        },
        everything_ms
            .get(everything_ms.len() / 2)
            .copied()
            .unwrap_or(0.0)
    );
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}

fn median(outcomes: &[semantic::QueryOutcome]) -> f64 {
    let mut ms: Vec<f64> = outcomes.iter().map(|o| o.hybrid_ms).collect();
    ms.sort_by(f64::total_cmp);
    ms.get(ms.len() / 2).copied().unwrap_or(0.0)
}

/// A new empty folder for the scratch library.
fn tempfile_dir() -> anyhow::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "quill-search-eval-{}-{}",
        std::process::id(),
        jiff::Timestamp::now().as_millisecond()
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
