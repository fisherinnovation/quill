//! The tools over MCP's Streamable HTTP transport at
//! `http://127.0.0.1:<port>/mcp`, for an assistant to connect to by URL.
//! The app runs it while Settings → Meetings → Let AI assistants read your
//! meetings is on.
//!
//! Anything on the computer can reach a localhost port, web pages in a
//! browser included, so every request needs the bearer [`token`] (in an
//! `Authorization: Bearer …` header). As well, rmcp turns away a request with
//! any `Origin` (only browsers send one) or a `Host` other than a loopback
//! name (DNS rebinding), and it listens on IPv4 loopback only, so other
//! computers can't reach it at all.
//!
//! Stateless: each request is answered on its own, with JSON, so there are
//! no sessions to keep or expire.
//!
//! [`token`]: crate::token

use std::convert::Infallible;
use std::net::Ipv4Addr;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue, WWW_AUTHENTICATE};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::Context;
use crate::token;
use crate::tools::Server;

/// Where the server answers.
pub const PATH: &str = "/mcp";

type Mcp = StreamableHttpService<Server, NeverSessionManager>;
type Body = BoxBody<Bytes, Infallible>;

/// A running server. Dropping it (or [`HttpServer::stop`]) stops it and
/// closes every connection.
pub struct HttpServer {
    port: u16,
    token: Arc<RwLock<String>>,
    stop: CancellationToken,
}

impl HttpServer {
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The URL an assistant connects to.
    pub fn url(&self) -> String {
        url(self.port)
    }

    /// Changes the token requests must carry, from the next request on.
    pub fn set_token(&self, token: String) {
        *self.token.write().unwrap_or_else(PoisonError::into_inner) = token;
    }

    pub fn stop(self) {}
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

/// The URL of a server on `port`.
pub fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}{PATH}")
}

/// Starts serving on `127.0.0.1:port` (0 for any free port) to requests
/// that carry `token`. Fails if the port can't be had, e.g. another app
/// uses it. Must be called within a Tokio runtime, which it then runs on.
pub async fn serve(
    context: Arc<dyn Context>,
    port: u16,
    token: String,
) -> std::io::Result<HttpServer> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let port = listener.local_addr()?.port();
    let stop = CancellationToken::new();
    let token = Arc::new(RwLock::new(token));
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .enforce_origin_validation()
        .with_cancellation_token(stop.child_token());
    let mcp = StreamableHttpService::new(
        move || Ok(Server::new(Arc::clone(&context))),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    tokio::spawn(accept(listener, mcp, Arc::clone(&token), stop.clone()));
    tracing::info!(port, "serving the library to AI assistants on localhost");
    Ok(HttpServer { port, token, stop })
}

/// Takes connections until stopped, each on a task of its own.
async fn accept(
    listener: TcpListener,
    mcp: Mcp,
    token: Arc<RwLock<String>>,
    stop: CancellationToken,
) {
    loop {
        let stream = tokio::select! {
            () = stop.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(e) => {
                    // E.g. out of file handles: wait rather than spin.
                    tracing::warn!("assistant server couldn't take a connection: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
        };
        let (mcp, token, stop) = (mcp.clone(), Arc::clone(&token), stop.clone());
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |request| {
                let (mcp, token) = (mcp.clone(), Arc::clone(&token));
                async move { Ok::<_, Infallible>(respond(request, &mcp, &token).await) }
            });
            let connection = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service);
            // Stopping drops the connection, closing it mid-request if need be.
            tokio::select! {
                _ = connection => {}
                () = stop.cancelled() => {}
            }
        });
    }
    tracing::info!("stopped serving AI assistants");
}

/// Answers one request: MCP at [`PATH`] with the token, nothing otherwise.
async fn respond(request: Request<Incoming>, mcp: &Mcp, token: &RwLock<String>) -> Response<Body> {
    if request.uri().path() != PATH {
        return plain(StatusCode::NOT_FOUND, "Quill's MCP endpoint is /mcp.");
    }
    let presented = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map_or("", |(_, presented)| presented.trim());
    let allowed = token::matches(
        presented,
        &token.read().unwrap_or_else(PoisonError::into_inner),
    );
    if !allowed {
        tracing::info!("assistant request turned away: no valid token");
        let mut response = plain(
            StatusCode::UNAUTHORIZED,
            "Quill needs the token from Settings → Meetings → AI assistants, as \
             'Authorization: Bearer <token>'.",
        );
        response
            .headers_mut()
            .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        return response;
    }
    mcp.handle(request).await
}

fn plain(status: StatusCode, text: &'static str) -> Response<Body> {
    let mut response = Response::new(Full::new(Bytes::from_static(text.as_bytes())).boxed());
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::testing;

    const TOKEN: &str = "0123456789abcdef";

    async fn post(
        port: u16,
        token: Option<&str>,
        origin: Option<&str>,
        body: Value,
    ) -> (u16, String) {
        let mut request = reqwest::Client::new()
            .post(url(port))
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .body(body.to_string());
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        if let Some(origin) = origin {
            request = request.header("Origin", origin);
        }
        let response = request.send().await.unwrap();
        (response.status().as_u16(), response.text().await.unwrap())
    }

    fn initialize() -> Value {
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1"},
        }})
    }

    fn search() -> Value {
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
            "name": "search", "arguments": {"query": "prices"},
        }})
    }

    #[tokio::test]
    async fn answers_requests_with_the_token_until_stopped() {
        let (context, id) = testing::context();
        let server = serve(context, 0, TOKEN.into()).await.unwrap();
        let port = server.port();
        assert_eq!(server.url(), format!("http://127.0.0.1:{port}/mcp"));

        let (status, body) = post(port, Some(TOKEN), None, initialize()).await;
        assert_eq!(status, 200, "{body}");
        let init: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "quill");

        // Stateless: a tool call needs no session from the handshake.
        let (status, body) = post(port, Some(TOKEN), None, search()).await;
        assert_eq!(status, 200, "{body}");
        let found: Value = serde_json::from_str(&body).unwrap();
        let text = found["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains(&id) && text.contains("[prices]"), "{text}");

        // No token, a wrong one, or a web page with the token: turned away.
        assert_eq!(post(port, None, None, search()).await.0, 401);
        assert_eq!(post(port, Some("wrong"), None, search()).await.0, 401);
        let from_page = post(port, Some(TOKEN), Some("https://example.com"), search()).await;
        assert_eq!(from_page.0, 403, "{}", from_page.1);

        // A new token locks the old one out.
        server.set_token("fedcba9876543210".into());
        assert_eq!(post(port, Some(TOKEN), None, search()).await.0, 401);
        assert_eq!(
            post(port, Some("fedcba9876543210"), None, search()).await.0,
            200
        );

        server.stop();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let refused = reqwest::Client::new().post(url(port)).send().await;
        assert!(refused.is_err(), "still listening after stop");
    }

    #[tokio::test]
    async fn only_the_mcp_path_is_served() {
        let (context, _) = testing::context();
        let server = serve(context, 0, TOKEN.into()).await.unwrap();
        let response = reqwest::Client::new()
            .get(format!("http://127.0.0.1:{}/", server.port()))
            .header("Authorization", format!("Bearer {TOKEN}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 404);
    }

    #[tokio::test]
    async fn a_port_in_use_is_an_error() {
        let (context, _) = testing::context();
        let first = serve(context.clone(), 0, TOKEN.into()).await.unwrap();
        assert!(serve(context, first.port(), TOKEN.into()).await.is_err());
    }
}
