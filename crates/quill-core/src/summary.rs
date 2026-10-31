//! Summarizing a transcript with an LLM.
//!
//! Short transcripts go to the model in one request. Long ones are split into
//! parts that are summarized separately ("map") and then combined ("reduce"),
//! so a three-hour meeting works even with a local 32k-token model.

use futures::stream::{self, StreamExt, TryStreamExt};
use quill_llm::prompts::SummaryPrompts;
use quill_llm::{CompletionRequest, CompletionResponse, LlmError, LlmProvider, Message};
use quill_stt::Segment;

/// Output budget for the final notes and for each part's notes. Generous,
/// because a reasoning model's thinking counts against it too: at 4,096,
/// a third of one user's notes were cut off, 22-minute meetings included.
const SUMMARY_MAX_TOKENS: u32 = 8192;
const PART_MAX_TOKENS: u32 = 4096;
/// How many times a model that stops at the budget is asked to carry on
/// before its notes are saved as incomplete.
const MAX_CONTINUATIONS: usize = 2;
const CONTINUE: &str = "You reached the length limit. Continue exactly where you \
    stopped, mid-sentence if need be. Don't repeat anything or start over.";
/// Low but non-zero: factual and consistent, without the repetitiveness
/// some models show at exactly 0.
const TEMPERATURE: f32 = 0.2;
/// Room for the system prompt, template text and the model's answer.
const RESERVED_TOKENS: usize = 10_000;
/// Upper bound per part in map-reduce. Smaller parts get more detailed notes.
const MAX_PART_TOKENS: usize = 15_000;

#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    #[error("the transcript is empty")]
    EmptyTranscript,
    /// A daily summary was asked for a day whose meetings have no notes.
    #[error("no meeting that day has notes yet")]
    NoNotes,
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
    /// False if the model still stopped at its length limit after being
    /// asked to carry on, so the notes end early.
    pub complete: bool,
}

/// A model's whole answer to one request, continued if it was cut off.
struct Answer {
    response: CompletionResponse,
    requests: usize,
}

/// Summarizes `segments` with `model` on `provider`, using one version of
/// the summary prompts (normally [`prompts::SUMMARY`]). `date` goes into the
/// title line (e.g. `2026-10-11`); `vocabulary` is the user's names and
/// terms (see [`crate::vocabulary`]), for versions that take it.
pub async fn summarize(
    provider: &dyn LlmProvider,
    model: &str,
    prompts: &SummaryPrompts,
    segments: &[Segment],
    date: &str,
    vocabulary: &[String],
) -> Result<Summary, SummaryError> {
    // Whole turns rather than Whisper's arbitrary segment cuts.
    let segments = &crate::meeting::turns(segments);
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
    let note = prompts
        .vocabulary
        .then(|| quill_llm::prompts::vocabulary_note(&crate::vocabulary::terms(vocabulary)));
    let note = note.as_deref();
    let request = |user: String, max_tokens| CompletionRequest {
        model: model.to_string(),
        messages: vec![Message::system(prompts.system), Message::user(user)],
        max_tokens: Some(max_tokens),
        temperature: Some(TEMPERATURE),
        no_reasoning: false,
    };

    let mut answers = Vec::new();
    if quill_llm::estimate_tokens(&transcript) <= budget {
        let prompt = render(
            prompts.single,
            &[
                ("date", date),
                ("duration", &duration),
                ("transcript", &transcript),
            ],
            note,
        )?;
        answers.push(complete_in_full(provider, request(prompt, SUMMARY_MAX_TOKENS)).await?);
    } else {
        let parts = split_lines(&lines, budget.min(MAX_PART_TOKENS));
        let count = parts.len().to_string();
        let part_requests = parts
            .iter()
            .enumerate()
            .map(|(i, part)| {
                let range = part_range(&segments[part.start..part.end]);
                let prompt = render(
                    prompts.chunk,
                    &[
                        ("part", &(i + 1).to_string()),
                        ("parts", &count),
                        ("range", &range),
                        ("transcript", &lines[part.clone()].join("\n")),
                    ],
                    note,
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
        let part_notes: Vec<(String, Answer)> = stream::iter(part_requests)
            .map(|(range, req)| async move {
                Ok::<_, LlmError>((range, complete_in_full(provider, req).await?))
            })
            .buffered(provider.max_concurrency())
            .try_collect()
            .await?;

        // "Reduce": combine the part notes into the final summary.
        let notes = part_notes
            .iter()
            .enumerate()
            .map(|(i, (range, a))| format!("### Part {} ({range})\n\n{}", i + 1, notes_of(a)))
            .collect::<Vec<_>>()
            .join("\n\n");
        let prompt = render(
            prompts.reduce,
            &[
                ("date", date),
                ("duration", &duration),
                ("parts", &count),
                ("notes", &notes),
            ],
            note,
        )?;
        answers.extend(part_notes.into_iter().map(|(_, a)| a));
        answers.push(complete_in_full(provider, request(prompt, SUMMARY_MAX_TOKENS)).await?);
    }

    // The last answer is always the final summary. A part cut off short
    // loses that part's notes, so the summary is incomplete too.
    let last = answers.last().ok_or(SummaryError::EmptyTranscript)?;
    let complete = answers.iter().all(|a| !a.response.truncated);
    if !complete {
        tracing::warn!(
            requests = answers.iter().map(|a| a.requests).sum::<usize>(),
            "notes still stopped at the length limit; saved as incomplete"
        );
    }
    let responses = || answers.iter().map(|a| &a.response);
    Ok(Summary {
        markdown: notes_of(last),
        model: last.response.model.clone(),
        prompt_version: prompts.version,
        requests: answers.iter().map(|a| a.requests).sum(),
        prompt_tokens: responses().map(|r| u64::from(r.prompt_tokens)).sum(),
        completion_tokens: responses().map(|r| u64::from(r.completion_tokens)).sum(),
        // `Option<f64>: Sum` — None if any request didn't report a cost.
        cost_usd: responses().map(|r| r.cost_usd).sum(),
        latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        complete,
    })
}

/// Renders a summary template, with the vocabulary `note` for versions that
/// take one (rendering refuses a value the template has no place for).
fn render(
    template: &str,
    vars: &[(&str, &str)],
    note: Option<&str>,
) -> Result<String, quill_llm::prompts::RenderError> {
    let mut vars = vars.to_vec();
    if let Some(note) = note {
        vars.push(("vocabulary", note));
    }
    quill_llm::prompts::render(template, &vars)
}

/// Sends `req`, and while the model stops at `max_tokens`, asks it to carry
/// on (up to [`MAX_CONTINUATIONS`] times), joining the pieces into one
/// answer. Tokens and cost add up over every request; `truncated` says
/// whether the last one was still cut off.
async fn complete_in_full(
    provider: &dyn LlmProvider,
    mut req: CompletionRequest,
) -> Result<Answer, LlmError> {
    let asked = req.messages.len();
    let mut response = provider.complete(req.clone()).await?;
    let mut requests = 1;
    while response.truncated && requests <= MAX_CONTINUATIONS {
        tracing::info!(requests, "notes reached the length limit; continuing");
        req.messages.truncate(asked);
        req.messages.push(Message::assistant(clean(&response.text)));
        req.messages.push(Message::user(CONTINUE));
        let more = provider.complete(req.clone()).await?;
        requests += 1;
        response = CompletionResponse {
            text: response.text + without_think(&more.text),
            prompt_tokens: response.prompt_tokens + more.prompt_tokens,
            completion_tokens: response.completion_tokens + more.completion_tokens,
            cost_usd: response.cost_usd.zip(more.cost_usd).map(|(a, b)| a + b),
            latency_ms: response.latency_ms + more.latency_ms,
            ..more
        };
    }
    Ok(Answer { response, requests })
}

/// An answer's notes, cleaned. Notes cut off at the length limit end
/// mid-line; that line is dropped, so half an action item ("send the list
/// of names for") never reaches the action items list.
fn notes_of(answer: &Answer) -> String {
    let notes = clean(&answer.response.text);
    if !answer.response.truncated {
        return notes;
    }
    match notes.rfind('\n') {
        Some(at) => notes[..at].trim_end().to_string(),
        None => notes,
    }
}

/// A continuation's text without the empty `<think>` block some models
/// start every answer with. Unlike [`clean`], it keeps leading spaces: a
/// continuation may pick up mid-sentence.
fn without_think(text: &str) -> &str {
    let start = text.trim_start();
    match (start.strip_prefix("<think>"), start.find("</think>")) {
        (Some(_), Some(end)) => start[end + "</think>".len()..].trim_start_matches(['\r', '\n']),
        _ => text,
    }
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

/// Meeting length for the prompt: exact for short meetings (a 2:22 call is
/// "2 min 22 s", not "3 min", which a model will repeat back), rounded to
/// the nearest minute for longer ones.
pub(crate) fn format_duration(ms: u64) -> String {
    let secs = (ms + 500) / 1000;
    if secs < 10 * 60 {
        return match (secs / 60, secs % 60) {
            (0, s) => format!("{s} s"),
            (m, 0) => format!("{m} min"),
            (m, s) => format!("{m} min {s} s"),
        };
    }
    let mins = (secs + 30) / 60;
    if mins >= 60 {
        format!("{} h {} min", mins / 60, mins % 60)
    } else {
        format!("{mins} min")
    }
}

/// Removes an empty `<think>…</think>` block some reasoning models emit even
/// with thinking disabled, plus surrounding whitespace.
pub(crate) fn clean(text: &str) -> String {
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

    /// Fake provider: records requests and answers "answer N", except that
    /// its first `cut_off` answers stop at the length limit mid-line.
    struct FakeProvider {
        context: usize,
        cut_off: usize,
        requests: Mutex<Vec<CompletionRequest>>,
    }

    #[async_trait::async_trait]
    impl LlmProvider for FakeProvider {
        async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            let mut requests = self.requests.lock().unwrap();
            requests.push(req);
            let n = requests.len();
            let truncated = n <= self.cut_off;
            Ok(CompletionResponse {
                text: if truncated {
                    format!("<think>\n\n</think>\n\n- item {n}\n- send the")
                } else {
                    format!("<think>\n\n</think>\n\nanswer {n}")
                },
                model: "fake-1".into(),
                prompt_tokens: 100,
                completion_tokens: 10,
                cost_usd: Some(0.001),
                latency_ms: 5,
                truncated,
            })
        }

        fn context_tokens(&self) -> usize {
            self.context
        }

        /// One at a time, so which request is "first" doesn't vary.
        fn max_concurrency(&self) -> usize {
            1
        }
    }

    fn fake(context: usize) -> FakeProvider {
        FakeProvider {
            context,
            cut_off: 0,
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
        let summary = summarize(&provider, "m", &SUMMARY, &meeting(10), "2026-10-11", &[])
            .await
            .unwrap();

        assert_eq!(summary.requests, 1);
        assert_eq!(summary.markdown, "answer 1");
        assert!(summary.complete);
        assert_eq!(summary.prompt_version, SUMMARY.version);
        let requests = provider.requests.lock().unwrap();
        let user = &requests[0].messages[1].content;
        assert!(user.contains("[00:05] Others: Sentence number 1"), "{user}");
        assert!(user.contains("— 2026-10-11"));
    }

    #[tokio::test]
    async fn long_transcript_uses_map_reduce_in_order() {
        // ~7k tokens of transcript with a 7k context → 1k budget per part.
        let provider = fake(7_000);
        let summary = summarize(&provider, "m", &SUMMARY, &meeting(400), "2026-10-11", &[])
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
    async fn notes_cut_off_at_the_limit_are_continued() {
        let provider = FakeProvider {
            cut_off: 1,
            ..fake(100_000)
        };
        let summary = summarize(&provider, "m", &SUMMARY, &meeting(10), "2026-10-11", &[])
            .await
            .unwrap();

        assert!(summary.complete);
        assert_eq!(summary.requests, 2);
        assert_eq!(summary.markdown, "- item 1\n- send theanswer 2");
        assert_eq!(summary.completion_tokens, 20);
        assert!((summary.cost_usd.unwrap() - 0.002).abs() < 1e-9);
        // The model is shown what it wrote so far and asked to carry on.
        let requests = provider.requests.lock().unwrap();
        let again = &requests[1].messages;
        assert_eq!(again.len(), 4);
        assert_eq!(again[2], Message::assistant("- item 1\n- send the"));
        assert_eq!(again[3], Message::user(CONTINUE));
    }

    #[tokio::test]
    async fn notes_still_cut_off_are_incomplete_without_the_unfinished_line() {
        let provider = FakeProvider {
            cut_off: usize::MAX,
            ..fake(100_000)
        };
        let summary = summarize(&provider, "m", &SUMMARY, &meeting(10), "2026-10-11", &[])
            .await
            .unwrap();

        assert!(!summary.complete);
        assert_eq!(summary.requests, 1 + MAX_CONTINUATIONS);
        assert!(
            summary.markdown.starts_with("- item 1\n"),
            "{}",
            summary.markdown
        );
        assert!(
            summary.markdown.ends_with("- item 3"),
            "the unfinished line is dropped: {}",
            summary.markdown
        );
        // Each retry shows the whole answer so far, not a growing chat.
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests[2].messages.len(), 4);
    }

    #[tokio::test]
    async fn a_part_cut_off_makes_long_notes_incomplete() {
        let provider = FakeProvider {
            context: 7_000,
            // Part 1, and both times it's asked to carry on.
            cut_off: 1 + MAX_CONTINUATIONS,
            requests: Mutex::new(Vec::new()),
        };
        let summary = summarize(&provider, "m", &SUMMARY, &meeting(400), "2026-10-11", &[])
            .await
            .unwrap();
        assert!(!summary.complete);
    }

    #[test]
    fn continuations_keep_their_leading_space() {
        assert_eq!(without_think(" of names"), " of names");
        assert_eq!(
            without_think("<think>\n\n</think>\n\n of names"),
            " of names"
        );
    }

    #[tokio::test]
    async fn the_vocabulary_reaches_every_prompt_that_takes_it() {
        let words = ["Prall".to_string(), "Derik".to_string()];
        // One request; then map-reduce's parts and its final request.
        for (context, lines) in [(100_000, 10), (7_000, 400)] {
            let provider = fake(context);
            summarize(
                &provider,
                "m",
                &SUMMARY,
                &meeting(lines),
                "2026-10-11",
                &words,
            )
            .await
            .unwrap();
            for req in provider.requests.lock().unwrap().iter() {
                let user = &req.messages[1].content;
                assert!(user.contains("exactly as listed: Prall, Derik."), "{user}");
            }
        }
        // Older prompt versions have no place for it, and still render.
        let provider = fake(100_000);
        let v3 = quill_llm::prompts::SUMMARY_V3;
        summarize(&provider, "m", &v3, &meeting(10), "2026-10-11", &words)
            .await
            .unwrap();
        assert!(
            !provider.requests.lock().unwrap()[0].messages[1]
                .content
                .contains("Prall")
        );
    }

    #[tokio::test]
    async fn empty_transcript_is_an_error() {
        let result = summarize(&fake(100_000), "m", &SUMMARY, &[], "d", &[]).await;
        assert!(matches!(result, Err(SummaryError::EmptyTranscript)));
    }

    #[test]
    fn durations_are_exact_when_short_and_rounded_when_long() {
        assert_eq!(format_duration(141_838), "2 min 22 s");
        assert_eq!(format_duration(45_000), "45 s");
        assert_eq!(format_duration(180_000), "3 min");
        assert_eq!(format_duration(29 * 60_000 + 40_000), "30 min");
        assert_eq!(format_duration(77 * 60_000), "1 h 17 min");
    }

    #[test]
    fn splits_lines_within_budget() {
        let lines: Vec<String> = (0..10).map(|_| "x".repeat(30)).collect(); // ~11 tokens each
        let parts = split_lines(&lines, 25);
        assert_eq!(parts, vec![0..2, 2..4, 4..6, 6..8, 8..10]);
    }
}
