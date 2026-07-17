//! Live spawn verification for the SidecarController.
//!
//! Escalates from cheap (--version) to verifying the controller's
//! state-query API works on an empty state.

use std::time::Duration;

use omnilauncher_lib::sidecar::SidecarController;
use omnilauncher_lib::proxy::ProxyState;

const BINARY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/llama-server/llama-server"
);

/// The binary's directory — needed for LD_LIBRARY_PATH since the bundled
/// llama-server loads sibling .so files.
const BINARY_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/resources/llama-server"
);

/// Confirm the binary path resolves and runs. llama-server writes its version
/// banner to stderr, so check both streams.
#[tokio::test]
async fn binary_runs_version() {
    let output = tokio::process::Command::new(BINARY)
        .arg("--version")
        .env("LD_LIBRARY_PATH", BINARY_DIR)
        .output()
        .await
        .expect("failed to spawn --version");
    assert!(output.status.success(), "--version should exit 0");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("version"), "expected 'version' in output: {combined}");
    println!("llama-server --version: {combined}");
}

/// Spawning --version with kill_on_drop + piped IO must not leave an orphan.
#[tokio::test]
async fn spawn_version_no_orphan() {
    let mut child = tokio::process::Command::new(BINARY)
        .arg("--version")
        .env("LD_LIBRARY_PATH", BINARY_DIR)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn failed");
    let pid = child.id().expect("no pid");
    let _ = child.wait().await.expect("wait failed");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let alive = unsafe { libc::kill(pid as i32, 0) } == 0;
    assert!(!alive, "pid {pid} should be gone (no orphan)");
    println!("--version process reaped, no orphan");
}

/// The SidecarController's status() API works on an empty state.
#[tokio::test]
async fn controller_status_on_empty_state() {
    let sc = SidecarController::default();
    let list = sc.status().await;
    assert!(list.is_empty(), "fresh controller should have no processes");
    println!("controller status() works on empty state");
}

/// Confirm the proxy routing integration: a ProxyState starts with no routing
/// and can be written/cleared.
#[tokio::test]
async fn proxy_routing_clear_works() {
    let proxy = ProxyState::new();
    {
        let mut r = proxy.routing.write().await;
        r.chat_port = Some(8080);
    }
    let r = proxy.routing.read().await;
    assert_eq!(r.chat_port, Some(8080));
    drop(r);

    {
        let mut r = proxy.routing.write().await;
        r.chat_port = None;
    }
    let r = proxy.routing.read().await;
    assert_eq!(r.chat_port, None);
    println!("proxy routing write/clear works");
}
