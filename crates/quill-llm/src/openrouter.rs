//! OpenRouter: one OpenAI-compatible API in front of hundreds of models.
//!
//! Privacy: by default we ask OpenRouter to route only to providers that
//! don't collect (store or train on) request data. Zero-data-retention is
//! stricter and optional, since fewer endpoints offer it.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;

use crate::{CompletionRequest, CompletionResponse, LlmError, LlmProvider};

const PROVIDER: &str = "OpenRouter";
const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// Data-handling requirements sent with every request (`provider` object).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Privacy {
    /// `data_collection: "deny"`: skip providers that store or train on data.
    pub deny_data_collection: bool,
    /// `zdr: true`: only Zero Data Retention endpoints.
    pub zero_data_retention: bool,
}

impl Default for Privacy {
    fn default() -> Self {
        Self {
            deny_data_collection: true,
            zero_data_retention: false,
        }
    }
}

pub struct OpenRouter {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    privacy: Privacy,
    /// Models that refused `no_reasoning` (their reasoning is mandatory),
    /// so later requests don't pay for the failed attempt again.
    reasoning_required: Mutex<HashSet<String>>,
}

impl OpenRouter {
    pub fn new(api_key: String, privacy: Privacy) -> Self {
        Self::with_base_url(api_key, privacy, DEFAULT_BASE_URL)
    }

    /// For tests: point at a mock server instead of openrouter.ai.
    pub fn with_base_url(api_key: String, privacy: Privacy, base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            // Long summaries can take a few minutes on slower models.
            .timeout(Duration::from_secs(600))
            .build()
            .unwrap_or_default();
        Self {
            client,
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            privacy,
            reasoning_required: Mutex::new(HashSet::new()),
        }
    }

    fn reasoning_required(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.reasoning_required
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn body(&self, req: &CompletionRequest, no_reasoning: bool) -> serde_json::Value {
        let mut provider = serde_json::Map::new();
        if self.privacy.deny_data_collection {
            provider.insert("data_collection".into(), json!("deny"));
        }
        if self.privacy.zero_data_retention {
            provider.insert("zdr".into(), json!(true));
        }
        let mut body = json!({
            "model": req.model,
            "messages": req.messages,
            "provider": provider,
        });
        if let Some(max_tokens) = req.max_tokens {
            body["max_tokens"] = json!(max_tokens);
        }
        if let Some(temperature) = req.temperature {
            body["temperature"] = json!(temperature);
        }
        if no_reasoning {
            // OpenRouter's unified switch. Models whose reasoning is
            // mandatory reject it (see `complete`).
            body["reasoning"] = json!({ "enabled": false });
        }
        body
    }
}

// Only the fields we use. Usage (tokens and cost) is included in every
// response; `usage.cost` is in OpenRouter credits, which are US dollars.
#[derive(Deserialize)]
struct ChatResponse {
    model: String,
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
    /// "stop", or "length" when `max_tokens` cut the answer off.
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    // Null when a reasoning model ran out of tokens before answering.
    content: Option<String>,
}

#[derive(Deserialize)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
    cost: Option<f64>,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: String,
}

#[async_trait::async_trait]
impl LlmProvider for OpenRouter {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let no_reasoning = req.no_reasoning && !self.reasoning_required().contains(&req.model);
        match self.send(&req, no_reasoning).await {
            Err(LlmError::Api {
                status: 400,
                message,
                ..
            }) if no_reasoning && message.contains("easoning is mandatory") => {
                tracing::debug!(model = req.model, "reasoning can't be disabled; retrying");
                self.reasoning_required().insert(req.model.clone());
                self.send(&req, false).await
            }
            result => result,
        }
    }

    fn context_tokens(&self) -> usize {
        // Most current models take 200k–1M tokens. Stay well under the
        // smallest so one request never fails for size; longer transcripts
        // (several hours) go through map-reduce.
        100_000
    }
}

impl OpenRouter {
    async fn send(
        &self,
        req: &CompletionRequest,
        no_reasoning: bool,
    ) -> Result<CompletionResponse, LlmError> {
        let started = Instant::now();
        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            // Optional OpenRouter attribution headers.
            .header("X-Title", "Quill")
            .json(&self.body(req, no_reasoning))
            .send()
            .await?;

        let status = response.status();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            let message = serde_json::from_slice::<ErrorBody>(&bytes)
                .map(|e| e.error.message)
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
        let choice = parsed.choices.into_iter().next().ok_or(LlmError::Decode {
            provider: PROVIDER,
            detail: "no choices in response".into(),
        })?;
        let truncated = choice.finish_reason.as_deref() == Some("length");
        let text = choice.message.content.ok_or(LlmError::Decode {
            provider: PROVIDER,
            detail: "the model returned no text (a reasoning model may have spent its \
                         token budget thinking)"
                .into(),
        })?;
        let usage = parsed.usage;
        Ok(CompletionResponse {
            text,
            model: parsed.model,
            prompt_tokens: usage.as_ref().map_or(0, |u| u.prompt_tokens),
            completion_tokens: usage.as_ref().map_or(0, |u| u.completion_tokens),
            cost_usd: usage.and_then(|u| u.cost),
            latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn request() -> CompletionRequest {
        CompletionRequest {
            model: "anthropic/claude-haiku-5.5".into(),
            messages: vec![Message::system("Be brief."), Message::user("Hi")],
            max_tokens: Some(100),
            temperature: Some(0.2),
            no_reasoning: false,
        }
    }

    #[tokio::test]
    async fn sends_privacy_preferences_and_reads_usage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({
                "model": "anthropic/claude-haiku-5.5",
                "messages": [{"role": "system", "content": "Be brief."}, {"role": "user", "content": "Hi"}],
                "max_tokens": 100,
                "provider": {"data_collection": "deny", "zdr": true},
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "anthropic/claude-haiku-5.5-20260901",
                "choices": [{"message": {"role": "assistant", "content": "Hello!"}}],
                "usage": {"prompt_tokens": 12, "completion_tokens": 3, "total_tokens": 15, "cost": 0.0000027},
            })))
            .expect(1)
            .mount(&server)
            .await;

        let privacy = Privacy {
            deny_data_collection: true,
            zero_data_retention: true,
        };
        let provider = OpenRouter::with_base_url("sk-test".into(), privacy, &server.uri());
        let response = provider.complete(request()).await.unwrap();

        assert_eq!(response.text, "Hello!");
        assert_eq!(response.model, "anthropic/claude-haiku-5.5-20260901");
        assert_eq!(
            (response.prompt_tokens, response.completion_tokens),
            (12, 3)
        );
        assert_eq!(response.cost_usd, Some(0.0000027));
        assert!(!response.truncated);
    }

    #[tokio::test]
    async fn reports_an_answer_cut_off_at_max_tokens() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "m",
                "choices": [{
                    "message": {"role": "assistant", "content": "## Action items\n- **Dan** — send the"},
                    "finish_reason": "length",
                }],
            })))
            .mount(&server)
            .await;
        let provider =
            OpenRouter::with_base_url("sk-test".into(), Privacy::default(), &server.uri());
        let response = provider.complete(request()).await.unwrap();
        assert!(response.truncated);
        assert!(response.text.ends_with("send the"));
    }

    #[tokio::test]
    async fn drops_no_reasoning_for_models_that_require_it_and_remembers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"reasoning": {"enabled": false}})))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": {"code": 400, "message": "Reasoning is mandatory for this endpoint and cannot be disabled."},
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "m",
                "choices": [{"message": {"role": "assistant", "content": "ok"}}],
            })))
            .expect(2)
            .mount(&server)
            .await;

        let provider =
            OpenRouter::with_base_url("sk-test".into(), Privacy::default(), &server.uri());
        let req = CompletionRequest {
            no_reasoning: true,
            ..request()
        };
        // First call: refused, retried without the flag. Second: no flag.
        assert_eq!(provider.complete(req.clone()).await.unwrap().text, "ok");
        assert_eq!(provider.complete(req).await.unwrap().text, "ok");
    }

    #[tokio::test]
    async fn null_content_is_an_error_not_a_crash() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "model": "m",
                "choices": [{"message": {"role": "assistant", "content": null}}],
            })))
            .mount(&server)
            .await;
        let provider =
            OpenRouter::with_base_url("sk-test".into(), Privacy::default(), &server.uri());
        let err = provider.complete(request()).await.unwrap_err();
        assert!(err.to_string().contains("returned no text"), "{err}");
    }

    #[tokio::test]
    async fn surfaces_api_error_messages() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(404).set_body_json(json!({
                "error": {"code": 404, "message": "No endpoints found matching your data policy"},
            })))
            .mount(&server)
            .await;

        let provider =
            OpenRouter::with_base_url("sk-test".into(), Privacy::default(), &server.uri());
        let err = provider.complete(request()).await.unwrap_err();

        assert_eq!(
            err.to_string(),
            "OpenRouter returned 404: No endpoints found matching your data policy"
        );
    }
}
