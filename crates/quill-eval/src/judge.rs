//! LLM-as-judge: score candidate summaries of the same transcript.
//!
//! For each case the judge sees all candidates at once, labelled A, B, C… in
//! a random order, with no model names, so it compares like with like and
//! can't favour a model by name or by position.

use std::collections::HashMap;

use quill_core::summary::transcript_line;
use quill_llm::prompts::{self, JUDGE};
use quill_llm::{CompletionRequest, LlmError, LlmProvider, Message};
use serde::{Deserialize, Serialize};

use crate::{Case, RunResult};

/// Rubric scores, each 1–10 (see prompts/judge-v1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scores {
    pub completeness: f64,
    pub accuracy: f64,
    pub action_items: f64,
    /// 10 = nothing invented (every claim supported by the transcript).
    pub faithfulness: f64,
    pub formatting: f64,
    pub overall: f64,
    #[serde(default)]
    pub notes: String,
}

/// What judging cost, summed over all cases.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JudgeUsage {
    pub requests: usize,
    pub cost_usd: Option<f64>,
    /// Cases the judge couldn't score (error or unparseable reply).
    pub failures: Vec<String>,
}

/// Scores every successful result in `results`, case by case, writing the
/// scores into the results. `seed` makes the label shuffle reproducible.
pub async fn judge_all(
    provider: &dyn LlmProvider,
    model: &str,
    cases: &[Case],
    results: &mut [RunResult],
    seed: u64,
) -> JudgeUsage {
    let mut usage = JudgeUsage {
        cost_usd: Some(0.0),
        ..JudgeUsage::default()
    };
    for (case_index, case) in cases.iter().enumerate() {
        // Indices into `results` of this case's successful summaries.
        let candidates: Vec<usize> = results
            .iter()
            .enumerate()
            .filter(|(_, r)| r.case == case.name && r.outcome.is_ok())
            .map(|(i, _)| i)
            .collect();
        if candidates.is_empty() {
            continue;
        }
        let order = shuffled(candidates.len(), seed.wrapping_add(case_index as u64));
        let texts: Vec<&str> = order
            .iter()
            .map(|&i| {
                results[candidates[i]]
                    .outcome
                    .as_ref()
                    .map_or("", |s| s.markdown.as_str())
            })
            .collect();

        match judge_case(provider, model, case, &texts).await {
            Ok((scores, cost)) => {
                usage.requests += 1;
                usage.cost_usd = usage.cost_usd.zip(cost).map(|(a, b)| a + b);
                // Label k (A, B, …) is candidate order[k].
                for (k, s) in scores.into_iter().enumerate() {
                    results[candidates[order[k]]].scores = Some(s);
                }
            }
            Err(e) => {
                tracing::warn!(case = %case.name, "judge failed: {e}");
                usage.failures.push(format!("{}: {e}", case.name));
            }
        }
    }
    usage
}

#[derive(Debug, thiserror::Error)]
pub enum JudgeError {
    #[error(transparent)]
    Llm(#[from] LlmError),
    #[error(transparent)]
    Prompt(#[from] prompts::RenderError),
    #[error("judge reply wasn't the expected JSON: {0}")]
    BadReply(String),
}

/// Asks the judge to score `summaries` (in label order A, B, …). Returns one
/// `Scores` per summary, in the same order, plus the request's cost.
async fn judge_case(
    provider: &dyn LlmProvider,
    model: &str,
    case: &Case,
    summaries: &[&str],
) -> Result<(Vec<Scores>, Option<f64>), JudgeError> {
    let labels: Vec<String> = (0..summaries.len()).map(label).collect();
    let transcript = quill_core::meeting::turns(&case.segments)
        .iter()
        .map(transcript_line)
        .collect::<Vec<_>>()
        .join("\n");
    let candidates = labels
        .iter()
        .zip(summaries)
        .map(|(l, s)| format!("<candidate label=\"{l}\">\n{s}\n</candidate>"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let user = prompts::render(
        JUDGE.user,
        &[
            ("labels", &labels.join(", ")),
            ("transcript", &transcript),
            ("candidates", &candidates),
        ],
    )?;

    let response = provider
        .complete(CompletionRequest {
            model: model.to_string(),
            messages: vec![Message::system(JUDGE.system), Message::user(user)],
            // Room for notes on several candidates (and any preamble a model
            // adds despite instructions).
            max_tokens: Some(4096),
            temperature: Some(0.0),
            no_reasoning: false,
        })
        .await?;

    let mut by_label = parse_reply(&response.text)?;
    let scores = labels
        .iter()
        .map(|l| {
            by_label
                .remove(l)
                .ok_or_else(|| JudgeError::BadReply(format!("no scores for candidate {l}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((scores, response.cost_usd))
}

/// Extracts the JSON object from the reply, tolerating code fences or a
/// sentence before/after it.
fn parse_reply(text: &str) -> Result<HashMap<String, Scores>, JudgeError> {
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        return Err(JudgeError::BadReply(snippet(text)));
    };
    serde_json::from_str(&text[start..=end])
        .map_err(|e| JudgeError::BadReply(format!("{e}: {}", snippet(text))))
}

fn snippet(text: &str) -> String {
    text.chars().take(200).collect()
}

/// A, B, … Z, then AA, AB… (more than 26 candidates per case is unlikely).
fn label(i: usize) -> String {
    if i < 26 {
        char::from(b'A' + i as u8).to_string()
    } else {
        format!("{}{}", label(i / 26 - 1), label(i % 26))
    }
}

/// A permutation of `0..n` from a seed: Fisher–Yates driven by a small
/// xorshift generator. Not cryptographic; it only needs to be unpredictable
/// to the judge and reproducible from the seed.
fn shuffled(n: usize, seed: u64) -> Vec<usize> {
    // Scramble the seed first (SplitMix64's finalizer) so nearby seeds give
    // unrelated orders; then force it non-zero, where xorshift gets stuck.
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    let mut state = (z ^ (z >> 31)) | 1;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut order: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Fake;
    use quill_core::summary::Summary;
    use quill_stt::Segment;

    fn summary(markdown: &str) -> Summary {
        Summary {
            markdown: markdown.into(),
            model: "m".into(),
            prompt_version: "summary-v1",
            requests: 1,
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: Some(0.0),
            latency_ms: 0,
        }
    }

    fn result(model: &str, markdown: &str) -> RunResult {
        RunResult {
            case: "c".into(),
            case_hours: 1.0,
            model: model.into(),
            prompt_version: "summary-v1",
            outcome: Ok(summary(markdown)),
            scores: None,
        }
    }

    #[test]
    fn shuffle_is_a_reproducible_permutation() {
        let a = shuffled(8, 42);
        let mut sorted = a.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..8).collect::<Vec<_>>());
        assert_eq!(a, shuffled(8, 42));
        // Neighbouring seeds (consecutive cases use seed + i) must differ.
        assert_ne!(a, shuffled(8, 43));
        assert_ne!(shuffled(8, 0), shuffled(8, 1));
    }

    #[test]
    fn labels() {
        assert_eq!(label(0), "A");
        assert_eq!(label(25), "Z");
        assert_eq!(label(26), "AA");
    }

    #[test]
    fn parses_fenced_json_with_preamble() {
        let reply = "Here are my scores:\n```json\n{\"A\": {\"completeness\": 8, \"accuracy\": 9, \
                     \"action_items\": 7, \"faithfulness\": 10, \"formatting\": 9, \"overall\": 8.5, \
                     \"notes\": \"Solid.\"}}\n```";
        let parsed = parse_reply(reply).unwrap();
        assert_eq!(parsed["A"].overall, 8.5);
        assert_eq!(parsed["A"].notes, "Solid.");
    }

    /// The judge's scores land on the right results despite the shuffle, and
    /// failed results are never shown to the judge.
    #[tokio::test]
    async fn maps_shuffled_labels_back_to_models() {
        // The fake judge scores each candidate by the digit in its text, so
        // we can check every score reached the summary it was about.
        let judge = Fake::new(|req: &CompletionRequest| {
            let user = &req.messages[1].content;
            let mut out = serde_json::Map::new();
            for part in user.split("<candidate label=\"").skip(1) {
                let (label, rest) = part.split_once("\">\n").unwrap();
                let digit: f64 = rest[..1].parse().unwrap();
                out.insert(
                    label.into(),
                    serde_json::json!({"completeness": digit, "accuracy": digit, "action_items": digit,
                        "faithfulness": digit, "formatting": digit, "overall": digit}),
                );
            }
            Ok(serde_json::Value::Object(out).to_string())
        });
        let case = Case {
            name: "c".into(),
            segments: vec![Segment {
                start_ms: 0,
                end_ms: 1000,
                text: "hi".into(),
                speaker: None,
            }],
        };
        let mut results = vec![
            result("m3", "3 notes"),
            result("m5", "5 notes"),
            RunResult {
                outcome: Err("boom".into()),
                ..result("failed", "")
            },
            result("m7", "7 notes"),
        ];

        let usage = judge_all(&judge, "judge", &[case], &mut results, 7).await;

        assert_eq!(usage.requests, 1);
        assert!(usage.failures.is_empty());
        let overall = |m: &str| {
            results
                .iter()
                .find(|r| r.model == m)
                .and_then(|r| r.scores.as_ref())
                .map(|s| s.overall)
        };
        assert_eq!(overall("m3"), Some(3.0));
        assert_eq!(overall("m5"), Some(5.0));
        assert_eq!(overall("m7"), Some(7.0));
        assert_eq!(overall("failed"), None);
        // Model names never reach the judge.
        let prompt = &judge.requests.lock().unwrap()[0].messages[1].content;
        assert!(!prompt.contains("m3") && !prompt.contains("failed"));
    }
}
