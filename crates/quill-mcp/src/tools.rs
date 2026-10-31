//! The tools an assistant can call: thin wrappers over
//! `quill_core::assistants`, which decides what they return.

use std::fmt::Display;
use std::sync::Arc;
use std::time::Instant;

use jiff::civil::Date;
use quill_core::{assistants, vocabulary};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::{Context, TURNED_OFF};

/// Most results of each kind a search returns, and meetings a list does.
const MAX_SEARCH: usize = 50;
const MAX_LIST: usize = 500;

/// The MCP server. Cloning shares the context (rmcp clones the handler).
#[derive(Clone)]
pub struct Server {
    context: Arc<dyn Context>,
}

impl Server {
    pub fn new(context: Arc<dyn Context>) -> Self {
        Self { context }
    }

    /// Runs one tool's work off the async threads (SQLite and the search
    /// model block), if access is still on. An `Err` reaches the assistant
    /// as a failed tool call with the message, which it can act on. Logs
    /// the tool and timing only, never what was asked or found.
    async fn answer(
        &self,
        tool: &'static str,
        work: impl FnOnce(&dyn Context) -> Result<String, String> + Send + 'static,
    ) -> Result<String, String> {
        let context = Arc::clone(&self.context);
        let started = Instant::now();
        let result = tokio::task::spawn_blocking(move || {
            if !context.allowed() {
                return Err(TURNED_OFF.to_string());
            }
            work(context.as_ref())
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        tracing::info!(
            tool,
            ok = result.is_ok(),
            ms = started.elapsed().as_millis(),
            "assistant request"
        );
        result
    }
}

/// An error as the message an assistant sees.
fn message(e: impl Display) -> String {
    e.to_string()
}

/// Turns a value into the JSON text a tool returns.
fn json(value: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(message)
}

/// A `YYYY-MM-DD` date from a tool's arguments.
fn day(text: &str) -> Result<Date, String> {
    text.parse()
        .map_err(|_| format!("{text:?} isn't a date like 2026-10-27"))
}

/// A moment from a tool's arguments: RFC 3339, or a `YYYY-MM-DD` day for
/// the start of that local day.
fn moment(text: &str) -> Result<jiff::Timestamp, String> {
    if let Ok(at) = text.parse::<jiff::Timestamp>() {
        return Ok(at);
    }
    day(text)
        .ok()
        .and_then(|d| d.to_zoned(jiff::tz::TimeZone::system()).ok())
        .map(|start| start.timestamp())
        .ok_or_else(|| {
            format!(
                "{text:?} isn't a time like 2026-10-30T17:00:00-07:00 or a date like 2026-10-30"
            )
        })
}

// Each tool's arguments. The doc comments become the descriptions in the
// JSON Schema the assistant is shown; `crate = …` because schemars comes
// through rmcp rather than as a dependency of our own.

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SearchArgs {
    /// Words or a question, e.g. "pricing decision" or "who owns the launch".
    query: String,
    /// Most results of each kind (meetings, moments, notes, action items).
    /// Default 10.
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ListArgs {
    /// Only meetings on or after this local day, `YYYY-MM-DD`.
    from: Option<String>,
    /// Only meetings on or before this local day, `YYYY-MM-DD`.
    to: Option<String>,
    /// Only meetings recorded or given new notes at or after this moment:
    /// RFC 3339 (`2026-10-30T17:00:00-07:00`), or `YYYY-MM-DD` for the
    /// start of that local day. For syncing: pass the time of the last sync.
    updated_since: Option<String>,
    /// Most meetings to list, newest first. Default 50.
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct MeetingArgs {
    /// The meeting's id, from search or list_meetings.
    id: String,
    /// Include the transcript, from this line (0 for the start). Leave it out
    /// for just the notes. Long transcripts come in pages; the reply says
    /// where the next one starts.
    transcript_from: Option<usize>,
}

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ActionArgs {
    /// Include items already ticked off. Default false.
    include_done: Option<bool>,
    /// Only items from this meeting (an id from list_meetings or search).
    meeting_id: Option<String>,
    /// Only items from meetings on or after this local day, `YYYY-MM-DD`.
    since: Option<String>,
    /// Only items from meetings on or before this local day, `YYYY-MM-DD`.
    until: Option<String>,
    /// true: only the user's own items; false: only other people's.
    mine: Option<bool>,
    /// Only items whose owner includes this name, ignoring case, e.g.
    /// "Priya" (which also finds "Priya and Dan").
    owner: Option<String>,
    /// Most items to return, newest meeting first. Default: all of them.
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct DayArgs {
    /// The local day, `YYYY-MM-DD`.
    date: String,
}

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct TermsArgs {
    /// Names and terms as they should be spelled, each one line of at most
    /// six words, e.g. ["Simant Shrivastava", "OpenRouter"].
    terms: Vec<String>,
}

// `#[tool_router]` collects the `#[tool]` methods below into the list the
// assistant sees; `#[tool_handler]` (further down) answers its requests
// with them. Each method's doc comment is its description.
#[tool_router]
impl Server {
    /// Search the user's meetings by words and by meaning: meeting titles,
    /// moments in transcripts, points in meeting notes, and action items.
    /// Start here to find which meetings are about something, then read
    /// them with get_meeting. Matching words are wrapped in [ ]. Times are
    /// UTC (RFC 3339); start_ms is milliseconds into the recording.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn search(&self, Parameters(args): Parameters<SearchArgs>) -> Result<String, String> {
        self.answer("search", move |context| {
            let limit = args.limit.unwrap_or(10).clamp(1, MAX_SEARCH);
            let embedder = context.embedder();
            let found =
                assistants::search(context.library(), &args.query, embedder.as_deref(), limit)
                    .map_err(message)?;
            json(&found)
        })
        .await
    }

    /// List the user's meetings, newest first, optionally between two local
    /// days or only those changed since a moment: id, title, start time
    /// (`started` local, `started_at` RFC 3339 with the local offset),
    /// length in minutes, whether it has notes, and when the notes were
    /// last written and how many times (`notes_updated_at`,
    /// `notes_version`, which grows when notes are summarized again).
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn list_meetings(
        &self,
        Parameters(args): Parameters<ListArgs>,
    ) -> Result<String, String> {
        self.answer("list_meetings", move |context| {
            let from = args.from.as_deref().map(day).transpose()?;
            let to = args.to.as_deref().map(day).transpose()?;
            let since = args.updated_since.as_deref().map(moment).transpose()?;
            let limit = args.limit.unwrap_or(50).clamp(1, MAX_LIST);
            let listed = assistants::meetings(context.library(), from, to, since, limit);
            json(&listed.map_err(message)?)
        })
        .await
    }

    /// Read one meeting: when it was and its notes (summary, decisions,
    /// action items, discussion by topic, open questions, quotes), and,
    /// if asked, its transcript with speakers and times. The notes answer
    /// most questions; read the transcript for exact wording or detail.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn get_meeting(
        &self,
        Parameters(args): Parameters<MeetingArgs>,
    ) -> Result<String, String> {
        self.answer("get_meeting", move |context| {
            assistants::meeting_markdown(context.library(), &args.id, args.transcript_from)
                .map_err(message)
        })
        .await
    }

    /// The action items from meetings' notes, newest meeting first: the
    /// task, who owns it ("mine" is the user), when it's due, and the
    /// meeting it came from. Open ones only unless include_done. Narrow it
    /// down rather than reading them all: by meeting, by the days meetings
    /// were on (since, until), to the user's own (mine) or someone else's
    /// (owner), and with a limit.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn action_items(
        &self,
        Parameters(args): Parameters<ActionArgs>,
    ) -> Result<String, String> {
        self.answer("action_items", move |context| {
            let filter = assistants::ActionFilter {
                include_done: args.include_done.unwrap_or(false),
                meeting_id: args.meeting_id,
                since: args.since.as_deref().map(day).transpose()?,
                until: args.until.as_deref().map(day).transpose()?,
                mine: args.mine,
                owner: args.owner.filter(|o| !o.trim().is_empty()),
                limit: args.limit,
            };
            json(&assistants::action_items(context.library(), &filter).map_err(message)?)
        })
        .await
    }

    /// The summary of one day's meetings, if the user has had Quill write
    /// one. Otherwise, list that day's meetings and read their notes.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn daily_summary(&self, Parameters(args): Parameters<DayArgs>) -> Result<String, String> {
        self.answer("daily_summary", move |context| {
            let day = day(&args.date)?;
            let summary = assistants::daily_markdown(context.library(), day).map_err(message)?;
            Ok(summary.unwrap_or_else(|| format!("No daily summary has been written for {day}.")))
        })
        .await
    }

    /// The user's vocabulary: names and terms Quill spells exactly when it
    /// writes meeting notes and types dictations. A name the transcripts or
    /// notes keep getting wrong belongs here (see add_vocabulary).
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn vocabulary(&self) -> Result<String, String> {
        self.answer("vocabulary", |context| json(&context.vocabulary()))
            .await
    }

    /// Add names or terms to the user's vocabulary, spelled as they should
    /// be: people's names that transcripts mishear, product and system
    /// names. Notes written from now on use them; existing notes change only
    /// when summarized again. Terms already there are skipped. Works only if
    /// the user allows it in Quill, and through the app's own connection.
    /// Returns the whole vocabulary.
    #[tool(annotations(
        read_only_hint = false,
        destructive_hint = false,
        idempotent_hint = true,
        open_world_hint = false
    ))]
    async fn add_vocabulary(
        &self,
        Parameters(args): Parameters<TermsArgs>,
    ) -> Result<String, String> {
        self.answer("add_vocabulary", move |context| {
            json(&context.edit_vocabulary(&|now| vocabulary::added(now, &args.terms))?)
        })
        .await
    }

    /// Take names or terms out of the user's vocabulary (ignoring case):
    /// ones that are wrong or no longer come up. Needs the same permission
    /// as add_vocabulary. Returns the whole vocabulary.
    #[tool(annotations(
        read_only_hint = false,
        destructive_hint = true,
        idempotent_hint = true,
        open_world_hint = false
    ))]
    async fn remove_vocabulary(
        &self,
        Parameters(args): Parameters<TermsArgs>,
    ) -> Result<String, String> {
        self.answer("remove_vocabulary", move |context| {
            json(&context.edit_vocabulary(&|now| Ok(vocabulary::removed(now, &args.terms)))?)
        })
        .await
    }
}

#[tool_handler(
    name = "quill",
    instructions = "Quill holds the user's meeting notes and transcripts, recorded and \
        transcribed on their computer. Use it to answer questions about what was said or \
        decided in meetings: search first, then read the meetings that match. Quote the \
        notes or transcript and name the meeting and its date when you rely on them. Names \
        Quill keeps mishearing can go in its vocabulary, if the user allows it."
)]
impl ServerHandler for Server {}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use rmcp::ServiceExt;
    use serde_json::{Value, json};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::*;
    use crate::testing;

    /// Talks to the server the way an assistant does over stdio: one
    /// JSON-RPC message per line, over an in-memory pipe.
    struct Client {
        reader: BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
        writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
        next_id: u64,
    }

    impl Client {
        async fn send(&mut self, message: Value) {
            let mut line = message.to_string();
            line.push('\n');
            self.writer.write_all(line.as_bytes()).await.unwrap();
        }

        /// Sends a request and waits for its response's `result`.
        async fn call(&mut self, method: &str, params: Value) -> Value {
            self.next_id += 1;
            let id = self.next_id;
            self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
                .await;
            loop {
                let mut line = String::new();
                assert!(
                    self.reader.read_line(&mut line).await.unwrap() > 0,
                    "server hung up"
                );
                let message: Value = serde_json::from_str(&line).unwrap();
                if message["id"] == id {
                    return message["result"].clone();
                }
            }
        }

        /// Calls a tool; its text, and whether it failed.
        async fn tool(&mut self, name: &str, arguments: Value) -> (String, bool) {
            let result = self
                .call("tools/call", json!({"name": name, "arguments": arguments}))
                .await;
            let text = result["content"][0]["text"].as_str().unwrap_or_default();
            (text.to_string(), result["isError"] == true)
        }
    }

    #[tokio::test]
    async fn serves_the_library_while_access_is_on() {
        let (context, id) = testing::context();
        let server = Server::new(context.clone());
        let (ours, theirs) = tokio::io::duplex(1 << 16);
        tokio::spawn(async move { server.serve(theirs).await.unwrap().waiting().await });
        let (reader, writer) = tokio::io::split(ours);
        let mut client = Client {
            reader: BufReader::new(reader),
            writer,
            next_id: 0,
        };

        let init = client
            .call(
                "initialize",
                json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "1"},
                }),
            )
            .await;
        assert_eq!(init["serverInfo"]["name"], "quill");
        client
            .send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;

        let tools = client.call("tools/list", json!({})).await;
        let mut names: Vec<_> = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "action_items",
                "add_vocabulary",
                "daily_summary",
                "get_meeting",
                "list_meetings",
                "remove_vocabulary",
                "search",
                "vocabulary"
            ]
        );

        let (found, failed) = client.tool("search", json!({"query": "prices"})).await;
        assert!(!failed, "{found}");
        assert!(found.contains(&id) && found.contains("[prices]"), "{found}");

        let (meeting, _) = client
            .tool("get_meeting", json!({"id": id, "transcript_from": 0}))
            .await;
        assert!(
            meeting.contains("## Decisions\n- Raise prices in March."),
            "{meeting}"
        );
        assert!(
            meeting.contains("[0:00:00] Priya: We should raise prices"),
            "{meeting}"
        );

        let (missing, failed) = client.tool("get_meeting", json!({"id": "nope"})).await;
        assert!(
            failed && missing.contains("no meeting with id nope"),
            "{missing}"
        );

        let (bad, failed) = client
            .tool("daily_summary", json!({"date": "tomorrow"}))
            .await;
        assert!(failed && bad.contains("isn't a date"), "{bad}");
        let (bad, failed) = client
            .tool("action_items", json!({"mine": true, "since": "last week"}))
            .await;
        assert!(failed && bad.contains("isn't a date"), "{bad}");
        let (none, failed) = client
            .tool(
                "action_items",
                json!({"mine": true, "since": "2026-10-01", "limit": 5}),
            )
            .await;
        assert!(!failed && none.trim() == "[]", "{none}");

        // Changed since: a day or a moment, and the fields a sync needs.
        let (listed, failed) = client
            .tool("list_meetings", json!({"updated_since": "2026-01-01"}))
            .await;
        assert!(!failed && listed.contains(&id), "{listed}");
        assert!(listed.contains("\"notes_version\": 1"), "{listed}");
        assert!(listed.contains("\"notes_updated_at\": \"20"), "{listed}");
        let (later, _) = client
            .tool(
                "list_meetings",
                json!({"updated_since": "2999-01-01T00:00:00Z"}),
            )
            .await;
        assert_eq!(later.trim(), "[]");
        let (bad, failed) = client
            .tool("list_meetings", json!({"updated_since": "yesterday"}))
            .await;
        assert!(failed && bad.contains("isn't a time"), "{bad}");

        // The vocabulary: readable, and changed only once the user allows it.
        let (words, _) = client.tool("vocabulary", json!({})).await;
        assert!(words.contains("Quill"), "{words}");
        let add = |terms: Value| json!({"terms": terms});
        let (refused, failed) = client.tool("add_vocabulary", add(json!(["Prall"]))).await;
        assert!(
            failed && refused.contains("doesn't let AI assistants"),
            "{refused}"
        );
        context.vocabulary_editable.store(true, Ordering::SeqCst);
        let (words, failed) = client
            .tool("add_vocabulary", add(json!(["Prall", "quill"])))
            .await;
        assert!(!failed && words.contains("Prall"), "{words}");
        assert_eq!(*context.vocabulary.lock().unwrap(), ["Quill", "Prall"]);
        let (bad, failed) = client.tool("add_vocabulary", add(json!(["a\nb"]))).await;
        assert!(failed && bad.contains("isn't a name or term"), "{bad}");
        client
            .tool("remove_vocabulary", add(json!(["QUILL"])))
            .await;
        assert_eq!(*context.vocabulary.lock().unwrap(), ["Prall"]);

        // Turned off in Settings while connected: nothing more is served.
        context.allowed.store(false, Ordering::SeqCst);
        let (refused, failed) = client.tool("list_meetings", json!({})).await;
        assert!(
            failed && refused.contains("Let AI assistants read"),
            "{refused}"
        );
    }
}
