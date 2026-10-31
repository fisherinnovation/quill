//! What AI assistants (Claude and others) may read from the library, when
//! the user lets them ([`Settings::assistant_access`], off by default).
//! `quill-cli mcp` serves it over the Model Context Protocol; this module
//! decides what they see and how, so any other way in sees the same.
//!
//! Meetings only: titles, notes, transcripts, action items and daily
//! summaries. Dictations are left out, even from search: they're often
//! private messages typed into other apps, not meeting content. Read-only,
//! except that the user may also let assistants change the vocabulary
//! ([`crate::vocabulary`]) and tick off action items.
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

/// Said before notes the model didn't finish ([`StoredSummary::complete`]).
///
/// [`StoredSummary::complete`]: crate::library::StoredSummary::complete
pub const INCOMPLETE: &str = "Notes incomplete: the model stopped at its length limit, so \
    these notes end early and later sections may be missing. Read the transcript for the \
    rest, or summarize the meeting again in Quill.";

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
    /// The same moment as RFC 3339 with the local offset,
    /// `2026-10-30T17:00:00-07:00`.
    pub started_at: String,
    pub minutes: u64,
    pub has_notes: bool,
    /// When the latest notes were written (RFC 3339, local offset); `None`
    /// without notes.
    pub notes_updated_at: Option<String>,
    /// How many times the notes have been written: 0 without notes, 1 for
    /// the first, and one more each time the meeting is summarized again.
    pub notes_version: u64,
}

/// Meetings with something said, newest first, started between `from` and
/// `to` (local days, both included), up to `limit`. With `updated_since`,
/// only those recorded or given new notes at or after that moment.
pub fn meetings(
    library: &Library,
    from: Option<Date>,
    to: Option<Date>,
    updated_since: Option<jiff::Timestamp>,
    limit: usize,
) -> Result<Vec<Listed>, LibraryError> {
    let versions = library.notes_versions()?;
    let instant = |at: &str| at.parse::<jiff::Timestamp>().ok();
    Ok(library
        .list()?
        .into_iter()
        .filter(|m| !m.empty)
        .filter(|m| {
            let day = local_day(&m.started_at);
            from.is_none_or(|from| day.is_some_and(|d| d >= from))
                && to.is_none_or(|to| day.is_some_and(|d| d <= to))
        })
        .filter(|m| {
            updated_since.is_none_or(|since| {
                let notes = versions.get(&m.id).and_then(|v| instant(&v.written_at));
                instant(&m.started_at)
                    .max(notes)
                    .is_some_and(|at| at >= since)
            })
        })
        .take(limit)
        .map(|m: Meeting| {
            let notes = versions.get(&m.id).cloned();
            Listed {
                started: local_time(&m.started_at).unwrap_or_else(|| m.started_at.clone()),
                started_at: rfc3339_local(&m.started_at),
                minutes: (m.duration_ms + 30_000) / 60_000,
                has_notes: m.has_summary,
                notes_updated_at: notes.as_ref().map(|v| rfc3339_local(&v.written_at)),
                notes_version: notes.map_or(0, |v| v.version),
                id: m.id,
                title: m.title,
            }
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

/// Which action items [`action_items`] returns. The default is every open
/// item; each filter narrows it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ActionFilter {
    /// Ticked-off items too.
    pub include_done: bool,
    /// Only this meeting's.
    pub meeting_id: Option<String>,
    /// Only from meetings on or after this local day.
    pub since: Option<Date>,
    /// Only from meetings on or before this local day.
    pub until: Option<Date>,
    /// `Some(true)`: only the user's own; `Some(false)`: only other people's.
    pub mine: Option<bool>,
    /// Only items whose owner includes this, ignoring case: "priya" finds
    /// "Priya and Dan".
    pub owner: Option<String>,
    /// At most this many, newest meeting first.
    pub limit: Option<usize>,
}

/// Action items, newest meeting first, as `filter` narrows them.
pub fn action_items(
    library: &Library,
    filter: &ActionFilter,
) -> Result<Vec<ActionItem>, LibraryError> {
    let owner = filter.owner.as_deref().map(str::to_lowercase);
    let items = library.action_items()?.into_iter().filter(|a| {
        let day = local_day(&a.meeting_started_at);
        (filter.include_done || a.done_at.is_none())
            && filter
                .meeting_id
                .as_ref()
                .is_none_or(|id| *id == a.meeting_id)
            && filter
                .since
                .is_none_or(|since| day.is_some_and(|d| d >= since))
            && filter
                .until
                .is_none_or(|until| day.is_some_and(|d| d <= until))
            && filter.mine.is_none_or(|mine| mine == a.mine)
            && owner.as_ref().is_none_or(|o| {
                a.owner
                    .as_ref()
                    .is_some_and(|owner| owner.to_lowercase().contains(o.as_str()))
            })
    });
    Ok(items.take(filter.limit.unwrap_or(usize::MAX)).collect())
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
        "Started {when} (local time, {}), {}.",
        rfc3339_local(&m.started_at),
        duration(m.duration_ms)
    );
    // Newest first, so the first is the latest and the count is its version.
    if let Some(notes) = detail.summaries.first() {
        let _ = writeln!(
            out,
            "Notes version {}, written {}.",
            detail.summaries.len(),
            rfc3339_local(&notes.created_at)
        );
    }
    out.push('\n');
    match detail.summaries.first() {
        Some(notes) => {
            if !notes.complete {
                out.push_str(INCOMPLETE);
                out.push_str("\n\n");
            }
            out.push_str(notes.markdown.trim());
        }
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

/// An RFC 3339 time (as stored, in UTC) as RFC 3339 with the local offset,
/// to the second: `2026-10-30T17:00:00-07:00`. Unchanged if it can't be read.
pub fn rfc3339_local(at: &str) -> String {
    at.parse::<jiff::Timestamp>().map_or_else(
        |_| at.to_string(),
        |at| {
            at.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%dT%H:%M:%S%:z")
                .to_string()
        },
    )
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
            complete: true,
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
    fn meetings_say_when_their_notes_changed() {
        let (lib, _dir) = library();
        let said = [segment(0, "Me", "Hello.")];
        let old = meeting(&lib, "2026-10-19T12:00:00Z", &said);
        let new = meeting(&lib, "2026-10-20T12:00:00Z", &said);
        // Summarized twice, just now: newer notes than either meeting.
        lib.save_summary(&old, &summary(NOTES)).unwrap();
        lib.save_summary(&old, &summary(NOTES)).unwrap();

        let listed = meetings(&lib, None, None, None, 10).unwrap();
        assert_eq!(listed[0].id, new);
        assert_eq!(
            (listed[0].notes_version, &listed[0].notes_updated_at),
            (0, &None)
        );
        assert_eq!(listed[1].notes_version, 2);
        assert!(listed[1].notes_updated_at.is_some());
        // RFC 3339 with the local offset: the same moment as stored.
        let started = &listed[1].started_at;
        assert_eq!(
            started.len(),
            "2026-10-19T12:00:00+00:00".len(),
            "{started}"
        );
        assert_eq!(
            started.parse::<jiff::Timestamp>().unwrap(),
            "2026-10-19T12:00:00Z".parse::<jiff::Timestamp>().unwrap()
        );

        // Since a minute ago: only the meeting whose notes were just written.
        let ids = |since: jiff::Timestamp| -> Vec<String> {
            meetings(&lib, None, None, Some(since), 10)
                .unwrap()
                .into_iter()
                .map(|m| m.id)
                .collect()
        };
        let minute_ago = jiff::Timestamp::now()
            .checked_sub(jiff::SignedDuration::from_mins(1))
            .unwrap();
        assert_eq!(ids(minute_ago), std::slice::from_ref(&old));
        // Since before the newer meeting started: both.
        assert_eq!(ids("2026-10-20T00:00:00Z".parse().unwrap()), [new, old]);
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
            meetings(&lib, from.map(day), to.map(day), None, limit)
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
        let everything = ActionFilter {
            include_done: true,
            ..ActionFilter::default()
        };
        let all = action_items(&lib, &everything).unwrap();
        assert_eq!(all.len(), 2);
        lib.set_action_item_done(all[0].id, true).unwrap();
        let open = action_items(&lib, &ActionFilter::default()).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].task, all[1].task);
    }

    #[test]
    fn action_items_narrow_by_meeting_day_owner_and_count() {
        let (lib, _dir) = library();
        let said = [segment(0, "Me", "Hi.")];
        // Midday UTC, so the local day is the same wherever the tests run.
        let old = meeting(&lib, "2026-10-19T12:00:00Z", &said);
        let new = meeting(&lib, "2026-10-30T12:00:00Z", &said);
        lib.save_summary(
            &old,
            &summary("## Action items\n- **You** — send the old deck\n- **Priya** — old ads\n"),
        )
        .unwrap();
        lib.save_summary(
            &new,
            &summary(
                "## Action items\n- **You** — book the room — by Monday\n\
                 - **Priya and Dan** — post the job ads\n- **Dan** — fix the build\n",
            ),
        )
        .unwrap();
        let tasks = |filter: ActionFilter| -> Vec<String> {
            action_items(&lib, &filter)
                .unwrap()
                .into_iter()
                .map(|a| a.task)
                .collect()
        };
        let day = |d: &str| Some(d.parse::<Date>().unwrap());

        // What a daily sync asks for: the user's own items since a day.
        let mine_since = ActionFilter {
            mine: Some(true),
            since: day("2026-10-30"),
            ..ActionFilter::default()
        };
        assert_eq!(tasks(mine_since), ["book the room"]);
        let others = ActionFilter {
            mine: Some(false),
            until: day("2026-10-29"),
            ..ActionFilter::default()
        };
        assert_eq!(tasks(others), ["old ads"]);
        let priya = ActionFilter {
            owner: Some("priya".into()),
            ..ActionFilter::default()
        };
        assert_eq!(tasks(priya), ["post the job ads", "old ads"]);
        let one_meeting = ActionFilter {
            meeting_id: Some(old.clone()),
            ..ActionFilter::default()
        };
        assert_eq!(tasks(one_meeting).len(), 2);
        let newest_two = ActionFilter {
            limit: Some(2),
            ..ActionFilter::default()
        };
        assert_eq!(tasks(newest_two), ["book the room", "post the job ads"]);
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
        assert!(first.contains("\nNotes version 1, written 20"), "{first}");
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
        assert!(!past.contains(INCOMPLETE));

        // Notes the model didn't finish say so before anything else.
        let cut_off = Summary {
            complete: false,
            ..summary(NOTES)
        };
        lib.save_summary(&id, &cut_off).unwrap();
        let text = meeting_markdown(&lib, &id, None).unwrap();
        assert!(
            text.contains(&format!("{INCOMPLETE}\n\n# Hiring plan")),
            "{text}"
        );

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
