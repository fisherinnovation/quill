//! Lets AI assistants such as Claude read the meeting library, over the Model
//! Context Protocol (MCP), while the user allows it (Settings → Meetings →
//! Let AI assistants read your meetings, `Settings::assistant_access`).
//!
//! What they can read is decided in `quill_core::assistants`; [`tools`] maps
//! it onto MCP tools. Two ways in, both serving the same tools:
//!
//! - [`http`]: Streamable HTTP on `127.0.0.1`, behind a bearer [`token`].
//!   The app runs it while the setting is on, so an assistant connects to a
//!   URL.
//! - [`serve_stdio`]: over stdin/stdout, for an assistant that starts the
//!   server itself (`quill-cli mcp`).
//!
//! Whoever hosts the server says how to reach the library through
//! [`Context`], so the app can share its own (and its search model).
//!
//! [`Settings::assistant_access`]: quill_core::settings::Settings::assistant_access

use std::sync::Arc;

use quill_core::library::Library;
use quill_embed::Embedder;
use rmcp::ServiceExt;

pub mod http;
pub mod token;
pub mod tools;

/// What the server needs from whoever hosts it.
pub trait Context: Send + Sync + 'static {
    /// Whether the user still lets assistants read the library. Asked on
    /// every request, so turning the setting off takes effect at once.
    fn allowed(&self) -> bool;
    fn library(&self) -> &Library;
    /// The search model, if search by meaning is on and it's downloaded
    /// (never downloading it); `None` searches by words only.
    fn embedder(&self) -> Option<Arc<dyn Embedder>>;
}

/// What an assistant is told when access is off.
pub const TURNED_OFF: &str = "Quill isn't sharing meetings with AI assistants. The user can turn \
     it on in Quill: Settings → Meetings → Let AI assistants read your meetings.";

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("MCP: {0}")]
    Protocol(String),
}

/// Serves the tools over stdin/stdout until the assistant closes stdin
/// (when it quits or restarts the server). stdout carries the protocol, so
/// logs must go to stderr.
pub async fn serve_stdio(context: Arc<dyn Context>) -> Result<(), McpError> {
    let running = tools::Server::new(context)
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| McpError::Protocol(e.to_string()))?;
    running
        .waiting()
        .await
        .map_err(|e| McpError::Protocol(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod testing {
    //! A library with one meeting in it, for the tests of both transports.

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use quill_core::library::Library;
    use quill_core::summary::Summary;
    use quill_embed::Embedder;
    use quill_stt::Segment;

    pub struct TestContext {
        pub library: Library,
        pub allowed: AtomicBool,
        _dir: tempfile::TempDir,
    }

    impl super::Context for TestContext {
        fn allowed(&self) -> bool {
            self.allowed.load(Ordering::SeqCst)
        }
        fn library(&self) -> &Library {
            &self.library
        }
        fn embedder(&self) -> Option<Arc<dyn Embedder>> {
            None
        }
    }

    /// The context, and the id of its one meeting: Priya on pricing.
    pub fn context() -> (Arc<TestContext>, String) {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(dir.path(), &dir.path().join("recordings")).unwrap();
        let id = library.create_meeting().unwrap().id;
        let line = Segment {
            start_ms: 0,
            end_ms: 4_000,
            text: "We should raise prices in March.".into(),
            speaker: Some("Priya".into()),
        };
        library.save_transcript(&id, &[line]).unwrap();
        let notes = "# Pricing — 2026-10-20\n\n## Decisions\n- Raise prices in March.\n";
        let summary = Summary {
            markdown: notes.into(),
            model: "test".into(),
            prompt_version: "test",
            requests: 1,
            prompt_tokens: 0,
            completion_tokens: 0,
            cost_usd: None,
            latency_ms: 0,
        };
        library.save_summary(&id, &summary).unwrap();
        let context = TestContext {
            library,
            allowed: AtomicBool::new(true),
            _dir: dir,
        };
        (Arc::new(context), id)
    }
}
