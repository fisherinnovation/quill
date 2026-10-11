//! LLM access for Quill: the `LlmProvider` trait, OpenRouter and Ollama /
//! LM Studio implementations, and prompt rendering.
//!
//! Providers are chosen by a model spec string (see [`provider_for`]):
//! `anthropic/claude-haiku-5.5` goes to OpenRouter, `ollama:qwen3:8b` to a
//! local Ollama server.

pub mod keys;
mod ollama;
mod openrouter;
pub mod prompts;

pub use ollama::Ollama;
pub use openrouter::{OpenRouter, Privacy};

use serde::{Deserialize, Serialize};

/// Who said a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    /// Provider-specific model id, e.g. `anthropic/claude-haiku-5.5` or `qwen3:8b`.
    pub model: String,
    pub messages: Vec<Message>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompletionResponse {
    pub text: String,
    /// The model that actually answered (may differ from the request when a
    /// provider resolves aliases).
    pub model: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// Cost in USD as reported by the provider; `Some(0.0)` for local models,
    /// `None` if the provider didn't say.
    pub cost_usd: Option<f64>,
    pub latency_ms: u64,
}

/// Anything that can complete a chat conversation.
///
/// `async fn` in a trait works in plain Rust, but such a trait can't be used
/// as `dyn LlmProvider` (each implementation's future is a different,
/// unnameable type). `#[async_trait]` rewrites the method to return a boxed
/// future, `Pin<Box<dyn Future + Send>>`, which is the same type for every
/// implementation, so the CLI can pick a provider at runtime.
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError>;

    /// Roughly how many tokens of input this provider/model can take, used to
    /// decide between single-pass and map-reduce summarization.
    fn context_tokens(&self) -> usize;

    /// How many requests to run at once (map-reduce parts). Cloud APIs
    /// handle several; a local model on one GPU is fastest one at a time.
    fn max_concurrency(&self) -> usize {
        4
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{provider} returned {status}: {message}")]
    Api {
        provider: &'static str,
        status: u16,
        message: String,
    },
    #[error("unexpected response from {provider}: {detail}")]
    Decode {
        provider: &'static str,
        detail: String,
    },
    #[error("no API key for {0}; run: quill-cli keys set {0}")]
    MissingKey(&'static str),
    #[error("keychain error: {0}")]
    Keyring(#[from] keyring::Error),
}

/// Picks a provider from a model spec:
/// - `ollama:<model>` → local Ollama (`OLLAMA_HOST` or `http://localhost:11434`)
/// - anything else → OpenRouter, with the key from the keychain
///
/// Returns the provider and the model id to send it.
pub fn provider_for(
    spec: &str,
    privacy: Privacy,
) -> Result<(Box<dyn LlmProvider>, String), LlmError> {
    if let Some(model) = spec.strip_prefix("ollama:") {
        let host = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost:11434".into());
        return Ok((Box::new(Ollama::new(host)), model.to_string()));
    }
    let key = keys::openrouter_key()?;
    Ok((Box::new(OpenRouter::new(key, privacy)), spec.to_string()))
}

/// Rough token count for budgeting. English averages ~4 characters per
/// token; dividing by 3 overestimates on purpose, so a transcript we think
/// fits really does. Exact counts come back in the response usage.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(3)
}
