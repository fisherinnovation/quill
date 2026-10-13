//! The model choices Settings offers for summaries and dictation clean-up:
//! recommended models with live prices, the models installed in Ollama, and
//! OpenRouter's catalog for anything else. Replaces typing a model id from
//! memory.

use std::sync::Arc;
use std::time::{Duration, Instant};

use quill_core::llm_choices::{self, Recommended};
use quill_llm::catalog::{self, Listing};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::commands::{CommandResult, lock};

/// OpenRouter's catalog is fetched at most this often.
const CATALOG_TTL: Duration = Duration::from_secs(60 * 60);

/// The cached catalog, shared by both pickers.
#[derive(Default)]
pub struct CatalogCache(std::sync::Mutex<Option<(Instant, Arc<Vec<Listing>>)>>);

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Summary,
    Cleanup,
}

#[derive(Debug, Clone, Serialize)]
pub struct Choice {
    pub id: &'static str,
    pub label: &'static str,
    pub note: &'static str,
    /// Dollars to summarize an hour-long meeting (summary models only),
    /// when OpenRouter lists the model's prices.
    pub per_hour: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelChoices {
    pub recommended: Vec<Choice>,
    /// Installed Ollama models (as `ollama:<name>`), or `None` if Ollama
    /// isn't running.
    pub ollama: Option<Vec<String>>,
    /// OpenRouter's catalog; empty if it couldn't be fetched.
    pub catalog: Arc<Vec<Listing>>,
}

#[tauri::command]
pub async fn model_choices(app: AppHandle, purpose: Purpose) -> CommandResult<ModelChoices> {
    let catalog = cached_catalog(&app).await;
    // A refused connection (Ollama not running) fails at once.
    let ollama = catalog::ollama_models(&catalog::ollama_host()).await;
    let recommended: &[Recommended] = match purpose {
        Purpose::Summary => llm_choices::SUMMARY,
        Purpose::Cleanup => llm_choices::CLEANUP,
    };
    let recommended = recommended
        .iter()
        .map(|r| Choice {
            id: r.id,
            label: r.label,
            note: r.note,
            per_hour: match purpose {
                Purpose::Summary => catalog.iter().find(|l| l.id == r.id).and_then(|l| {
                    Some(llm_choices::cost_per_hour(
                        l.input_per_million?,
                        l.output_per_million?,
                    ))
                }),
                Purpose::Cleanup => None,
            },
        })
        .collect();
    Ok(ModelChoices {
        recommended,
        ollama: ollama
            .ok()
            .map(|names| names.into_iter().map(|n| format!("ollama:{n}")).collect()),
        catalog,
    })
}

/// OpenRouter's catalog, refetched once it's an hour old. A failed fetch
/// gives an empty list (the picker still works, it just can't check ids).
async fn cached_catalog(app: &AppHandle) -> Arc<Vec<Listing>> {
    let cache = &app.state::<CatalogCache>().0;
    if let Some((at, list)) = lock(cache).as_ref()
        && at.elapsed() < CATALOG_TTL
    {
        return Arc::clone(list);
    }
    match catalog::openrouter_models().await {
        Ok(list) => {
            let list = Arc::new(list);
            *lock(cache) = Some((Instant::now(), Arc::clone(&list)));
            list
        }
        Err(e) => {
            tracing::warn!("couldn't fetch OpenRouter's model list: {e}");
            lock(cache)
                .as_ref()
                .map_or_else(|| Arc::new(Vec::new()), |(_, list)| Arc::clone(list))
        }
    }
}
