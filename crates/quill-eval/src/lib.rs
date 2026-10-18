//! Evaluation harness for Quill: runs transcripts x models x prompt versions,
//! records tokens, cost and latency, and scores summaries with an LLM judge.
//!
//! Flow: [`load_cases`] → [`run_summaries`] → [`judge::judge_all`] →
//! [`store`] (SQLite + CSV) and [`report`] (leaderboard + side-by-side review).
//! [`wer`] scores dictation against reference text.

pub mod judge;
pub mod report;
pub mod store;
/// Word error rate, from `quill_core` (the app scores reviewed dictation
/// samples with it too).
pub use quill_core::wer;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::stream::{self, StreamExt};
use quill_core::summary::{self, Summary};
use quill_llm::prompts::SummaryPrompts;
use quill_llm::{LlmError, LlmProvider};
use quill_stt::Segment;

pub use judge::Scores;

/// One transcript to summarize.
#[derive(Debug, Clone)]
pub struct Case {
    /// File stem, e.g. `product-sync`.
    pub name: String,
    pub segments: Vec<Segment>,
}

impl Case {
    /// Meeting length in hours, for cost per meeting-hour.
    pub fn hours(&self) -> f64 {
        self.segments.iter().map(|s| s.end_ms).max().unwrap_or(0) as f64 / 3_600_000.0
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("no transcripts (*.json) found in {0}")]
    NoCases(PathBuf),
    #[error("reading {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("parsing {path}: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error(transparent)]
    Llm(#[from] LlmError),
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("CSV: {0}")]
    Csv(#[from] csv::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Loads every `*.json` transcript in `dir` and in `dir/private/` (where
/// real, gitignored meetings go), sorted by name.
pub fn load_cases(dir: &Path) -> Result<Vec<Case>, EvalError> {
    let mut paths = Vec::new();
    for d in [dir.to_path_buf(), dir.join("private")] {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                paths.push(path);
            }
        }
    }
    paths.sort();

    let mut cases = Vec::with_capacity(paths.len());
    for path in paths {
        let json = std::fs::read_to_string(&path).map_err(|source| EvalError::Read {
            path: path.clone(),
            source,
        })?;
        let segments = serde_json::from_str(&json).map_err(|source| EvalError::Parse {
            path: path.clone(),
            source,
        })?;
        let name = path
            .file_stem()
            .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        cases.push(Case { name, segments });
    }
    if cases.is_empty() {
        return Err(EvalError::NoCases(dir.to_path_buf()));
    }
    Ok(cases)
}

/// The outcome of summarizing one case with one model and prompt version.
#[derive(Debug, Clone)]
pub struct RunResult {
    pub case: String,
    pub case_hours: f64,
    /// Model spec as given (`anthropic/…` or `ollama:…`).
    pub model: String,
    pub prompt_version: &'static str,
    /// The summary, or why it failed.
    pub outcome: Result<Summary, String>,
    /// Filled in by the judge.
    pub scores: Option<Scores>,
}

/// Turns a model spec into a provider and the model id to send it. Injected
/// so tests can use fake providers; the CLI passes `quill_llm::provider_for`.
pub type ProviderFactory<'a> =
    &'a (dyn Fn(&str) -> Result<(Box<dyn LlmProvider>, String), LlmError> + Sync);

/// Summarizes every case with every model and prompt version, up to
/// `concurrency` at a time. Failures are recorded per result, not fatal: one
/// unavailable model shouldn't throw away a whole run.
pub async fn run_summaries(
    cases: &[Case],
    models: &[String],
    prompts: &[&'static SummaryPrompts],
    date: &str,
    factory: ProviderFactory<'_>,
    concurrency: usize,
) -> Vec<RunResult> {
    // One provider per model spec, shared by all its jobs. `Arc` because
    // several in-flight futures use the same provider at once.
    type Shared = Result<(Arc<dyn LlmProvider>, String), String>;
    let providers: HashMap<&str, Shared> = models
        .iter()
        .map(|spec| {
            let provider = factory(spec)
                .map(|(p, id)| (Arc::from(p), id))
                .map_err(|e| e.to_string());
            (spec.as_str(), provider)
        })
        .collect();

    let jobs = cases.iter().flat_map(|case| {
        models
            .iter()
            .flat_map(move |model| prompts.iter().map(move |p| (case, model, *p)))
    });

    stream::iter(jobs)
        .map(|(case, model, prompts)| {
            let provider = &providers[model.as_str()];
            async move {
                let outcome = match provider {
                    Ok((provider, id)) => {
                        tracing::info!(case = %case.name, model, prompt = prompts.version, "summarizing");
                        summary::summarize(provider.as_ref(), id, prompts, &case.segments, date)
                            .await
                            .map_err(|e| e.to_string())
                    }
                    Err(e) => Err(e.clone()),
                };
                if let Err(e) = &outcome {
                    tracing::warn!(case = %case.name, model, "failed: {e}");
                }
                RunResult {
                    case: case.name.clone(),
                    case_hours: case.hours(),
                    model: model.clone(),
                    prompt_version: prompts.version,
                    outcome,
                    scores: None,
                }
            }
        })
        .buffer_unordered(concurrency.max(1))
        .collect::<Vec<_>>()
        .await
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Mutex;

    use quill_llm::{CompletionRequest, CompletionResponse, LlmError, LlmProvider};

    /// Fake provider: answers every request with `reply(request)`.
    pub struct Fake<F> {
        pub reply: F,
        pub requests: Mutex<Vec<CompletionRequest>>,
    }

    impl<F> Fake<F> {
        pub fn new(reply: F) -> Self {
            Self {
                reply,
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl<F> LlmProvider for Fake<F>
    where
        F: Fn(&CompletionRequest) -> Result<String, LlmError> + Send + Sync,
    {
        async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            let text = (self.reply)(&req)?;
            let model = req.model.clone();
            self.requests.lock().unwrap().push(req);
            Ok(CompletionResponse {
                text,
                model,
                prompt_tokens: 1000,
                completion_tokens: 200,
                cost_usd: Some(0.01),
                latency_ms: 10,
            })
        }

        fn context_tokens(&self) -> usize {
            100_000
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quill_llm::prompts::SUMMARY;

    fn case(name: &str, minutes: u64) -> Case {
        Case {
            name: name.into(),
            segments: vec![Segment {
                start_ms: 0,
                end_ms: minutes * 60_000,
                text: "We agreed to ship on Friday.".into(),
                speaker: Some("Me".into()),
            }],
        }
    }

    #[tokio::test]
    async fn runs_every_combination_and_records_failures() {
        let factory = |spec: &str| -> Result<(Box<dyn LlmProvider>, String), LlmError> {
            if spec == "broken" {
                return Err(LlmError::MissingKey("openrouter"));
            }
            let fake = testing::Fake::new(|req: &quill_llm::CompletionRequest| {
                Ok(format!("# Notes by {}", req.model))
            });
            Ok((Box::new(fake), spec.to_string()))
        };
        let cases = [case("a", 30), case("b", 60)];
        let models = ["good".to_string(), "broken".to_string()];

        let results = run_summaries(&cases, &models, &[&SUMMARY], "d", &factory, 3).await;

        assert_eq!(results.len(), 4);
        let ok: Vec<_> = results.iter().filter(|r| r.outcome.is_ok()).collect();
        assert_eq!(ok.len(), 2);
        assert!(ok.iter().all(|r| r.model == "good"));
        let failed = results.iter().find(|r| r.model == "broken").unwrap();
        assert!(failed.outcome.as_ref().unwrap_err().contains("no API key"));
        let b = results.iter().find(|r| r.case == "b").unwrap();
        assert!((b.case_hours - 1.0).abs() < 1e-9);
    }

    #[test]
    fn loads_cases_including_private_dir() {
        let dir = tempfile::tempdir().unwrap();
        let seg = r#"[{"start_ms":0,"end_ms":1000,"text":"hi"}]"#;
        std::fs::write(dir.path().join("b.json"), seg).unwrap();
        std::fs::create_dir(dir.path().join("private")).unwrap();
        std::fs::write(dir.path().join("private/a.json"), seg).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();

        let cases = load_cases(dir.path()).unwrap();

        let names: Vec<_> = cases.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["b", "a"]); // sorted by path: b.json < private/a.json
    }
}
