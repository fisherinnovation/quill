//! Serving the library to AI assistants (`quill_mcp::http`) on
//! `http://127.0.0.1:<port>/mcp` while Settings → Meetings → Let AI
//! assistants read your meetings is on: started when it's turned on or
//! Quill starts with it on, stopped when it's turned off or Quill quits.
//! No restart is needed either way. The UI asks for confirmation before
//! turning it on.
//!
//! The token assistants need is kept in the keychain (`quill_mcp::token`)
//! and read once per launch; Settings shows it while the server runs.

use std::sync::{Arc, Mutex};

use quill_core::library::Library;
use quill_embed::Embedder;
use quill_mcp::http::HttpServer;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::AppState;
use crate::commands::{CommandError, CommandResult, lock};

/// Sent whenever [`AssistantServer`] changes.
pub const ASSISTANT_EVENT: &str = "assistant-server";

/// Kept in Tauri's state.
#[derive(Default)]
pub struct Assistants {
    /// Held while starting or stopping, so changes apply one at a time.
    server: tokio::sync::Mutex<Option<HttpServer>>,
    /// Read from the keychain once per launch.
    token: Mutex<Option<String>>,
    status: Mutex<AssistantServer>,
}

/// What Settings shows about the server.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct AssistantServer {
    pub running: bool,
    /// Where assistants connect, while running.
    pub url: Option<String>,
    /// What they must send, while running.
    pub token: Option<String>,
    /// Why it isn't running although it's on (the port is taken, the
    /// keychain refused).
    pub error: Option<String>,
}

/// Starts, restarts (on a new port) or stops the server to match the
/// settings, in the background; [`ASSISTANT_EVENT`] tells the UI how it went.
pub fn apply(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move { reconcile(&app).await });
}

async fn reconcile(app: &AppHandle) {
    let (on, port) = {
        let state = app.state::<AppState>();
        let settings = lock(&state.settings);
        (settings.assistant_access, settings.assistant_port)
    };
    let assistants = app.state::<Assistants>();
    let mut server = assistants.server.lock().await;
    if on && server.as_ref().is_some_and(|s| s.port() == port) {
        return;
    }
    if let Some(running) = server.take() {
        running.stop();
    }
    if !on {
        set_status(app, AssistantServer::default());
        return;
    }
    let token = match token(app).await {
        Ok(token) => token,
        Err(e) => {
            tracing::warn!("assistant server: {e}");
            set_status(app, failed(format!("Couldn't read its token: {e}")));
            return;
        }
    };
    let context = Arc::new(AppContext(app.clone()));
    match quill_mcp::http::serve(context, port, token.clone()).await {
        Ok(started) => {
            set_status(
                app,
                AssistantServer {
                    running: true,
                    url: Some(started.url()),
                    token: Some(token),
                    error: None,
                },
            );
            *server = Some(started);
        }
        Err(e) => {
            tracing::warn!(port, "assistant server couldn't start: {e}");
            let error = if e.kind() == std::io::ErrorKind::AddrInUse {
                format!("Port {port} is in use by another app. Choose another port.")
            } else {
                format!("Couldn't listen on port {port}: {e}")
            };
            set_status(app, failed(error));
        }
    }
}

fn failed(error: String) -> AssistantServer {
    AssistantServer {
        error: Some(error),
        ..AssistantServer::default()
    }
}

/// The token, from the keychain the first time (making one if need be).
async fn token(app: &AppHandle) -> Result<String, String> {
    if let Some(token) = lock(&app.state::<Assistants>().token).clone() {
        return Ok(token);
    }
    let token = tauri::async_runtime::spawn_blocking(quill_mcp::token::load_or_create)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    *lock(&app.state::<Assistants>().token) = Some(token.clone());
    Ok(token)
}

fn set_status(app: &AppHandle, status: AssistantServer) {
    let assistants = app.state::<Assistants>();
    let mut current = lock(&assistants.status);
    if *current != status {
        *current = status.clone();
        if let Err(e) = app.emit(ASSISTANT_EVENT, status) {
            tracing::warn!("could not emit assistant server status: {e}");
        }
    }
}

/// The library as assistants reach it: the app's own, and its search model.
struct AppContext(AppHandle);

impl quill_mcp::Context for AppContext {
    fn allowed(&self) -> bool {
        lock(&self.0.state::<AppState>().settings).assistant_access
    }

    fn library(&self) -> &Library {
        &self.0.state::<AppState>().inner().library
    }

    fn embedder(&self) -> Option<Arc<dyn Embedder>> {
        crate::semantic::embedder(&self.0).map(|e| e as Arc<dyn Embedder>)
    }
}

/// Where the server stands, for Settings.
#[tauri::command]
pub fn assistant_server(app: AppHandle) -> AssistantServer {
    lock(&app.state::<Assistants>().status).clone()
}

/// Replaces the token: assistants set up with the old one stop working
/// until they're given the new one.
#[tauri::command]
pub async fn renew_assistant_token(app: AppHandle) -> CommandResult<AssistantServer> {
    let token = tauri::async_runtime::spawn_blocking(quill_mcp::token::renew)
        .await
        .map_err(|e| CommandError::Other(e.to_string()))?
        .map_err(|e| CommandError::Other(e.to_string()))?;
    let assistants = app.state::<Assistants>();
    *lock(&assistants.token) = Some(token.clone());
    if let Some(server) = assistants.server.lock().await.as_ref() {
        server.set_token(token.clone());
        let status = lock(&assistants.status).clone();
        set_status(
            &app,
            AssistantServer {
                token: Some(token),
                ..status
            },
        );
    }
    tracing::info!("renewed the assistant token");
    Ok(assistant_server(app.clone()))
}
