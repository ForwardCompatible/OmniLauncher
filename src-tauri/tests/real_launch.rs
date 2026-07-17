//! Real end-to-end launch test for the SidecarController.
//!
//! Uses build_args + the SidecarController to launch the actual
//! Qwen3-Embedding-0.6B model, parse its port, curl an embeddings request
//! through a real proxy, and confirm vectors come back.
//!
//! Run with: cargo test --test real_launch -- --nocapture --ignored

use std::time::Duration;

use omnilauncher_lib::db::registry_ops::ModelSettings;
use omnilauncher_lib::hardware::HardwareProfile;
use omnilauncher_lib::process::{self, Role};
use omnilauncher_lib::proxy::{self, ProxyState};

const BINARY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/llama-server/llama-server"
);
const BINARY_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/llama-server"
);
const MODEL: &str = "/home/ryan/.local/share/com.omnilauncher.app/models/Qwen3-Embedding-0.6B-f16.gguf";

#[tokio::test]
#[ignore]
async fn real_embedding_round_trip_through_proxy() {
    // 1. Start the proxy on a random port.
    let proxy_port = free_port().await;
    let proxy_state = ProxyState::new();
    let state_clone = proxy_state.clone();
    tokio::spawn(async move {
        let _ = proxy::serve_with_state(proxy_port, true, state_clone).await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    println!("proxy listening on 127.0.0.1:{proxy_port}");

    // 2. Build args for the embedding model (small ctx, GPU path).
    let hw = HardwareProfile {
        gpu_name: "test".into(),
        total_vram_mb: 6144,
        total_system_ram_mb: 62000,
        cpu_physical_cores: 6,
        cpu_logical_threads: 12,
        last_scanned_at: "2026-07-10T00:00:00Z".into(),
        gpu_present: true,
    };
    let mut settings = ModelSettings::default();
    settings.vram_allocation_mb = Some(4096);
    settings.ctx_size = Some(2048);
    let args = process::build_args(MODEL, &settings, &hw, process::Role::Embedding);
    println!("args: {}", args.join(" "));

    // 3. Spawn llama-server directly (bypassing SidecarController which needs
    //    an AppHandle for event emission). We manually parse the port and
    //    write routing — the same internal logic the controller uses.
    let mut child = tokio::process::Command::new(BINARY)
        .args(&args)
        .env("LD_LIBRARY_PATH", BINARY_DIR)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn failed");
    let pid = child.id().unwrap();
    println!("spawned llama-server (pid {pid}), waiting for listening port...");

    // 4. Parse the port from stderr.
    let stderr = child.stderr.take().unwrap();
    let port = match tokio::time::timeout(
        Duration::from_secs(120),
        parse_port_from_stream(stderr),
    )
    .await
    {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => panic!("llama-server failed to start: {e}"),
        Err(_) => panic!("timed out waiting for listening port"),
    };
    println!("llama-server listening on 127.0.0.1:{port}");

    // 5. Write the port into proxy routing.
    proxy_state.routing.write().await.embedding_port = Some(port);

    // 6. Wait for model to finish loading.
    println!("waiting for model to load...");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // 7. Curl an embeddings request through the proxy.
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{proxy_port}/v1/embeddings"))
        .json(&serde_json::json!({ "model": "test", "input": "hello world" }))
        .send()
        .await
        .expect("proxy request failed");

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    println!("response status: {status}");

    assert!(status.is_success(), "embeddings request should succeed");

    let has_data = body["data"].is_array();
    let has_embedding = body["data"][0]["embedding"].is_array();
    assert!(has_data, "response should have a 'data' array");
    assert!(has_embedding, "response should have embedding vectors");

    let vec_len = body["data"][0]["embedding"].as_array().unwrap().len();
    println!("got embedding vector of length {vec_len}");

    // 8. Clean up.
    let _ = child.kill().await;
    println!("killed llama-server (pid {pid})");
}

async fn parse_port_from_stream<R>(reader: R) -> anyhow::Result<u16>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        eprintln!("[llama-server] {line}");
        if line.to_ascii_lowercase().contains("listen") {
            if let Some(idx) = line.rfind(':') {
                let candidate = &line[idx + 1..];
                let digits: String = candidate.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(p) = digits.parse::<u16>() {
                    return Ok(p);
                }
            }
        }
    }
    anyhow::bail!("stderr ended without a listening line")
}

async fn free_port() -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}
