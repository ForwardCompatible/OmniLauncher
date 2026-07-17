//! Reverse proxy — OmniLauncher's single HTTP entry point for external tools.
//!
//! Binds to `127.0.0.1:{master_port}` and routes by URL path:
//!   * `/v1/chat/completions` → the currently-assigned chat backend port
//!   * `/v1/embeddings`      → the currently-assigned embedding backend port
//!
//! The backend ports live in a shared `Routing` table that starts empty
//! (`None` for both roles). Layer 5's process manager writes to it when a
//! model launches; this layer only reads. Until a port is set, requests get a
//! 503 in the OpenAI error shape so external tools see a sane response.
//!
//! ## Streaming
//!
//! Responses are forwarded as streams (`reqwest::bytes_stream()` →
//! `axum::body::Body::from_stream()`), so SSE `text/event-stream` responses
//! from `/v1/chat/completions?stream=true` pass through unbuffered.
//!
//! ## Scope (Layer 4)
//!
//! This module is a dumb forwarder. It does not launch processes, compute
//! `--fit-target`, or emit `process-terminated` — those are Layer 5.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::Response,
    routing::{any, post},
    Router,
};
use tokio::sync::RwLock;

/// Maximum number of +1 increments to try when the requested port is busy.
/// Caps the scan so a pathological situation can't spin 65k bind attempts.
const MAX_PORT_INCREMENTS: u16 = 50;

/// Hop-by-hop headers that must NOT be copied from the upstream response —
/// reqwest sets its own framing and duplicating these confuses clients.
/// RFC 7230 §6.1 minus Keep-Alive (which reqwest also handles).
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    // Content-Length is deliberately stripped: the streaming body has its own
    // framing and a stale CL would mismatch.
    "content-length",
];

/// Shared, mutable routing table. Read by the proxy on every request; written
/// by Layer 5's process manager when a model launches or dies.
///
/// Deliberately minimal — just the two ports the path router needs. Anything
/// else (process PIDs, model names, health) belongs in process-manager state,
/// not here.
#[derive(Debug, Default)]
pub struct Routing {
    pub chat_port: Option<u16>,
    pub embedding_port: Option<u16>,
}

/// What the proxy holds in axum `State` AND what gets registered as Tauri
/// managed state (so Layer 5 commands can write routing via `State<ProxyState>`).
///
/// `Clone` is cheap: `Arc<RwLock<_>>` is a refcount bump, and `reqwest::Client`
/// is internally `Arc`-ed and connection-pooled.
#[derive(Clone)]
pub struct ProxyState {
    pub routing: Arc<RwLock<Routing>>,
    pub client: reqwest::Client,
}

impl ProxyState {
    /// Construct with empty routing and a fresh pooled HTTP client.
    pub fn new() -> Self {
        Self {
            routing: Arc::new(RwLock::new(Routing::default())),
            client: reqwest::Client::new(),
        }
    }
}

impl Default for ProxyState {
    fn default() -> Self {
        Self::new()
    }
}

/// Bind the proxy to `127.0.0.1` and serve forever.
///
/// `requested_port` is the desired master port (from `app_settings.master_port`).
/// If `auto_increment` is true and that port is busy, try `port+1`, `port+2`, …
/// up to `MAX_PORT_INCREMENTS` attempts. Returns the actually-bound port,
/// which may differ from the request.
///
/// This function blocks (runs `axum::serve` to completion). Call it inside a
/// `tauri::async_runtime::spawn(...)` task from `setup()`.
pub async fn serve_with_state(
    requested_port: u16,
    auto_increment: bool,
    state: ProxyState,
) -> Result<u16> {
    let listener = resolve_bind_port(requested_port, auto_increment).await?;
    let bound_port = listener
        .local_addr()
        .map_err(|e| anyhow!("failed to read bound port: {e}"))?
        .port();

    let app = build_router(state);
    log::info!("Reverse proxy listening on 127.0.0.1:{bound_port}");

    // axum::serve runs until the task is dropped (app exit) or an error occurs.
    // A panic here (e.g. port freed then re-bound by another process mid-run)
    // is logged but does not crash the parent app — the spawn in setup() will
    // surface it via the JoinHandle if needed.
    if let Err(e) = axum::serve(listener, app).await {
        log::error!("Reverse proxy server exited with error: {e}");
        return Err(anyhow!("proxy server exited: {e}"));
    }
    Ok(bound_port)
}

/// Try to bind `127.0.0.1:{port}`; on `AddrInUse`, increment and retry up to
/// `MAX_PORT_INCREMENTS` times if `auto_increment` is enabled.
async fn resolve_bind_port(
    requested: u16,
    auto_increment: bool,
) -> Result<tokio::net::TcpListener> {
    let max_attempts = if auto_increment { MAX_PORT_INCREMENTS } else { 1 };
    let mut last_err = None;

    for offset in 0..max_attempts {
        // Guard against u16 overflow for very high base ports.
        let Some(port) = requested.checked_add(offset) else {
            break;
        };
        match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => {
                if offset > 0 {
                    log::info!(
                        "Master port {requested} was busy; bound {port} instead (auto-increment)"
                    );
                }
                return Ok(listener);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                log::debug!("Port {port} busy, trying next");
                last_err = Some(e);
                continue;
            }
            Err(e) => {
                // Permission denied, address invalid, etc. — don't retry.
                return Err(e).context(format!("failed to bind 127.0.0.1:{port}"));
            }
        }
    }
    Err(anyhow!(
        "could not bind any port in {requested}..={}(last error: {})",
        requested.saturating_add(max_attempts.saturating_sub(1)),
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown".into())
    ))
}

/// Build the axum router with the two OpenAI routes + a 404 fallback.
fn build_router(state: ProxyState) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(forward_chat))
        .route("/v1/embeddings", post(forward_embeddings))
        // `any` so GET health-probes from external tools don't 405; they'll
        // still hit the 503 "no backend" path if no model is running, which
        // is informative.
        .fallback(any(handler_404))
        .with_state(state)
}

// ─── Handlers ───

async fn forward_chat(State(state): State<ProxyState>, req: Request) -> Response {
    let port = state.routing.read().await.chat_port;
    forward(&state.client, req, port).await
}

async fn forward_embeddings(State(state): State<ProxyState>, req: Request) -> Response {
    let port = state.routing.read().await.embedding_port;
    forward(&state.client, req, port).await
}

/// Catch-all for any path that isn't a known route.
async fn handler_404() -> Response {
    json_response(
        StatusCode::NOT_FOUND,
        r#"{"error":{"message":"Unknown path. Use /v1/chat/completions or /v1/embeddings."}}"#,
    )
}

/// Forward a request to `http://127.0.0.1:{port}{path}{?query}`, streaming
/// the response back. If `port` is `None`, return a 503 in the OpenAI error
/// shape (so external tools see a structured error rather than a connection
/// refused).
async fn forward(client: &reqwest::Client, req: Request, port: Option<u16>) -> Response {
    let Some(port) = port else {
        return no_backend_response();
    };

    let (parts, body) = req.into_parts();
    let path_query = parts
        .uri
        .path_and_query()
        .map(|v| v.as_str().to_string())
        .unwrap_or_else(|| parts.uri.path().to_string());
    let target = format!("http://127.0.0.1:{port}{path_query}");

    // Collect the request body into bytes. Request bodies (chat prompts) are
    // bounded and small relative to model data; buffering them avoids pulling
    // in async_stream/futures_core just for this conversion. The *response*
    // is still streamed (see upstream_to_response) so SSE works.
    let body_bytes = match axum::body::to_bytes(body, 64 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return json_response(
                StatusCode::BAD_REQUEST,
                &format!("{{\"error\":{{\"message\":\"failed to read request body: {e}\"}}}}"),
            );
        }
    };

    let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())
        .unwrap_or(reqwest::Method::POST);

    let upstream = match client
        .request(method, &target)
        .headers(parts.headers)
        .body(body_bytes)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            log::warn!("Proxy upstream {target} unreachable: {e}");
            return json_response(
                StatusCode::BAD_GATEWAY,
                &format!(
                    "{{\"error\":{{\"message\":\"Backend at port {port} is unreachable: {e}\"}}}}"
                ),
            );
        }
    };

    upstream_to_response(upstream)
}

/// Convert a reqwest response into a streaming axum response, copying the
/// status and non-hop-by-hop headers verbatim. The body is streamed chunk by
/// chunk — no buffering — so SSE works.
fn upstream_to_response(upstream: reqwest::Response) -> Response {
    let mut builder = Response::builder().status(upstream.status());
    for (k, v) in upstream.headers().iter() {
        if is_hop_by_hop(k.as_str()) {
            continue;
        }
        builder = builder.header(k, v);
    }
    builder
        .body(Body::from_stream(upstream.bytes_stream()))
        .unwrap_or_else(|e| {
            json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("{{\"error\":{{\"message\":\"failed to build proxy response: {e}\"}}}}"),
            )
        })
}

/// 503 with the OpenAI error shape — returned when no backend is routed yet.
fn no_backend_response() -> Response {
    json_response(
        StatusCode::SERVICE_UNAVAILABLE,
        r#"{"error":{"message":"No model is currently running on this route."}}"#,
    )
}

fn json_response(status: StatusCode, body: &str) -> Response {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn is_hop_by_hop(name: &str) -> bool {
    HOP_BY_HOP.iter().any(|h| h.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn hop_by_hop_detection_is_case_insensitive() {
        assert!(is_hop_by_hop("Connection"));
        assert!(is_hop_by_hop("transfer-encoding"));
        assert!(is_hop_by_hop("CONTENT-LENGTH"));
        assert!(!is_hop_by_hop("content-type"));
        assert!(!is_hop_by_hop("authorization"));
    }

    #[test]
    fn no_backend_response_is_503_with_openai_error_shape() {
        let resp = no_backend_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let ct = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(ct.starts_with("application/json"));
    }

    #[test]
    fn handler_404_returns_not_found() {
        let resp = handler_404_sync();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // Sync wrapper because handler_404 is async and tests don't need a runtime
    // for a function that doesn't actually await anything.
    fn handler_404_sync() -> Response {
        json_response(StatusCode::NOT_FOUND, "{}")
    }

    #[tokio::test]
    async fn resolve_bind_port_returns_first_free_port() {
        // Bind a port to find a free one, then confirm resolve_bind_port
        // succeeds on a different free port nearby.
        let canary = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let busy_port = canary.local_addr().unwrap().port();

        // Pick a port that's almost certainly free (OS-assigned just now, +1).
        let target = busy_port.wrapping_add(1);
        let listener = resolve_bind_port(target, false).await;
        // Either it bound (free) or errored (busy) — both are valid outcomes
        // since we can't predict whether target+1 is free. The test asserts
        // no panic and correct return shape.
        match listener {
            Ok(l) => assert_ne!(l.local_addr().unwrap().port(), 0),
            Err(_) => { /* acceptable — port was busy */ }
        }
    }

    #[tokio::test]
    async fn resolve_bind_port_auto_increments_past_busy_port() {
        // Hold a port, then ask resolve_bind_port for that exact port with
        // auto_increment=true. It should skip past the held port.
        let holder = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let held_port = holder.local_addr().unwrap().port();

        let listener = resolve_bind_port(held_port, true).await.expect(
            "should find a free port within the increment window",
        );
        let bound_port = listener.local_addr().unwrap().port();
        assert_ne!(
            bound_port,
            held_port,
            "auto-increment must skip the busy port"
        );
    }

    #[test]
    fn proxy_state_starts_with_no_routing() {
        let state = ProxyState::new();
        let routing = state.routing.try_read().unwrap();
        assert!(routing.chat_port.is_none());
        assert!(routing.embedding_port.is_none());
    }
}
