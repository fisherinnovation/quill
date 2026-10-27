//! What AI assistants (Claude and others) may read from the library, when
//! the user lets them ([`Settings::assistant_access`], off by default).
//! `quill-cli mcp` serves it over the Model Context Protocol; this module
//! decides what they see and how, so any other way in sees the same.
//!
//! Read-only, and meetings only: titles, notes, transcripts, action items
//! and daily summaries. Dictations are left out, even from search: they're
//! often private messages typed into other apps, not meeting content.
//!
//! [`Settings::assistant_access`]: crate::settings::Settings::assistant_access

use std::fmt::Write as _;

use jiff::civil::Date;
use quill_embed::Embedder;
use serde::Serialize;

use crate::daily::local_day;
use crate::library::{ActionItem, Library, LibraryError, Meeting, SearchHit};
use crate::search::{ActionHit, Group, MeetingHit, NoteHit, Scope};
use crate::semantic::SemanticError;

/// Transcript lines per [`meeting_markdown`] page: about 10k tokens, so a
/// long meeting doesn't fill an assistant's context in one call.
pub const TRANSCRIPT_PAGE: usize = 300;

/// What [`search`] found: [`crate::search::Results`] without dictations.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Found {
    /// Meetings whose title matches.
    pub meetings: Group<MeetingHit>,
    /// Moments in transcripts.
    pub moments: Group<SearchHit>,
    pub notes: Group<NoteHit>,
    pub actions: Group<ActionHit>,
}

/// Searches meetings, notes and action items as the app's search does (by
/// words, and by meaning with an `embedder`): up to `limit` of each.
pub fn search(
    library: &Library,
    query: &str,
    embedder: Option<&dyn Embedder>,
    limit: usize,
) -> Result<Found, SemanticError> {
    let results = library.search_everything(query, Scope::All, embedder, limit)?;
    Ok(Found {
        meetings: results.meetings,
        moments: results.moments,
        notes: results.notes,
        actions: results.actions,
    })
}

/// A meeting in [`meetings`]: what an assistant needs to pick one.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Listed {
    pub id: String,
    pub title: String,
    /// Local time, `YYYY-MM-DD HH:MM`.
    pub started: String,
    pub minutes: u64,
    pub has_notes: bool,
}

/// Meetings with something said, newest first, started between `from` and
/// `to` (local days, both included), up to `limit`.
pub fn meetings(
    library: &Library,
    from: Option<Date>,
    to: Option<Date>,
    limit: usize,
) -> Result<Vec<Listed>, LibraryError> {
    Ok(library
        .list()?
        .into_iter()
        .filter(|m| !m.empty)
        .filter(|m| {
            let day = local_day(&m.started_at);
            from.is_none_or(|from| day.is_some_and(|d| d >= from))
                && to.is_none_or(|to| day.is_some_and(|d| d <= to))
        })
        .take(limit)
        .map(|m: Meeting| Listed {
            started: local_time(&m.started_at).unwrap_or_else(|| m.started_at.clone()),
            minutes: (m.duration_ms + 30_000) / 60_000,
            has_notes: m.has_summary,
            id: m.id,
            title: m.title,
        })
        .collect())
}

/// The daily summary of a local day, as Markdown, if one has been written.
pub fn daily_markdown(library: &Library, day: Date) -> Result<Option<String>, LibraryError> {
    Ok(library.daily_summary(day)?.map(|d| {
        let mut out = d.markdown.trim().to_string();
        if d.outdated {
            out.push_str(
                "

(Written before some of the day's notes changed: check the meetings' own notes.)",
            );
        }
        out
    }))
}

/// Action items, newest meeting first; open ones only unless `include_done`.
pub fn action_items(
    library: &Library,
    include_done: bool,
) -> Result<Vec<ActionItem>, LibraryError> {
    let mut items = library.action_items()?;
    if !include_done {
        items.retain(|a| a.done_at.is_none());
    }
    Ok(items)
}

/// One meeting as Markdown: when it was, its latest notes, and, if
/// `transcript_from` is given, [`TRANSCRIPT_PAGE`] lines of its transcript
/// from that line (0 for the start) with speakers and times.
pub fn meeting_markdown(
    library: &Library,
    id: &str,
    transcript_from: Option<usize>,
) -> Result<String, LibraryError> {
    let detail = library.get(id)?;
    let m = &detail.meeting;
    let mut out = format!("Meeting {}: {}\n", m.id, m.title);
    let when = local_time(&m.started_at).unwrap_or_else(|| m.started_at.clone());
    let _ = writeln!(
        out,
        "Started {when} (local time), {}.\n",
        duration(m.duration_ms)
    );
    match detail.summaries.first() {
        Some(notes) => out.push_str(notes.markdown.trim()),
        None => out.push_str("No notes have been written for this meeting."),
    }
    out.push_str("\n\n## Transcript\n");
    let total = detail.segments.len();
    let Some(from) = transcript_from else {
        let _ = writeln!(
            out,
            "{total} lines. Ask for the transcript only if the notes don't answer the question."
        );
        return Ok(out);
    };
    if from >= total {
        let _ = writeln!(
            out,
            "The transcript has {total} lines, none from line {from}."
        );
        return Ok(out);
    }
    let to = (from + TRANSCRIPT_PAGE).min(total);
    let _ = writeln!(out, "Lines {from}–{} of {total}.", to - 1);
    for s in &detail.segments[from..to] {
        let speaker = s.speaker.as_deref().unwrap_or("Unknown");
        let _ = writeln!(out, "[{}] {speaker}: {}", clock(s.start_ms), s.text.trim());
    }
    if to < total {
        let _ = writeln!(out, "More follows: ask again from line {to}.");
    }
    Ok(out)
}

/// An RFC 3339 time as local `YYYY-MM-DD HH:MM`.
fn local_time(at: &str) -> Option<String> {
    let at: jiff::Timestamp = at.parse().ok()?;
    let local = at.to_zoned(jiff::tz::TimeZone::system());
    Some(local.strftime("%Y-%m-%d %H:%M").to_string())
}

/// "45 min", "1 h 05 min".
fn duration(ms: u64) -> String {
    let minutes = (ms + 30_000) / 60_000;
    if minutes < 60 {
        format!("{minutes} min")
    } else {
        format!("{} h {:02} min", minutes / 60, minutes % 60)
    }
}

/// A time into the recording, `H:MM:SS`.
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

#[cfg(test)]
mod tests {
    use quill_stt::Segment;

    use super::*;
    use crate::library::DictationEntry;
    use crate::summary::Summary;

    fn library() -> (Library, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path(), &dir.path().join("recordings")).unwrap();
        (lib, dir)
    }

    fn summary(markdown: &str) -> Summary {
        Summary {
            markdown: markdown.into(),
            model: "test".into(),
            prompt_version: "test",
            requests: 1,
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: None,
            latency_ms: 0,
        }
    }

    fn segment(i: u64, speaker: &str, text: &str) -> Segment {
        Segment {
            start_ms: i * 5_000,
            end_ms: i * 5_000 + 4_000,
            text: text.into(),
            speaker: Some(speaker.into()),
        }
    }

    /// A meeting that started at `started_at`, with a transcript of `lines`.
    fn meeting(lib: &Library, started_at: &str, lines: &[Segment]) -> String {
        let id = lib.create_meeting().unwrap().id;
        lib.conn()
            .execute(
                "UPDATE meetings SET started_at = ?2 WHERE id = ?1",
                [&id, started_at],
            )
            .unwrap();
        lib.set_duration(&id, 45 * 60_000).unwrap();
        lib.save_transcript(&id, lines).unwrap();
        id
    }

    const NOTES: &str = "# Hiring plan — 2026-10-20

## TL;DR
- We agreed to start hiring two engineers.

## Action items
- **Priya** — post the job ads — by Friday
";

    #[test]
    fn search_leaves_dictations_out() {
        let (lib, _dir) = library();
        let id = meeting(
            &lib,
            "2026-10-20T15:00:00Z",
            &[segment(0, "Priya", "The hiring budget is approved.")],
        );
        lib.save_summary(&id, &summary(NOTES)).unwrap();
        lib.add_dictation(&DictationEntry {
            id: 0,
            created_at: String::new(),
            text: "hiring is a secret until Monday".into(),
            raw: None,
            polish_model: None,
            audio_ms: 3000,
            latency_ms: 300,
            pasted: true,
            app: Some("Slack".into()),
            app_id: None,
            wait_ms: None,
            corrected: None,
        })
        .unwrap();
        // The app's own search finds the dictation...
        let everything = lib
            .search_everything("hiring", Scope::All, None, 10)
            .unwrap();
        assert_eq!(everything.dictations.hits.len(), 1);
        // ...an assistant's doesn't, and its results can't say there is one.
        let found = search(&lib, "hiring", None, 10).unwrap();
        assert_eq!(found.moments.hits.len(), 1);
        assert_eq!(found.notes.hits.len(), 1);
        let json = serde_json::to_string(&found).unwrap();
        assert!(
            !json.contains("secret") && !json.contains("dictation"),
            "{json}"
        );
    }

    #[test]
    fn meetings_filter_by_local_day_and_skip_empty_ones() {
        let (lib, _dir) = library();
        let said = [segment(0, "Me", "Hello.")];
        // Midday UTC, so the local day is the same in any time zone that
        // the machine running the tests could plausibly be in.
        let a = meeting(&lib, "2026-10-19T12:00:00Z", &said);
        let b = meeting(&lib, "2026-10-20T12:00:00Z", &said);
        let c = meeting(&lib, "2026-10-21T12:00:00Z", &said);
        let empty = meeting(&lib, "2026-10-20T13:00:00Z", &[]);
        let ids = |from: Option<&str>, to: Option<&str>, limit| -> Vec<String> {
            let day = |d: &str| d.parse::<Date>().unwrap();
            meetings(&lib, from.map(day), to.map(day), limit)
                .unwrap()
                .into_iter()
                .map(|m| m.id)
                .collect()
        };
        assert_eq!(ids(None, None, 10), [c.clone(), b.clone(), a.clone()]);
        assert_eq!(ids(Some("2026-10-20"), None, 10), [c.clone(), b.clone()]);
        assert_eq!(ids(None, Some("2026-10-20"), 10), [b.clone(), a]);
        assert_eq!(ids(Some("2026-10-20"), Some("2026-10-20"), 10), [b]);
        assert_eq!(ids(None, None, 1), [c]);
        assert!(!ids(None, None, 10).contains(&empty));
    }

    #[test]
    fn open_action_items_unless_done_ones_are_asked_for() {
        let (lib, _dir) = library();
        let id = meeting(&lib, "2026-10-20T15:00:00Z", &[segment(0, "Me", "Hi.")]);
        let notes = "## Action items\n- **Priya** — post the job ads\n- **You** — book the room\n";
        lib.save_summary(&id, &summary(notes)).unwrap();
        let all = action_items(&lib, true).unwrap();
        assert_eq!(all.len(), 2);
        lib.set_action_item_done(all[0].id, true).unwrap();
        let open = action_items(&lib, false).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].task, all[1].task);
    }

    #[test]
    fn a_meeting_reads_as_notes_then_transcript_pages() {
        let (lib, _dir) = library();
        let lines: Vec<_> = (0..TRANSCRIPT_PAGE as u64 + 5)
            .map(|i| {
                segment(
                    i,
                    if i % 2 == 0 { "Priya" } else { "Me" },
                    &format!("line {i}"),
                )
            })
            .collect();
        let id = meeting(&lib, "2026-10-20T15:00:00Z", &lines);

        let text = meeting_markdown(&lib, &id, None).unwrap();
        assert!(text.contains("No notes have been written"), "{text}");
        assert!(text.contains("45 min"), "{text}");
        assert!(text.contains(&format!("{} lines.", lines.len())), "{text}");
        assert!(
            !text.contains("line 0"),
            "no transcript unless asked: {text}"
        );

        lib.save_summary(&id, &summary(NOTES)).unwrap();
        let first = meeting_markdown(&lib, &id, Some(0)).unwrap();
        assert!(first.contains("## TL;DR\n- We agreed"), "{first}");
        assert!(first.contains("[0:00:00] Priya: line 0\n[0:00:05] Me: line 1\n"));
        assert!(first.contains(&format!("ask again from line {TRANSCRIPT_PAGE}")));
        assert!(!first.contains(&format!("line {TRANSCRIPT_PAGE}\n")));

        let rest = meeting_markdown(&lib, &id, Some(TRANSCRIPT_PAGE)).unwrap();
        assert!(
            rest.contains(&format!("line {TRANSCRIPT_PAGE}\n")),
            "{rest}"
        );
        assert!(rest.contains(&format!("line {}\n", lines.len() - 1)));
        assert!(!rest.contains("More follows"));

        let past = meeting_markdown(&lib, &id, Some(10_000)).unwrap();
        assert!(past.contains("none from line 10000"), "{past}");

        assert!(matches!(
            meeting_markdown(&lib, "nope", None),
            Err(LibraryError::NotFound(_))
        ));
    }

    #[test]
    fn durations_and_clock_times() {
        assert_eq!(duration(44 * 60_000 + 40_000), "45 min");
        assert_eq!(duration(65 * 60_000), "1 h 05 min");
        assert_eq!(clock(3_723_000), "1:02:03");
    }
}
