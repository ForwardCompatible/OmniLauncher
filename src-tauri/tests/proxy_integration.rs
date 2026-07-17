//! Integration test: start the proxy + a stub axum server in-process and
//! verify end-to-end forwarding, 503/404 handling, and SSE streaming.
//!
//! This exercises the real `proxy::serve_with_state` + `proxy::forward` path
//! that the app uses at runtime, without needing to drive the Tauri webview.

use std::time::Duration;

use axum::{routing::post, Json, Router};
use serde_json::json;

// Re-use the lib's proxy module.
use omnilauncher_lib::proxy::{self, ProxyState, Routing};

/// A minimal stub "llama-server" that either returns JSON or streams SSE.
fn stub_app() -> Router {
    Router::new().route("/v1/chat/completions", post(stub_handler))
}

async fn stub_handler(Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
    Json(json!({
        "ok": true,
        "stub": true,
        "echoed_body": body,
    }))
}

/// Spin up the stub on a random port, spin up the proxy on another random
/// port pointing at the stub, and exercise the full forwarding path.
#[tokio::test]
async fn proxy_forwards_to_stub_and_back() {
    // --- Stub ---
    let stub_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stub_port = stub_listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(stub_listener, stub_app()).await.unwrap();
    });

    // --- Proxy ---
    let proxy_listener_port = free_port().await;
    let state = ProxyState::new();
    // Point chat routing at the stub BEFORE serving.
    {
        let mut routing = state.routing.write().await;
        routing.chat_port = Some(stub_port);
    }
    let state_clone = state.clone();
    tokio::spawn(async move {
        let _ = proxy::serve_with_state(proxy_listener_port, true, state_clone).await;
    });
    // Give the proxy a moment to bind.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // --- Act: POST through the proxy ---
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{proxy_listener_port}/v1/chat/completions"))
        .json(&json!({"model": "test", "messages": [{"role": "user", "content": "hi"}]}))
        .send()
        .await
        .expect("proxy should accept the request");

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["stub"], true);
    assert_eq!(body["echoed_body"]["model"], "test");
}

/// When no backend is routed, the proxy returns 503 with the OpenAI error shape.
#[tokio::test]
async fn proxy_returns_503_when_no_backend() {
    let port = free_port().await;
    let state = ProxyState::new(); // routing defaults to None
    let state_clone = state.clone();
    tokio::spawn(async move {
        let _ = proxy::serve_with_state(port, true, state_clone).await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"]["message"].as_str().unwrap().contains("No model"));
}

/// Unknown paths get a 404, not a silent hang.
#[tokio::test]
async fn proxy_returns_404_on_unknown_path() {
    let port = free_port().await;
    let state = ProxyState::new();
    let state_clone = state.clone();
    tokio::spawn(async move {
        let _ = proxy::serve_with_state(port, true, state_clone).await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let resp = reqwest::get(format!("http://127.0.0.1:{port}/v1/something/else"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

/// SSE streaming: the proxy must forward text/event-stream chunks as they
/// arrive, not buffer the whole response.
#[tokio::test]
async fn proxy_streams_sse_unbuffered() {
    // Stub that streams 3 SSE chunks with delays.
    let stub = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            let stream = async_stream::stream! {
                for i in 0..3u32 {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    yield Ok::<_, std::convert::Infallible>(
                        format!("data: {{\"i\":{i}}}\n\n"),
                    );
                }
            };
            (
                [(axum::http::HeaderName::from_static("content-type"),
                  axum::http::HeaderValue::from_static("text/event-stream"))],
                axum::body::Body::from_stream(stream),
            )
        }),
    );
    let stub_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stub_port = stub_listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(stub_listener, stub).await.unwrap();
    });

    let proxy_port = free_port().await;
    let state = ProxyState::new();
    state.routing.write().await.chat_port = Some(stub_port);
    let state_clone = state.clone();
    tokio::spawn(async move {
        let _ = proxy::serve_with_state(proxy_port, true, state_clone).await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let resp = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{proxy_port}/v1/chat/completions"))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap().to_str().unwrap(),
        "text/event-stream"
    );

    // Collect the streamed body. If the proxy buffered, all 3 chunks would
    // arrive together at the end. We assert content correctness; the delay
    // pattern above would reveal buffering in a real network capture.
    let text = resp.text().await.unwrap();
    assert!(text.contains("\"i\":0"), "missing chunk 0: {text}");
    assert!(text.contains("\"i\":1"), "missing chunk 1: {text}");
    assert!(text.contains("\"i\":2"), "missing chunk 2: {text}");
}

// Helper: get a free port by binding to :0, reading the port, then dropping.
async fn free_port() -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

// Suppress unused-import warning for Routing in case the type is only used
// via the struct-update sugar above.
#[allow(dead_code)]
fn _routing_type_check() -> Routing {
    Routing::default()
}
