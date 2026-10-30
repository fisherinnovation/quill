//! Search across everything with the real search model: notes points,
//! action items and dictations found by meaning, and nothing for nonsense.
//! A check on the cut-offs (`semantic::Cutoffs`), which were tuned on
//! transcript chunks, for these much shorter texts.
//!
//! Ignored by default because it needs the downloaded model. Run it with:
//!
//! ```text
//! cargo run -p quill-cli -- models download search
//! cargo run -p quill-cli -- models download search-tokenizer
//! QUILL_TEST_MODELS_DIR="$(cargo run -q -p quill-cli -- models path)" \
//!     cargo test --release -p quill-core --test search_smoke -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use quill_core::library::{DictationEntry, Library};
use quill_core::search::{Results, Scope};
use quill_core::semantic::{self, Matched};
use quill_core::summary::Summary;

const NOTES: &str = "# Q4 planning — 2026-10-01

## TL;DR
- The trade show budget was cut by a third; the team will focus on two online campaigns instead.

## Decisions
- Hire one backend engineer in November.
- Move the pricing page redesign to the first quarter.

## Action items
- **Priya** — book the venue for the offsite — by Friday
- **You** — draft the job description for the backend role

## Discussion by topic
### Infrastructure
- The database is close to its storage limit; Dan will look at archiving old events.

## Open questions
- Do we keep the free tier after the price change?
";

const DICTATIONS: &[&str] = &[
    "Can you send me the slides before the board meeting tomorrow?",
    "The retry loop never caps its delay. Cap it at sixty seconds and add jitter.",
    "Let's push the launch to next Tuesday so marketing has time for the emails.",
    "I'm out from the 12th to the 26th; Priya covers support while I'm away.",
    "Thanks, that fixed it.",
    "We should interview at least three more candidates for the designer role.",
    "Remind me to renew the domain before it expires in November.",
    "Lunch on Friday works for me, the usual place?",
];

/// What a query should find: a kind and a word in the right result.
#[derive(Clone, Copy)]
enum Want {
    Notes(&'static str),
    Action(&'static str),
    Dictation(&'static str),
    Nothing,
}

const QUERIES: &[(&str, Want)] = &[
    ("vacation", Want::Dictation("12th")),
    ("presentation for the directors", Want::Dictation("slides")),
    ("postpone the release", Want::Dictation("launch")),
    ("exponential backoff", Want::Dictation("retry")),
    ("recruiting a designer", Want::Dictation("candidates")),
    ("staffing plans", Want::Notes("engineer")),
    ("conference spending", Want::Notes("trade show")),
    ("running out of disk space", Want::Notes("storage")),
    ("writing the job posting", Want::Action("job description")),
    (
        "reserve a location for the team retreat",
        Want::Action("venue"),
    ),
    ("purple elephant symphony", Want::Nothing),
    ("qwerty asdf zxcv", Want::Nothing),
];

#[test]
#[ignore = "needs the search model; set QUILL_TEST_MODELS_DIR"]
fn finds_notes_action_items_and_dictations_by_meaning() {
    let models =
        PathBuf::from(std::env::var("QUILL_TEST_MODELS_DIR").expect("set QUILL_TEST_MODELS_DIR"));
    let embedder = semantic::load_embedder(&models, semantic::default_threads()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path(), &dir.path().join("recordings")).unwrap();
    let meeting = lib.create_meeting().unwrap();
    lib.save_summary(
        &meeting.id,
        &Summary {
            markdown: NOTES.into(),
            model: "test".into(),
            prompt_version: "test",
            requests: 1,
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: None,
            latency_ms: 0,
            complete: true,
        },
    )
    .unwrap();
    for text in DICTATIONS {
        lib.add_dictation(&DictationEntry {
            id: 0,
            created_at: String::new(),
            text: (*text).into(),
            raw: None,
            polish_model: None,
            audio_ms: 3000,
            latency_ms: 300,
            pasted: true,
            app: Some("Slack".into()),
            app_id: None,
            wait_ms: None,
            corrected: None,
        })
        .unwrap();
    }
    while lib.index_search_items(&embedder, &|| true).unwrap() != Some(0) {}

    // Cut-offs to try instead of the defaults: QUILL_MIN_SCORE, QUILL_SPREAD.
    let defaults = semantic::Cutoffs::default();
    let var = |name: &str, default: f32| {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    };
    let cutoffs = semantic::Cutoffs {
        min_score: var("QUILL_MIN_SCORE", defaults.min_score),
        spread: var("QUILL_SPREAD", defaults.spread),
    };
    let mut found = 0;
    let mut noise = 0;
    for (query, want) in QUERIES {
        let started = std::time::Instant::now();
        let r = lib
            .search_everything_with(query, Scope::All, Some(&embedder), 5, cutoffs)
            .unwrap();
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let first = |texts: Vec<&str>, word: &str| texts.first().is_some_and(|t| t.contains(word));
        let ok = match want {
            Want::Notes(w) => first(r.notes.hits.iter().map(|h| h.snippet.as_str()).collect(), w),
            Want::Action(w) => first(r.actions.hits.iter().map(|h| h.task.as_str()).collect(), w),
            Want::Dictation(w) => first(
                r.dictations
                    .hits
                    .iter()
                    .map(|h| h.snippet.as_str())
                    .collect(),
                w,
            ),
            Want::Nothing => r.is_empty(),
        };
        // Results found by meaning that aren't what was asked for.
        let extra =
            meaning_only(&r).saturating_sub(usize::from(ok && !matches!(want, Want::Nothing)));
        noise += extra;
        found += usize::from(ok);
        println!(
            "{} {query:?} ({ms:.0} ms, {extra} other by meaning)",
            if ok { "ok  " } else { "MISS" }
        );
        print_results(&r);
        for (kind, text, score) in lib.item_scores(query, &embedder, 4).unwrap() {
            println!("       {score:.3} {kind:9} {}", text.replace('\n', " "));
        }
    }
    println!(
        "{found} of {} as expected; {noise} other results by meaning",
        QUERIES.len()
    );
    // Nonsense finds nothing, and most paraphrases find their answer first.
    assert!(
        found >= QUERIES.len() - 2,
        "only {found} of {} as expected",
        QUERIES.len()
    );
}

fn meaning_only(r: &Results) -> usize {
    r.notes
        .hits
        .iter()
        .filter(|h| h.matched == Matched::Meaning)
        .count()
        + r.actions
            .hits
            .iter()
            .filter(|h| h.matched == Matched::Meaning)
            .count()
        + r.dictations
            .hits
            .iter()
            .filter(|h| h.matched == Matched::Meaning)
            .count()
}

fn print_results(r: &Results) {
    for h in &r.notes.hits {
        println!(
            "     notes      {:?} {:?} {}",
            h.matched,
            h.heading,
            h.snippet.replace('\n', " ")
        );
    }
    for h in &r.actions.hits {
        println!("     action     {:?} {}", h.matched, h.snippet);
    }
    for h in &r.dictations.hits {
        println!("     dictation  {:?} {}", h.matched, h.snippet);
    }
}
