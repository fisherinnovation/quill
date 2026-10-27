//! `quill-cli mcp`: lets AI assistants such as Claude read the meeting
//! library, as a Model Context Protocol (MCP) server on stdin/stdout. The
//! assistant starts it (e.g. from Claude Desktop's `claude_desktop_config.json`
//! or `claude mcp add`) and calls its tools; what they return is decided by
//! `quill_core::assistants`.
//!
//! It serves only while **Settings → Meetings → Let AI assistants read your
//! meetings** is on (`assistant_access`), and checks that on every request.
//! stdout carries the protocol, so logs go to stderr, as always in the CLI.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use anyhow::{Context, bail};
use jiff::civil::Date;
use quill_core::assistants;
use quill_core::library::Library;
use quill_core::semantic;
use quill_core::settings::Settings;
use quill_embed::Embedder;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde::Deserialize;

/// What an assistant is told when access is off.
const TURNED_OFF: &str = "Quill isn't sharing meetings with AI assistants. The user can turn it on \
     in Quill: Settings → Meetings → Let AI assistants read your meetings.";

/// Most results of each kind a search returns, and meetings a list does.
const MAX_SEARCH: usize = 50;
const MAX_LIST: usize = 500;

pub fn run(models_dir: &Path) -> anyhow::Result<()> {
    let data_dir = quill_core::paths::data_dir().context("could not find a home directory")?;
    let settings = Settings::load(&data_dir).context("reading settings")?;
    if !settings.assistant_access {
        bail!("{TURNED_OFF}");
    }
    let library = Library::open(&data_dir, &settings.recordings_dir(&data_dir))
        .context("opening the meeting library")?;
    let server = Server::new(library, data_dir, models_dir.to_path_buf());
    tracing::info!("serving the library to an AI assistant over MCP");
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        // `serve` answers the assistant's handshake; `waiting` then runs
        // until it closes stdin (when the assistant quits or restarts us).
        server
            .serve(rmcp::transport::stdio())
            .await?
            .waiting()
            .await?;
        anyhow::Ok(())
    })
}

/// The MCP server. Cloning shares the library (rmcp clones the handler).
#[derive(Clone)]
struct Server {
    inner: Arc<Inner>,
}

struct Inner {
    library: Library,
    data_dir: PathBuf,
    models_dir: PathBuf,
    /// The search model, loaded on the first search if search by meaning is
    /// on and the model is downloaded (this never downloads it); `None`
    /// searches by words only.
    embedder: OnceLock<Option<quill_embed::E5Embedder>>,
}

impl Inner {
    fn embedder(&self) -> Option<&dyn Embedder> {
        self.embedder
            .get_or_init(|| {
                let on = Settings::load(&self.data_dir).is_ok_and(|s| s.semantic_search);
                if !on || !semantic::model_ready(&self.models_dir) {
                    return None;
                }
                semantic::load_embedder(&self.models_dir, semantic::default_threads())
                    .inspect_err(|e| tracing::warn!("search by words only: {e}"))
                    .ok()
            })
            .as_ref()
            .map(|e| e as &dyn Embedder)
    }
}

/// Turns a value into the JSON text a tool returns.
fn json(value: &impl serde::Serialize) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(value)?)
}

/// A `YYYY-MM-DD` date from a tool's arguments.
fn day(text: &str) -> anyhow::Result<Date> {
    text.parse()
        .with_context(|| format!("{text:?} isn't a date like 2026-10-27"))
}

impl Server {
    fn new(library: Library, data_dir: PathBuf, models_dir: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                library,
                data_dir,
                models_dir,
                embedder: OnceLock::new(),
            }),
        }
    }

    /// Runs one tool's work off the async threads (SQLite and the search
    /// model block), if access is still on. An `Err` reaches the assistant
    /// as a failed tool call with the message, which it can act on. Logs
    /// the tool and timing only, never what was asked or found.
    async fn answer(
        &self,
        tool: &'static str,
        work: impl FnOnce(&Inner) -> anyhow::Result<String> + Send + 'static,
    ) -> Result<String, String> {
        let inner = Arc::clone(&self.inner);
        let started = Instant::now();
        let result = tokio::task::spawn_blocking(move || {
            if !Settings::load(&inner.data_dir).is_ok_and(|s| s.assistant_access) {
                bail!("{TURNED_OFF}");
            }
            work(&inner)
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r.map_err(|e| format!("{e:#}")));
        tracing::info!(
            tool,
            ok = result.is_ok(),
            ms = started.elapsed().as_millis(),
            "assistant request"
        );
        result
    }
}

// Each tool's arguments. The doc comments become the descriptions in the
// JSON Schema the assistant is shown; `crate = …` because schemars comes
// through rmcp rather than as a dependency of our own.

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct SearchArgs {
    /// Words or a question, e.g. "pricing decision" or "who owns the launch".
    query: String,
    /// Most results of each kind (meetings, moments, notes, action items).
    /// Default 10.
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct ListArgs {
    /// Only meetings on or after this local day, `YYYY-MM-DD`.
    from: Option<String>,
    /// Only meetings on or before this local day, `YYYY-MM-DD`.
    to: Option<String>,
    /// Most meetings to list, newest first. Default 50.
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct MeetingArgs {
    /// The meeting's id, from search or list_meetings.
    id: String,
    /// Include the transcript, from this line (0 for the start). Leave it out
    /// for just the notes. Long transcripts come in pages; the reply says
    /// where the next one starts.
    transcript_from: Option<usize>,
}

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct ActionArgs {
    /// Include items already ticked off. Default false.
    include_done: Option<bool>,
}

#[derive(Debug, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
struct DayArgs {
    /// The local day, `YYYY-MM-DD`.
    date: String,
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
        self.answer("search", move |inner| {
            let limit = args.limit.unwrap_or(10).clamp(1, MAX_SEARCH);
            json(&assistants::search(
                &inner.library,
                &args.query,
                inner.embedder(),
                limit,
            )?)
        })
        .await
    }

    /// List the user's meetings, newest first, optionally between two local
    /// days: id, title, local start time, length in minutes, and whether it
    /// has notes.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn list_meetings(
        &self,
        Parameters(args): Parameters<ListArgs>,
    ) -> Result<String, String> {
        self.answer("list_meetings", move |inner| {
            let from = args.from.as_deref().map(day).transpose()?;
            let to = args.to.as_deref().map(day).transpose()?;
            let limit = args.limit.unwrap_or(50).clamp(1, MAX_LIST);
            json(&assistants::meetings(&inner.library, from, to, limit)?)
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
        self.answer("get_meeting", move |inner| {
            Ok(assistants::meeting_markdown(
                &inner.library,
                &args.id,
                args.transcript_from,
            )?)
        })
        .await
    }

    /// The action items from every meeting's notes, newest meeting first:
    /// the task, who owns it ("mine" is the user), when it's due, and the
    /// meeting it came from. Open ones only unless include_done.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn action_items(
        &self,
        Parameters(args): Parameters<ActionArgs>,
    ) -> Result<String, String> {
        self.answer("action_items", move |inner| {
            json(&assistants::action_items(
                &inner.library,
                args.include_done.unwrap_or(false),
            )?)
        })
        .await
    }

    /// The summary of one day's meetings, if the user has had Quill write
    /// one. Otherwise, list that day's meetings and read their notes.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn daily_summary(&self, Parameters(args): Parameters<DayArgs>) -> Result<String, String> {
        self.answer("daily_summary", move |inner| {
            let day = day(&args.date)?;
            let summary = assistants::daily_markdown(&inner.library, day)?;
            Ok(summary.unwrap_or_else(|| format!("No daily summary has been written for {day}.")))
        })
        .await
    }
}

#[tool_handler(
    name = "quill",
    instructions = "Quill holds the user's meeting notes and transcripts, recorded and \
        transcribed on their computer. Use it to answer questions about what was said or \
        decided in meetings: search first, then read the meetings that match. Quote the \
        notes or transcript and name the meeting and its date when you rely on them."
)]
impl ServerHandler for Server {}

#[cfg(test)]
mod tests {
    use quill_core::summary::Summary;
    use quill_stt::Segment;
    use serde_json::{Value, json};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::*;

    fn allow(dir: &Path, on: bool) {
        let settings = Settings {
            assistant_access: on,
            semantic_search: false,
            ..Settings::default()
        };
        settings.save(dir).unwrap();
    }

    /// Talks to the server the way an assistant does: one JSON-RPC message
    /// per line, over an in-memory pipe instead of stdin/stdout.
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
        let dir = tempfile::tempdir().unwrap();
        allow(dir.path(), true);
        let library = Library::open(dir.path(), &dir.path().join("recordings")).unwrap();
        let id = library.create_meeting().unwrap().id;
        let said = |text: &str| Segment {
            start_ms: 0,
            end_ms: 4_000,
            text: text.into(),
            speaker: Some("Priya".into()),
        };
        library
            .save_transcript(&id, &[said("We should raise prices in March.")])
            .unwrap();
        let notes = "# Pricing — 2026-10-20\n\n## Decisions\n- Raise prices in March.\n";
        library
            .save_summary(
                &id,
                &Summary {
                    markdown: notes.into(),
                    model: "test".into(),
                    prompt_version: "test",
                    requests: 1,
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    cost_usd: None,
                    latency_ms: 0,
                },
            )
            .unwrap();

        let server = Server::new(library, dir.path().into(), dir.path().join("models"));
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
                "daily_summary",
                "get_meeting",
                "list_meetings",
                "search"
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

        // Turned off in Settings while connected: nothing more is served.
        allow(dir.path(), false);
        let (refused, failed) = client.tool("list_meetings", json!({})).await;
        assert!(
            failed && refused.contains("Let AI assistants read"),
            "{refused}"
        );
    }
}
