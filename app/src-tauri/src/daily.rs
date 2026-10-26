//! Daily summaries in the app: listing them for the Meetings page, and
//! writing one when the user asks ("Summarize this day"). The logic is in
//! `quill_core::daily`; the summary model is the one meeting notes use.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Instant;

use jiff::civil::Date;
use quill_core::daily::{self, DailySummary};
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::commands::{CommandError, CommandResult, lock};

/// Days whose summary is being written, so a second click doesn't start
/// another.
static WRITING: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Every stored daily summary, each marked outdated if its day's notes
/// have changed since.
#[tauri::command]
pub fn daily_summaries(state: tauri::State<'_, AppState>) -> CommandResult<Vec<DailySummary>> {
    Ok(state.library.daily_summaries()?)
}

/// Writes `day`'s summary (`YYYY-MM-DD`, local) from its meetings' notes,
/// replacing any before, and returns it.
#[tauri::command]
pub async fn summarize_day(app: AppHandle, day: String) -> CommandResult<DailySummary> {
    let date: Date = day
        .parse()
        .map_err(|_| CommandError::Other(format!("not a day: {day}")))?;
    if !lock(&WRITING)
        .get_or_insert_with(HashSet::new)
        .insert(day.clone())
    {
        return Err(CommandError::Other(
            "That day's summary is already being written.".into(),
        ));
    }
    let result = write(&app, date).await;
    if let Some(writing) = lock(&WRITING).as_mut() {
        writing.remove(&day);
    }
    result
}

async fn write(app: &AppHandle, day: Date) -> CommandResult<DailySummary> {
    let state = app.state::<AppState>();
    let sources = state.library.day_sources(day)?;
    let settings = lock(&state.settings).clone();
    let (provider, model) =
        crate::pipeline::notes_provider(app, &settings).map_err(CommandError::Other)?;
    let started = Instant::now();
    let summary = daily::summarize_day(
        provider.as_ref(),
        &model,
        &quill_llm::prompts::DAILY,
        day,
        &sources,
    )
    .await
    .map_err(|e| {
        tracing::warn!(
            meetings = sources.len(),
            ms = started.elapsed().as_millis(),
            "couldn't write a daily summary: {e}"
        );
        CommandError::Other(e.to_string())
    })?;
    tracing::info!(
        meetings = sources.len(),
        ms = started.elapsed().as_millis(),
        prompt_tokens = summary.prompt_tokens,
        completion_tokens = summary.completion_tokens,
        cost_usd = summary.cost_usd,
        "wrote a daily summary"
    );
    Ok(state.library.save_daily_summary(day, &sources, &summary)?)
}
