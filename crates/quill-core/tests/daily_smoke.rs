//! A daily summary from a real model: three meetings whose notes overlap (a
//! launch date that moves, a hire discussed twice), to see that the summary
//! connects them, keeps what a decision ended up as, and adds nothing.
//!
//! Ignored by default because it needs a model. Run it with a local Ollama
//! model (nothing leaves the computer):
//!
//! ```text
//! QUILL_TEST_DAILY_MODEL=ollama:qwen3.5:9b \
//!     cargo test --release -p quill-core --test daily_smoke -- --ignored --nocapture
//! ```

use quill_core::daily;
use quill_core::library::Library;
use quill_core::summary::Summary;
use quill_llm::Privacy;
use quill_llm::prompts::DAILY;

const MEETINGS: &[(&str, &str)] = &[
    (
        "Product sync",
        "# Product sync — 2026-10-26

## TL;DR
- The onboarding redesign ships Friday behind a flag, to 10% of new sign-ups.
- Pricing experiments wait until the analytics double-counting is fixed.

## Decisions
- Ship onboarding Friday at 10%.
- Pause pricing experiments.

## Action items
- **You** — send the rollout plan to support — by Wednesday
- **Priya** — fix the duplicate signup events — by Thursday

## Open questions / risks
- Should the invite step move after the first project?
",
    ),
    (
        "Hiring sync",
        "# Hiring sync — 2026-10-26

## TL;DR
- Two backend roles are approved; the job ads go up Monday.
- Dan will run the take-home review.

## Decisions
- Open two backend engineering roles.

## Action items
- **You** — write the job description for the backend roles — by Friday
- **Dan** — write up the take-home rubric
",
    ),
    (
        "1:1 with Priya",
        "# 1:1 with Priya — 2026-10-26

## TL;DR
- Priya found the double-counting: a retry re-sends the event. The fix needs until Monday, so onboarding moves to Monday too.
- She'd like to interview candidates for the backend roles.

## Decisions
- Onboarding launch moves from Friday to Monday, after the analytics fix.

## Action items
- **Priya** — fix the retry that re-sends signup events — by Monday
- **You** — add Priya to the backend interview panel
",
    ),
];

#[tokio::test]
#[ignore = "needs a model; set QUILL_TEST_DAILY_MODEL (e.g. ollama:qwen3.5:9b)"]
async fn summarizes_a_day_that_changed_its_mind() {
    let spec = std::env::var("QUILL_TEST_DAILY_MODEL").expect("set QUILL_TEST_DAILY_MODEL");
    let (provider, model) = quill_llm::provider_for(&spec, Privacy::default()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let lib = Library::open(dir.path(), &dir.path().join("recordings")).unwrap();
    for (title, notes) in MEETINGS {
        let m = lib.create_meeting().unwrap();
        lib.save_summary(
            &m.id,
            &Summary {
                markdown: (*notes).into(),
                model: "test".into(),
                prompt_version: "test",
                requests: 1,
                prompt_tokens: 0,
                completion_tokens: 0,
                cost_usd: None,
                latency_ms: 0,
            },
        )
        .unwrap();
        lib.rename(&m.id, title).unwrap();
    }
    let day = jiff::Zoned::now().date();
    let sources = lib.day_sources(day).unwrap();
    assert_eq!(sources.len(), 3);

    let summary = daily::summarize_day(provider.as_ref(), &model, &DAILY, day, &sources)
        .await
        .unwrap();
    println!("{}\n", summary.markdown);
    println!(
        "{} · {} ms · {} prompt + {} completion tokens · cost {:?}",
        summary.model,
        summary.latency_ms,
        summary.prompt_tokens,
        summary.completion_tokens,
        summary.cost_usd
    );

    let text = summary.markdown.to_lowercase();
    assert!(summary.markdown.starts_with("## TL;DR"), "no TL;DR first");
    // The launch ended up on Monday, not Friday.
    assert!(
        text.contains("monday"),
        "the launch's move to Monday is missing"
    );
    // The tasks given to "you" are there.
    assert!(text.contains("rollout plan") && text.contains("job description"));
    // Meetings are cited by title, not as "Meeting 3".
    assert!(text.contains("hiring sync") || text.contains("product sync"));
    assert!(!text.contains("(meeting "), "cited a meeting by number");
    // Nobody who isn't in the notes.
    for name in ["sarah", "alex", "john", "maria"] {
        assert!(!text.contains(name), "invented a person: {name}");
    }
    let saved = lib.save_daily_summary(day, &sources, &summary).unwrap();
    assert!(!saved.outdated);
}
