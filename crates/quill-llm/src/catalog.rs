//! What models are available: OpenRouter's public catalog and the models
//! installed in the local Ollama. Used to offer real choices in Settings
//! instead of a free-text model id.
//!
//! Neither call sends anything about the user: OpenRouter's model list is
//! public (no key), and Ollama runs on this computer.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::LlmError;

/// One model in OpenRouter's catalog.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Listing {
    pub id: String,
    pub name: String,
    /// US dollars per million tokens, if listed.
    pub input_per_million: Option<f64>,
    pub output_per_million: Option<f64>,
    pub context_length: Option<u64>,
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<RawModel>,
}

#[derive(Deserialize)]
struct RawModel {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    pricing: Option<RawPricing>,
    #[serde(default)]
    context_length: Option<u64>,
}

#[derive(Deserialize)]
struct RawPricing {
    /// Dollars per token, as a decimal string.
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
}

const OPENROUTER_MODELS: &str = "https://openrouter.ai/api/v1/models";

/// OpenRouter's catalog, sorted by id.
pub async fn openrouter_models() -> Result<Vec<Listing>, LlmError> {
    openrouter_models_from(OPENROUTER_MODELS).await
}

async fn openrouter_models_from(url: &str) -> Result<Vec<Listing>, LlmError> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()?;
    let response = client.get(url).send().await?.error_for_status()?;
    let body: ModelsResponse = response.json().await?;
    let per_million = |price: Option<&String>| {
        price
            .and_then(|p| p.parse::<f64>().ok())
            .filter(|p| *p >= 0.0)
            .map(|p| p * 1_000_000.0)
    };
    let mut listings: Vec<Listing> = body
        .data
        .into_iter()
        .map(|m| Listing {
            name: if m.name.is_empty() {
                m.id.clone()
            } else {
                m.name
            },
            input_per_million: per_million(m.pricing.as_ref().and_then(|p| p.prompt.as_ref())),
            output_per_million: per_million(m.pricing.as_ref().and_then(|p| p.completion.as_ref())),
            context_length: m.context_length,
            id: m.id,
        })
        .collect();
    listings.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(listings)
}

#[derive(Deserialize)]
struct TagsResponse {
    #[serde(default)]
    models: Vec<Tag>,
}

#[derive(Deserialize)]
struct Tag {
    name: String,
}

/// The models installed in Ollama at `host`, e.g. `["qwen3:8b"]`. An error
/// usually means Ollama isn't running.
pub async fn ollama_models(host: &str) -> Result<Vec<String>, LlmError> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(1))
        .timeout(Duration::from_secs(3))
        .build()?;
    let url = format!("{}/api/tags", host.trim_end_matches('/'));
    let response = client.get(url).send().await?.error_for_status()?;
    let body: TagsResponse = response.json().await?;
    let mut names: Vec<String> = body.models.into_iter().map(|t| t.name).collect();
    names.sort();
    Ok(names)
}

/// Where Ollama is: `OLLAMA_HOST`, or the default local address.
pub fn ollama_host() -> String {
    std::env::var("OLLAMA_HOST")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|h| {
            if h.starts_with("http") {
                h
            } else {
                format!("http://{h}")
            }
        })
        .unwrap_or_else(|| "http://localhost:11434".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn reads_openrouter_catalog_with_prices_per_million() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"id": "z/model", "name": "Z", "pricing": {"prompt": "0.0000001", "completion": "0.0000005"}, "context_length": 1000000},
                    {"id": "a/model", "pricing": {"prompt": "-1"}}
                ]
            })))
            .mount(&server)
            .await;
        let list = openrouter_models_from(&format!("{}/models", server.uri()))
            .await
            .unwrap();
        assert_eq!(list[0].id, "a/model");
        assert_eq!(list[0].name, "a/model");
        assert_eq!(list[0].input_per_million, None);
        let z = &list[1];
        assert!((z.input_per_million.unwrap() - 0.1).abs() < 1e-9);
        assert!((z.output_per_million.unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(z.context_length, Some(1_000_000));
    }

    #[tokio::test]
    async fn lists_installed_ollama_models() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [{"name": "qwen3:8b"}, {"name": "llama3.3:70b"}]
            })))
            .mount(&server)
            .await;
        assert_eq!(
            ollama_models(&server.uri()).await.unwrap(),
            ["llama3.3:70b", "qwen3:8b"]
        );
    }

    #[tokio::test]
    async fn ollama_not_running_is_an_error() {
        // Nothing listens on port 9 (discard) here.
        assert!(ollama_models("http://127.0.0.1:9").await.is_err());
    }
}
