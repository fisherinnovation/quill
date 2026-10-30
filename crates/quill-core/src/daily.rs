//! Daily summaries: one summary of a day's meetings, written from each
//! meeting's latest notes when the user asks (the Meetings page's
//! "Summarize this day").
//!
//! Built from notes, not transcripts: the notes already hold what each
//! meeting came to, so a day is one short request (and nothing goes to the
//! model that its notes didn't already send). A summary records which notes
//! it was written from ([`sources`]), so new or rewritten notes, or another
//! meeting that day, mark it outdated. It holds its meetings' content, so
//! deleting any of them deletes it (migration 13's trigger).

use jiff::civil::Date;
use quill_llm::prompts::DailyPrompts;
use quill_llm::{CompletionRequest, LlmProvider, Message};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::library::{Library, LibraryError};
use crate::summary::{Summary, SummaryError, clean, format_duration};

/// Output budget for the day's summary, as for a meeting's notes. Models
/// whose reasoning can't be turned off spend part of it thinking.
const MAX_TOKENS: u32 = 4096;
/// As for meeting notes: factual and consistent.
const TEMPERATURE: f32 = 0.2;
/// Room for the system prompt, the template and the answer.
const RESERVED_TOKENS: usize = 4_000;
/// The sections of each meeting's notes kept when the whole day's notes are
/// too long for the model (lowercase prefixes of `##` headings).
const ESSENTIAL_SECTIONS: &[&str] = &["tl;dr", "decision", "action item", "open question"];

/// One meeting's notes, as a daily summary uses them.
#[derive(Debug, Clone, PartialEq)]
pub struct DaySource {
    pub meeting_id: String,
    pub title: String,
    pub started_at: String,
    pub duration_ms: u64,
    /// The notes' row in `summaries`: new notes get a new one.
    pub summary_id: i64,
    pub notes: String,
}

/// A stored daily summary.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DailySummary {
    /// The local day, `YYYY-MM-DD`.
    pub day: String,
    pub markdown: String,
    pub model: String,
    pub prompt_version: String,
    /// When it was written (RFC 3339).
    pub created_at: String,
    pub cost_usd: Option<f64>,
    /// How many meetings' notes it was written from.
    pub meetings: usize,
    /// The day's notes have changed since: a meeting's notes were
    /// rewritten, or another meeting that day got notes.
    pub outdated: bool,
}

/// Which notes a summary was written from, as stored: `id@summary` for each
/// meeting, in order. Stable across versions (unlike a hash).
pub fn sources(sources: &[DaySource]) -> String {
    sources
        .iter()
        .map(|s| format!("{}@{}", s.meeting_id, s.summary_id))
        .collect::<Vec<_>>()
        .join(",")
}

/// The local day a meeting started on.
pub fn local_day(started_at: &str) -> Option<Date> {
    let at: jiff::Timestamp = started_at.parse().ok()?;
    Some(at.to_zoned(jiff::tz::TimeZone::system()).date())
}

impl Library {
    /// Every meeting with notes, with its latest notes, oldest first.
    fn all_sources(&self) -> Result<Vec<DaySource>, LibraryError> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT m.id, m.title, m.started_at, m.duration_ms, s.id, s.markdown
             FROM meetings m
             JOIN summaries s ON s.id = (SELECT MAX(id) FROM summaries t WHERE t.meeting_id = m.id)
             ORDER BY m.started_at, m.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(DaySource {
                meeting_id: r.get(0)?,
                title: r.get(1)?,
                started_at: r.get(2)?,
                duration_ms: u64::try_from(r.get::<_, i64>(3)?).unwrap_or(0),
                summary_id: r.get(4)?,
                notes: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The meetings on `day` (local) that have notes, in the order they
    /// happened, with their latest notes.
    pub fn day_sources(&self, day: Date) -> Result<Vec<DaySource>, LibraryError> {
        Ok(self
            .all_sources()?
            .into_iter()
            .filter(|s| local_day(&s.started_at) == Some(day))
            .collect())
    }

    /// Every stored daily summary, newest day first, each marked outdated
    /// if its day's notes have changed since.
    pub fn daily_summaries(&self) -> Result<Vec<DailySummary>, LibraryError> {
        let all = self.all_sources()?;
        let rows: Vec<(DailySummary, String)> = {
            let conn = self.conn();
            let mut stmt = conn.prepare_cached(
                "SELECT d.day, d.markdown, d.model, d.prompt_version, d.created_at, d.cost_usd,
                        d.sources
                 FROM daily_summaries d ORDER BY d.day DESC",
            )?;
            let rows = stmt.query_map([], |r| {
                let sources: String = r.get(6)?;
                Ok((
                    DailySummary {
                        day: r.get(0)?,
                        markdown: r.get(1)?,
                        model: r.get(2)?,
                        prompt_version: r.get(3)?,
                        created_at: r.get(4)?,
                        cost_usd: r.get(5)?,
                        meetings: sources.split(',').filter(|s| !s.is_empty()).count(),
                        outdated: false,
                    },
                    sources,
                ))
            })?;
            rows.collect::<Result<_, _>>()?
        };
        Ok(rows
            .into_iter()
            .map(|(mut summary, written_from)| {
                let now: Vec<DaySource> = all
                    .iter()
                    .filter(|s| {
                        local_day(&s.started_at).map(|d| d.to_string()).as_deref()
                            == Some(summary.day.as_str())
                    })
                    .cloned()
                    .collect();
                summary.outdated = sources(&now) != written_from;
                summary
            })
            .collect())
    }

    /// The stored summary of `day`, if there is one.
    pub fn daily_summary(&self, day: Date) -> Result<Option<DailySummary>, LibraryError> {
        let day = day.to_string();
        Ok(self.daily_summaries()?.into_iter().find(|d| d.day == day))
    }

    /// Stores `summary` as `day`'s, written from `sources`, replacing any
    /// before. Returns it as listed.
    pub fn save_daily_summary(
        &self,
        day: Date,
        sources_used: &[DaySource],
        summary: &Summary,
    ) -> Result<DailySummary, LibraryError> {
        let key = day.to_string();
        {
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            tx.execute("DELETE FROM daily_summaries WHERE day = ?1", [&key])?;
            tx.execute(
                "INSERT INTO daily_summaries (day, markdown, sources, model, prompt_version,
                     prompt_tokens, completion_tokens, cost_usd, latency_ms, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    key,
                    summary.markdown,
                    sources(sources_used),
                    summary.model,
                    summary.prompt_version,
                    i64::try_from(summary.prompt_tokens).unwrap_or(i64::MAX),
                    i64::try_from(summary.completion_tokens).unwrap_or(i64::MAX),
                    summary.cost_usd,
                    i64::try_from(summary.latency_ms).unwrap_or(i64::MAX),
                    jiff::Timestamp::now().to_string(),
                ],
            )?;
            for s in sources_used {
                // A meeting deleted meanwhile has no row to point at; the
                // summary would hold its content, so don't keep it.
                let exists: Option<i64> = tx
                    .query_row(
                        "SELECT 1 FROM meetings WHERE id = ?1",
                        [&s.meeting_id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if exists.is_none() {
                    return Err(LibraryError::NotFound(s.meeting_id.clone()));
                }
                tx.execute(
                    "INSERT INTO daily_summary_meetings (day, meeting_id) VALUES (?1, ?2)",
                    params![key, s.meeting_id],
                )?;
            }
            tx.commit()?;
        }
        self.daily_summary(day)?
            .ok_or_else(|| LibraryError::NotFound(key))
    }
}

/// Summarizes a day from its meetings' notes ([`Library::day_sources`]),
/// with `model` on `provider`, in one request. If the whole day's notes are
/// too long for the model, only their essential sections are sent (TL;DR,
/// decisions, action items, open questions).
pub async fn summarize_day(
    provider: &dyn LlmProvider,
    model: &str,
    prompts: &DailyPrompts,
    day: Date,
    sources: &[DaySource],
) -> Result<Summary, SummaryError> {
    if sources.is_empty() {
        return Err(SummaryError::NoNotes);
    }
    let started = std::time::Instant::now();
    let budget = provider
        .context_tokens()
        .saturating_sub(RESERVED_TOKENS)
        .max(1_000);
    let mut meetings = meetings_text(sources, false);
    if quill_llm::estimate_tokens(&meetings) > budget {
        tracing::info!(
            meetings = sources.len(),
            "a day's notes are long for the model; sending their essentials"
        );
        meetings = meetings_text(sources, true);
    }
    let date = day.strftime("%A, %Y-%m-%d").to_string();
    let prompt = quill_llm::prompts::render(
        prompts.user,
        &[
            ("count", &sources.len().to_string()),
            ("date", &date),
            ("meetings", &meetings),
        ],
    )?;
    let response = provider
        .complete(CompletionRequest {
            model: model.to_string(),
            messages: vec![Message::system(prompts.system), Message::user(prompt)],
            max_tokens: Some(MAX_TOKENS),
            temperature: Some(TEMPERATURE),
            // The notes are already distilled, so thinking first adds
            // little, and it used up 2,048 tokens on a real day's notes.
            no_reasoning: true,
        })
        .await?;
    Ok(Summary {
        markdown: clean(&response.text),
        model: response.model,
        prompt_version: prompts.version,
        requests: 1,
        prompt_tokens: u64::from(response.prompt_tokens),
        completion_tokens: u64::from(response.completion_tokens),
        cost_usd: response.cost_usd,
        latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        complete: !response.truncated,
    })
}

/// The meetings for the prompt: a heading for each ("Product sync (09:30,
/// 31 min)"), then its notes with their headings moved two levels down
/// under it and their own title line left out. Not numbered: given
/// "Meeting 3: …", models cite "(Meeting 3)" rather than the title.
fn meetings_text(sources: &[DaySource], essentials_only: bool) -> String {
    sources
        .iter()
        .map(|s| {
            let time = s
                .started_at
                .parse::<jiff::Timestamp>()
                .map(|t| {
                    t.to_zoned(jiff::tz::TimeZone::system())
                        .strftime("%H:%M")
                        .to_string()
                })
                .unwrap_or_default();
            let notes = if essentials_only {
                essentials(&s.notes)
            } else {
                s.notes.clone()
            };
            format!(
                "### {} ({time}, {})\n\n{}",
                s.title,
                format_duration(s.duration_ms),
                demote(&notes)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Notes with their `#` title line left out and every other heading two
/// levels down, to sit under a meeting's `###` heading.
fn demote(notes: &str) -> String {
    notes
        .lines()
        .filter_map(|line| {
            let hashes = line.chars().take_while(|c| *c == '#').count();
            match hashes {
                0 => Some(line.to_string()),
                1 if line[1..].starts_with(' ') => None,
                _ => Some(format!("##{line}")),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Only the notes' essential `##` sections ([`ESSENTIAL_SECTIONS`]), and the
/// title line.
fn essentials(notes: &str) -> String {
    let mut keep = true;
    notes
        .lines()
        .filter(|line| {
            let hashes = line.chars().take_while(|c| *c == '#').count();
            if hashes == 2 {
                let heading = line[2..].trim().to_lowercase();
                keep = ESSENTIAL_SECTIONS.iter().any(|s| heading.starts_with(s));
            } else if hashes == 1 {
                keep = true;
            }
            keep
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use quill_llm::prompts::DAILY;
    use quill_llm::{CompletionResponse, LlmError};

    use super::*;

    /// Fake provider: records the request and answers with fixed notes.
    struct FakeProvider {
        context: usize,
        requests: Mutex<Vec<CompletionRequest>>,
    }

    #[async_trait::async_trait]
    impl LlmProvider for FakeProvider {
        async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            self.requests.lock().unwrap().push(req);
            Ok(CompletionResponse {
                text: "<think>\n</think>\n## TL;DR\nA day.".into(),
                model: "fake-1".into(),
                prompt_tokens: 900,
                completion_tokens: 80,
                cost_usd: Some(0.0004),
                latency_ms: 5,
                truncated: false,
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

    fn library() -> (Library, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path(), &dir.path().join("recordings")).unwrap();
        (lib, dir)
    }

    fn notes(markdown: &str) -> Summary {
        Summary {
            markdown: markdown.into(),
            model: "test".into(),
            prompt_version: "test",
            requests: 1,
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: None,
            latency_ms: 0,
            complete: true,
        }
    }

    const PRODUCT: &str = "# Product sync — 2026-10-26\n\n## TL;DR\n- Ship Friday.\n\n## Decisions\n- Ship at 10%.\n\n## Discussion by topic\n### Onboarding\n- Completion rose.\n";

    fn today() -> Date {
        jiff::Zoned::now().date()
    }

    #[test]
    fn notes_are_demoted_under_each_meeting() {
        assert_eq!(
            demote(PRODUCT),
            "#### TL;DR\n- Ship Friday.\n\n#### Decisions\n- Ship at 10%.\n\n#### Discussion by topic\n##### Onboarding\n- Completion rose."
        );
        let short = essentials(PRODUCT);
        assert!(short.contains("Ship at 10%"));
        assert!(!short.contains("Completion rose"), "{short}");
    }

    #[tokio::test]
    async fn summarizes_the_days_notes_in_one_request() {
        let (lib, _dir) = library();
        let a = lib.create_meeting().unwrap();
        lib.save_summary(&a.id, &notes(PRODUCT)).unwrap();
        let b = lib.create_meeting().unwrap();
        lib.save_summary(
            &b.id,
            &notes("# 1:1 — 2026-10-26\n\n## TL;DR\n- Friday holds.\n"),
        )
        .unwrap();
        // A meeting without notes isn't part of it.
        lib.create_meeting().unwrap();

        let sources = lib.day_sources(today()).unwrap();
        assert_eq!(sources.len(), 2);
        let provider = fake(100_000);
        let summary = summarize_day(&provider, "m", &DAILY, today(), &sources)
            .await
            .unwrap();
        assert_eq!(summary.markdown, "## TL;DR\nA day.");
        assert_eq!(summary.prompt_version, DAILY.version);
        let user = {
            let requests = provider.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            requests[0].messages[1].content.clone()
        };
        assert!(user.contains("### Product sync ("), "{user}");
        assert!(user.contains("### 1:1 ("), "{user}");
        assert!(user.contains("#### Decisions\n- Ship at 10%."));
        assert!(user.contains("notes from the 2 meetings"));

        // Too long for the model: only the essentials go.
        let provider = fake(1_000);
        let long: Vec<DaySource> = sources
            .iter()
            .map(|s| DaySource {
                notes: format!(
                    "{}\n## Notable quotes\n- {}",
                    s.notes,
                    "quote ".repeat(2_000)
                ),
                ..s.clone()
            })
            .collect();
        summarize_day(&provider, "m", &DAILY, today(), &long)
            .await
            .unwrap();
        let user = provider.requests.lock().unwrap()[0].messages[1]
            .content
            .clone();
        assert!(!user.contains("quote quote"), "notable quotes were sent");
        assert!(user.contains("Ship at 10%"));

        // No notes, no summary.
        let none = summarize_day(&fake(100_000), "m", &DAILY, today(), &[]).await;
        assert!(matches!(none, Err(SummaryError::NoNotes)));
    }

    #[tokio::test]
    async fn stored_summaries_go_outdated_and_go_with_their_meetings() {
        let (lib, _dir) = library();
        let a = lib.create_meeting().unwrap();
        lib.save_summary(&a.id, &notes(PRODUCT)).unwrap();
        let b = lib.create_meeting().unwrap();
        lib.save_summary(&b.id, &notes("## TL;DR\n- Friday holds.\n"))
            .unwrap();
        let sources = lib.day_sources(today()).unwrap();
        let summary = summarize_day(&fake(100_000), "m", &DAILY, today(), &sources)
            .await
            .unwrap();
        let saved = lib.save_daily_summary(today(), &sources, &summary).unwrap();
        assert_eq!(saved.day, today().to_string());
        assert_eq!(saved.meetings, 2);
        assert!(!saved.outdated);
        assert_eq!(saved.cost_usd, Some(0.0004));

        // Rewritten notes make it outdated; so does another meeting's.
        lib.save_summary(&b.id, &notes("## TL;DR\n- Friday moved to Monday.\n"))
            .unwrap();
        assert!(lib.daily_summary(today()).unwrap().unwrap().outdated);
        let sources = lib.day_sources(today()).unwrap();
        lib.save_daily_summary(today(), &sources, &summary).unwrap();
        assert!(!lib.daily_summary(today()).unwrap().unwrap().outdated);
        let c = lib.create_meeting().unwrap();
        lib.save_summary(&c.id, &notes("## TL;DR\n- Lunch.\n"))
            .unwrap();
        assert!(lib.daily_summary(today()).unwrap().unwrap().outdated);

        // Deleting one of its meetings deletes it: it holds their content.
        lib.delete(&a.id).unwrap();
        assert!(lib.daily_summary(today()).unwrap().is_none());
        assert!(lib.daily_summaries().unwrap().is_empty());
    }
}
