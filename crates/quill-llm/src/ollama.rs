//! Ollama: local models, nothing leaves the machine.
//!
//! Uses Ollama's native `/api/chat` rather than its OpenAI-compatible
//! endpoint, because only the native API lets us set the context window
//! (`num_ctx`). Ollama's default window is small, and input beyond it is
//! silently cut off — a whole meeting transcript would lose its beginning.

use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;

use crate::{CompletionRequest, CompletionResponse, LlmError, LlmProvider, Message};

const PROVIDER: &str = "Ollama";

/// Context window we request. 32k tokens covers ~1.5 hours of meeting in one
/// pass and fits comfortably in memory for 8B-class models; longer meetings
/// use map-reduce.
const NUM_CTX: usize = 32_768;

pub struct Ollama {
    client: reqwest::Client,
    base_url: String,
}

impl Ollama {
    pub fn new(base_url: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            // Local generation of a long summary can take minutes.
            .timeout(Duration::from_secs(1800))
            .build()
            .unwrap_or_default();
        Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }
}

#[derive(Deserialize)]
struct ChatResponse {
    model: String,
    message: Message,
    #[serde(default)]
    prompt_eval_count: u32,
    #[serde(default)]
    eval_count: u32,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

#[async_trait::async_trait]
impl LlmProvider for Ollama {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut options = json!({ "num_ctx": NUM_CTX });
        if let Some(t) = req.temperature {
            options["temperature"] = json!(t);
        }
        if let Some(n) = req.max_tokens {
            options["num_predict"] = json!(n);
        }
        let body = json!({
            "model": req.model,
            "messages": req.messages,
            "stream": false,
            // Reasoning models (Qwen3, DeepSeek-R1) would otherwise spend
            // minutes "thinking" before a summary; it doesn't help here.
            "think": false,
            "options": options,
        });

        let started = Instant::now();
        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            let message = serde_json::from_slice::<ErrorBody>(&bytes)
                .map(|e| e.error)
                .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned());
            return Err(LlmError::Api {
                provider: PROVIDER,
                status: status.as_u16(),
                message,
            });
        }
        let parsed: ChatResponse =
            serde_json::from_slice(&bytes).map_err(|e| LlmError::Decode {
                provider: PROVIDER,
                detail: e.to_string(),
            })?;

        Ok(CompletionResponse {
            text: parsed.message.content,
            model: parsed.model,
            prompt_tokens: parsed.prompt_eval_count,
            completion_tokens: parsed.eval_count,
            cost_usd: Some(0.0),
            latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }

    fn context_tokens(&self) -> usize {
        NUM_CTX
    }

    fn max_concurrency(&self) -> usize {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn sets_context_window_and_disables_thinking() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/chat"))
            .and(body_partial_json(json!({
                "model": "qwen3:8b",
                "stream": false,
                "think": false,
                "options": {"num_ctx": NUM_CTX, "temperature": 0.5},
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "qwen3:8b",
                "message": {"role": "assistant", "content": "Summary."},
                "done": true,
                "prompt_eval_count": 1200,
                "eval_count": 150,
            })))
            .expect(1)
            .mount(&server)
            .await;

        let response = Ollama::new(server.uri())
            .complete(CompletionRequest {
                model: "qwen3:8b".into(),
                messages: vec![Message::user("Summarize.")],
                max_tokens: None,
                temperature: Some(0.5),
            })
            .await
            .unwrap();

        assert_eq!(response.text, "Summary.");
        assert_eq!(
            (response.prompt_tokens, response.completion_tokens),
            (1200, 150)
        );
        assert_eq!(response.cost_usd, Some(0.0));
    }
}
