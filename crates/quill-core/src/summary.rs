//! Summarizing a transcript with an LLM.
//!
//! Short transcripts go to the model in one request. Long ones are split into
//! parts that are summarized separately ("map") and then combined ("reduce"),
//! so a three-hour meeting works even with a local 32k-token model.

use futures::stream::{self, StreamExt, TryStreamExt};
use quill_llm::prompts::SummaryPrompts;
use quill_llm::{CompletionRequest, CompletionResponse, LlmError, LlmProvider, Message};
use quill_stt::Segment;

/// Output budget for the final notes and for each part's notes.
const SUMMARY_MAX_TOKENS: u32 = 4096;
const PART_MAX_TOKENS: u32 = 2048;
/// Low but non-zero: factual and consistent, without the repetitiveness
/// some models show at exactly 0.
const TEMPERATURE: f32 = 0.2;
/// Room for the system prompt, template text and the model's answer.
const RESERVED_TOKENS: usize = 6_000;
/// Upper bound per part in map-reduce. Smaller parts get more detailed notes.
const MAX_PART_TOKENS: usize = 15_000;

#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    #[error("the transcript is empty")]
    EmptyTranscript,
    #[error(transparent)]
    Llm(#[from] LlmError),
    #[error("prompt template error: {0}")]
    Prompt(#[from] quill_llm::prompts::RenderError),
}

/// A finished summary plus what it took to make it.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub markdown: String,
    /// Model that wrote the final summary (as reported by the provider).
    pub model: String,
    pub prompt_version: &'static str,
    /// 1 for a single pass; parts + 1 for map-reduce.
    pub requests: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// Total cost if every request reported one.
    pub cost_usd: Option<f64>,
    /// Wall-clock time for the whole summary.
    pub latency_ms: u64,
}

/// Summarizes `segments` with `model` on `provider`, using one version of
/// the summary prompts (normally [`prompts::SUMMARY`]). `date` goes into the
/// title line (e.g. `2026-10-11`).
pub async fn summarize(
    provider: &dyn LlmProvider,
    model: &str,
    prompts: &SummaryPrompts,
    segments: &[Segment],
    date: &str,
) -> Result<Summary, SummaryError> {
    let lines: Vec<String> = segments.iter().map(transcript_line).collect();
    if lines.is_empty() {
        return Err(SummaryError::EmptyTranscript);
    }
    let started = std::time::Instant::now();
    let duration = format_duration(segments.iter().map(|s| s.end_ms).max().unwrap_or(0));
    let budget = provider
        .context_tokens()
        .saturating_sub(RESERVED_TOKENS)
        .max(1_000);
    let transcript = lines.join("\n");
    let request = |user: String, max_tokens| CompletionRequest {
        model: model.to_string(),
        messages: vec![Message::system(prompts.system), Message::user(user)],
        max_tokens: Some(max_tokens),
        temperature: Some(TEMPERATURE),
    };

    let mut responses = Vec::new();
    if quill_llm::estimate_tokens(&transcript) <= budget {
        let prompt = quill_llm::prompts::render(
            prompts.single,
            &[
                ("date", date),
                ("duration", &duration),
                ("transcript", &transcript),
            ],
        )?;
        responses.push(
            provider
                .complete(request(prompt, SUMMARY_MAX_TOKENS))
                .await?,
        );
    } else {
        let parts = split_lines(&lines, budget.min(MAX_PART_TOKENS));
        let count = parts.len().to_string();
        let part_requests = parts
            .iter()
            .enumerate()
            .map(|(i, part)| {
                let range = part_range(&segments[part.start..part.end]);
                let prompt = quill_llm::prompts::render(
                    prompts.chunk,
                    &[
                        ("part", &(i + 1).to_string()),
                        ("parts", &count),
                        ("range", &range),
                        ("transcript", &lines[part.clone()].join("\n")),
                    ],
                )?;
                Ok((range, request(prompt, PART_MAX_TOKENS)))
            })
            .collect::<Result<Vec<_>, SummaryError>>()?;

        // "Map": summarize the parts, several at once if the provider allows.
        // `buffered` keeps results in input order, which the reduce step needs.
        tracing::info!(
            parts = parts.len(),
            "transcript too long for one request; using map-reduce"
        );
        let part_notes: Vec<(String, CompletionResponse)> =
            stream::iter(part_requests)
                .map(|(range, req)| async move {
                    Ok::<_, LlmError>((range, provider.complete(req).await?))
                })
                .buffered(provider.max_concurrency())
                .try_collect()
                .await?;

        // "Reduce": combine the part notes into the final summary.
        let notes = part_notes
            .iter()
            .enumerate()
            .map(|(i, (range, r))| format!("### Part {} ({range})\n\n{}", i + 1, clean(&r.text)))
            .collect::<Vec<_>>()
            .join("\n\n");
        let prompt = quill_llm::prompts::render(
            prompts.reduce,
            &[
                ("date", date),
                ("duration", &duration),
                ("parts", &count),
                ("notes", &notes),
            ],
        )?;
        responses.extend(part_notes.into_iter().map(|(_, r)| r));
        responses.push(
            provider
                .complete(request(prompt, SUMMARY_MAX_TOKENS))
                .await?,
        );
    }

    // The last response is always the final summary.
    let last = responses
        .last()
        .cloned()
        .ok_or(SummaryError::EmptyTranscript)?;
    Ok(Summary {
        markdown: clean(&last.text),
        model: last.model,
        prompt_version: prompts.version,
        requests: responses.len(),
        prompt_tokens: responses.iter().map(|r| u64::from(r.prompt_tokens)).sum(),
        completion_tokens: responses
            .iter()
            .map(|r| u64::from(r.completion_tokens))
            .sum(),
        // `Option<f64>: Sum` — None if any request didn't report a cost.
        cost_usd: responses.iter().map(|r| r.cost_usd).sum(),
        latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

/// `[mm:ss] Speaker: text`, the transcript format the prompts describe.
pub fn transcript_line(segment: &Segment) -> String {
    let secs = segment.start_ms / 1000;
    let time = if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
    } else {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    };
    match &segment.speaker {
        Some(speaker) => format!("[{time}] {speaker}: {}", segment.text),
        None => format!("[{time}] {}", segment.text),
    }
}

/// Splits lines into consecutive index ranges of at most `max_tokens` each
/// (estimated). A single over-long line still gets its own part.
fn split_lines(lines: &[String], max_tokens: usize) -> Vec<std::ops::Range<usize>> {
    let mut parts = Vec::new();
    let (mut start, mut tokens) = (0, 0);
    for (i, line) in lines.iter().enumerate() {
        let t = quill_llm::estimate_tokens(line) + 1;
        if tokens + t > max_tokens && i > start {
            parts.push(start..i);
            (start, tokens) = (i, 0);
        }
        tokens += t;
    }
    parts.push(start..lines.len());
    parts
}

fn part_range(segments: &[Segment]) -> String {
    let first = segments.first().map_or(0, |s| s.start_ms);
    let last = segments.last().map_or(0, |s| s.end_ms);
    format!("{}–{}", clock(first), clock(last))
}

fn clock(ms: u64) -> String {
    let secs = ms / 1000;
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn format_duration(ms: u64) -> String {
    let mins = ms.div_ceil(60_000);
    if mins >= 60 {
        format!("{} h {} min", mins / 60, mins % 60)
    } else {
        format!("{mins} min")
    }
}

/// Removes an empty `<think>…</think>` block some reasoning models emit even
/// with thinking disabled, plus surrounding whitespace.
fn clean(text: &str) -> String {
    let text = text.trim();
    match (text.strip_prefix("<think>"), text.find("</think>")) {
        (Some(_), Some(end)) => text[end + "</think>".len()..].trim().to_string(),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use quill_llm::prompts::SUMMARY;

    use super::*;

    /// Fake provider: records requests and answers "notes N" / "final".
    struct FakeProvider {
        context: usize,
        requests: Mutex<Vec<CompletionRequest>>,
    }

    #[async_trait::async_trait]
    impl LlmProvider for FakeProvider {
        async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            let mut requests = self.requests.lock().unwrap();
            requests.push(req);
            let n = requests.len();
            Ok(CompletionResponse {
                text: format!("<think>\n\n</think>\n\nanswer {n}"),
                model: "fake-1".into(),
                prompt_tokens: 100,
                completion_tokens: 10,
                cost_usd: Some(0.001),
                latency_ms: 5,
            })
        }

        fn context_tokens(&self) -> usize {
            self.context
        }
    }

    fn fake(context: usize) -> FakeProvider {
        FakeProvider {
            context,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn meeting(n: usize) -> Vec<Segment> {
        (0..n)
            .map(|i| Segment {
                start_ms: i as u64 * 5_000,
                end_ms: i as u64 * 5_000 + 4_000,
                text: format!("Sentence number {i} about the quarterly roadmap and hiring."),
                speaker: Some(if i % 2 == 0 { "Me" } else { "Others" }.into()),
            })
            .collect()
    }

    #[tokio::test]
    async fn short_transcript_is_one_request() {
        let provider = fake(100_000);
        let summary = summarize(&provider, "m", &SUMMARY, &meeting(10), "2026-10-11")
            .await
            .unwrap();

        assert_eq!(summary.requests, 1);
        assert_eq!(summary.markdown, "answer 1");
        assert_eq!(summary.prompt_version, "summary-v1");
        let requests = provider.requests.lock().unwrap();
        let user = &requests[0].messages[1].content;
        assert!(user.contains("[00:05] Others: Sentence number 1"), "{user}");
        assert!(user.contains("— 2026-10-11"));
    }

    #[tokio::test]
    async fn long_transcript_uses_map_reduce_in_order() {
        // ~7k tokens of transcript with a 7k context → 1k budget per part.
        let provider = fake(7_000);
        let summary = summarize(&provider, "m", &SUMMARY, &meeting(400), "2026-10-11")
            .await
            .unwrap();

        let requests = provider.requests.lock().unwrap();
        let parts = requests.len() - 1;
        assert!(parts >= 5, "{parts} parts");
        assert_eq!(summary.requests, parts + 1);
        // The reduce request lists every part's notes, in order.
        let reduce = &requests[parts].messages[1].content;
        assert!(reduce.contains("### Part 1 (00:00–"));
        assert!(reduce.contains(&format!("### Part {parts} (")));
        // Tokens and cost add up over all requests.
        assert_eq!(summary.prompt_tokens, 100 * (parts as u64 + 1));
        assert!((summary.cost_usd.unwrap() - 0.001 * (parts as f64 + 1.0)).abs() < 1e-9);
        // Part notes were cleaned of empty <think> blocks before reducing.
        assert!(!reduce.contains("<think>"));
    }

    #[tokio::test]
    async fn empty_transcript_is_an_error() {
        let result = summarize(&fake(100_000), "m", &SUMMARY, &[], "d").await;
        assert!(matches!(result, Err(SummaryError::EmptyTranscript)));
    }

    #[test]
    fn splits_lines_within_budget() {
        let lines: Vec<String> = (0..10).map(|_| "x".repeat(30)).collect(); // ~11 tokens each
        let parts = split_lines(&lines, 25);
        assert_eq!(parts, vec![0..2, 2..4, 4..6, 6..8, 8..10]);
    }
}
