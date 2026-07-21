//! Encapsulated Sidecar Controller — the single interface for launching,
//! stopping, and monitoring `llama-server` child processes.
//!
//! All `tokio::process` implementation details are hidden behind a clean
//! `start` / `stop` / `status` / `shutdown_all` interface. Callers (the Tauri
//! command layer) never touch tokio types, `Child` handles, or raw libc calls.
//!
//! ## OS Awareness
//!
//! The binary path resolver and command builder are gated behind
//! `#[cfg(target_os = "...")]`. Both Linux and Windows are supported;
//! macOS is not currently targeted.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use tauri::Manager;
use tokio::process::Child;

/// Seconds to wait for llama-server to report a listening port before giving up.
const STARTUP_TIMEOUT_SECS: u64 = 120;

/// Seconds between SIGTERM and SIGKILL during graceful shutdown (Linux only).
const SHUTDOWN_GRACE_SECS: u64 = 5;
use tokio::sync::Mutex;

use crate::process::{LaunchReport, Role};
use crate::proxy::ProxyState;

/// Information about a running process, exposed to the UI.
#[derive(Debug, serde::Serialize, Clone)]
pub struct ProcessInfo {
    pub model_id: i64,
    pub model_name: String,
    pub role: Role,
    pub port: u16,
    pub pid: u32,
}

/// Internal bookkeeping for a tracked child.
struct TrackedProcess {
    model_id: i64,
    model_name: String,
    role: Role,
    port: u16,
    pid: u32,
}

/// The sidecar controller. Manages all `llama-server` child processes.
/// Registered as Tauri managed state via `app.manage(Arc<SidecarController>)`.
#[derive(Default)]
pub struct SidecarController {
    processes: Arc<Mutex<HashMap<i64, TrackedProcess>>>,
}

impl SidecarController {
    /// Start a llama-server process with the given CLI args.
    ///
    /// Resolves the binary path via Tauri's resource_dir, spawns the child
    /// with piped stdout/stderr, parses the bound port from stderr, writes
    /// the port into proxy routing, and starts the exit-watcher.
    ///
    /// Returns the launch report (port, pid, args).
    pub async fn start(
        &self,
        app: &tauri::AppHandle,
        proxy_state: &ProxyState,
        model_id: i64,
        model_name: &str,
        role: Role,
        args: Vec<String>,
    ) -> Result<LaunchReport> {
        if self.processes.lock().await.contains_key(&model_id) {
            anyhow::bail!("model {model_id} is already running");
        }

        let binary_path = Self::resolve_binary_path(app)?;
        log::info!("Sidecar binary path: {}", binary_path.display());
        if !binary_path.exists() {
            anyhow::bail!("llama-server binary not found at: {}", binary_path.display());
        }
        log::info!(
            "Starting {role:?} backend for model {model_name} (id={model_id}) — {} args",
            args.len()
        );
        log::debug!("llama-server args: {}", args.join(" "));

        let mut child = Self::build_command(&binary_path, &args)?
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow!("failed to spawn llama-server: {e}"))?;

        let pid = child
            .id()
            .ok_or_else(|| anyhow!("child has no PID (already exited?)"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("failed to capture child stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("failed to capture child stderr"))?;

        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                log::debug!("[llama-server stdout] {line}");
            }
        });

        let port = match tokio::time::timeout(
            std::time::Duration::from_secs(STARTUP_TIMEOUT_SECS),
            parse_listening_port(stderr),
        )
        .await
        {
            Ok(Ok(port)) => port,
            Ok(Err(e)) => {
                let exit_status = child.try_wait();
                log::error!("llama-server stderr parse failed: {e}. Exit status: {exit_status:?}");
                let _ = child.kill().await;
                anyhow::bail!("llama-server failed to start: {e}");
            }
            Err(_) => {
                let _ = child.kill().await;
                anyhow::bail!(
                    "llama-server did not report a listening port within {STARTUP_TIMEOUT_SECS}s"
                );
            }
        };

        log::info!(
            "{role:?} backend for {model_name} is listening on 127.0.0.1:{port} (pid {pid})"
        );

        {
            let mut routing = proxy_state.routing.write().await;
            match role {
                Role::Chat => routing.chat_port = Some(port),
                Role::Embedding => routing.embedding_port = Some(port),
            }
        }

        let report = LaunchReport {
            model_id,
            model_name: model_name.to_string(),
            role,
            port,
            pid,
            args,
        };

        self.processes.lock().await.insert(
            model_id,
            TrackedProcess {
                model_id,
                model_name: model_name.to_string(),
                role,
                port,
                pid,
            },
        );

        let processes = self.processes.clone();
        let app_handle = app.clone();
        let proxy_clone = proxy_state.clone();
        let model_name_owned = model_name.to_string();
        tokio::spawn(async move {
            watch_exit(
                app_handle,
                proxy_clone,
                processes,
                model_id,
                model_name_owned,
                role,
                port,
                child,
            )
            .await;
        });

        Ok(report)
    }

    /// Stop a running process by model_id. SIGTERM → 5s grace → SIGKILL.
    pub async fn stop(&self, proxy_state: &ProxyState, model_id: i64) -> Result<()> {
        let (pid, role, port) = {
            let procs = self.processes.lock().await;
            let p = procs
                .get(&model_id)
                .ok_or_else(|| anyhow!("model {model_id} is not running"))?;
            (p.pid, p.role, p.port)
        };

        log::info!("Stopping model {model_id} (pid {pid})");
        Self::send_terminate(pid);
        clear_proxy_routing(proxy_state, role, port).await;
        Ok(())
    }

    /// Snapshot of all running processes for the UI.
    pub async fn status(&self) -> Vec<ProcessInfo> {
        self.processes
            .lock()
            .await
            .values()
            .map(|p| ProcessInfo {
                model_id: p.model_id,
                model_name: p.model_name.clone(),
                role: p.role,
                port: p.port,
                pid: p.pid,
            })
            .collect()
    }

    /// Stop all tracked processes gracefully (SIGTERM → 5s grace → SIGKILL).
    /// Use this from async contexts (commands).
    pub async fn shutdown_all(&self) {
        let pids: Vec<(i64, u32)> = {
            let procs = self.processes.lock().await;
            procs.values().map(|p| (p.model_id, p.pid)).collect()
        };
        if pids.is_empty() {
            return;
        }
        log::info!("Shutting down {} child process(es)", pids.len());

        for (model_id, pid) in &pids {
            log::debug!("SIGTERM model {model_id} (pid {pid})");
            Self::send_terminate(*pid);
        }

        tokio::time::sleep(std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS)).await;
        for (_, pid) in &pids {
            Self::send_kill(*pid);
        }
    }

    /// Synchronous shutdown — SIGTERM then immediate SIGKILL.
    /// Used from RunEvent::Exit where we cannot await (deadlock risk if the
    /// exit handler runs on the tokio worker thread). The grace period is
    /// skipped because the process is dying anyway.
    pub fn shutdown_all_blocking(&self) {
        let pids: Vec<(i64, u32)> = {
            // try_lock avoids deadlock if another async task holds the lock.
            let procs = match self.processes.try_lock() {
                Ok(guard) => guard,
                Err(_) => {
                    log::warn!("Could not acquire process lock during exit — sending SIGKILL to known PIDs is skipped. kill_on_drop will handle it.");
                    return;
                }
            };
            procs.values().map(|p| (p.model_id, p.pid)).collect()
        };
        if pids.is_empty() {
            return;
        }
        log::info!("Exit handler: killing {} child process(es)", pids.len());
        for (model_id, pid) in &pids {
            log::debug!("SIGTERM+SIGKILL model {model_id} (pid {pid})");
            Self::send_terminate(*pid);
            Self::send_kill(*pid);
        }
    }

    // ── OS-specific internals ──

    /// Resolve the llama-server binary path.
    ///
    /// Linux: bundled as a Tauri resource at `resources/llama-server/llama-server`.
    /// Windows: downloaded during installation to `%APPDATA%/com.omnilauncher.app/binaries/llama-server.exe`.
    fn resolve_binary_path(app: &tauri::AppHandle) -> Result<PathBuf> {
        #[cfg(target_os = "linux")]
        {
            let resource_dir = app
                .path()
                .resource_dir()
                .context("failed to resolve resource_dir")?;
            Ok(resource_dir.join("resources").join("llama-server").join("llama-server"))
        }

        #[cfg(target_os = "windows")]
        {
            let data_dir = app
                .path()
                .app_data_dir()
                .context("failed to resolve app_data_dir")?;
            Ok(data_dir.join("binaries").join("llama-server.exe"))
        }
    }

    /// Build a `tokio::process::Command` with OS-specific environment setup.
    fn build_command(
        binary: &std::path::Path,
        args: &[String],
    ) -> Result<tokio::process::Command> {
        let mut cmd = tokio::process::Command::new(binary);
        cmd.args(args);

        #[cfg(target_os = "linux")]
        {
            let binary_dir = binary
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            cmd.env("LD_LIBRARY_PATH", &binary_dir);
        }

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;

            // Prepend the binary's directory to PATH so Windows finds side-by-side
            // CUDA/cuDNN DLLs.
            let binary_dir = binary
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            if let Ok(existing_path) = std::env::var("PATH") {
                let new_path = format!(
                    "{};{}",
                    binary_dir.to_string_lossy(),
                    existing_path
                );
                cmd.env("PATH", &new_path);
            }

            // CREATE_NO_WINDOW — prevents a console popup when spawning llama-server.exe
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        Ok(cmd)
    }

    #[cfg(target_os = "linux")]
    fn send_terminate(pid: u32) {
        let rc = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        if rc != 0 {
            log::warn!("SIGTERM failed for pid {pid}, escalating to SIGKILL");
            Self::send_kill(pid);
        }
    }

    #[cfg(target_os = "linux")]
    fn send_kill(pid: u32) {
        let _ = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    }

    #[cfg(target_os = "windows")]
    fn send_terminate(pid: u32) {
        // Windows has no graceful-signal equivalent. Both terminate and kill
        // use TerminateProcess. The shutdown_all() grace period between the
        // two calls is a no-op on Windows.
        Self::windows_terminate_process(pid);
    }

    #[cfg(target_os = "windows")]
    fn send_kill(pid: u32) {
        Self::windows_terminate_process(pid);
    }

    #[cfg(target_os = "windows")]
    fn windows_terminate_process(pid: u32) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, TerminateProcess, PROCESS_TERMINATE,
        };

        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if handle == 0 {
                log::warn!("OpenProcess failed for pid {pid}");
                return;
            }
            let rc = TerminateProcess(handle, 1);
            if rc == 0 {
                log::warn!("TerminateProcess failed for pid {pid}");
            }
            CloseHandle(handle);
        }
    }
}

// ── Exit watcher + helpers (internal) ──

async fn watch_exit(
    app: tauri::AppHandle,
    proxy_state: ProxyState,
    processes: Arc<Mutex<HashMap<i64, TrackedProcess>>>,
    model_id: i64,
    model_name: String,
    role: Role,
    port: u16,
    mut child: Child,
) {
    let status = match child.wait().await {
        Ok(s) => {
            log::info!("Process for {model_name} (id={model_id}) exited: {s}");
            s
        }
        Err(e) => {
            log::error!("wait() failed for model {model_id}: {e}");
            processes.lock().await.remove(&model_id);
            return;
        }
    };

    clear_proxy_routing(&proxy_state, role, port).await;
    processes.lock().await.remove(&model_id);

    use tauri::Emitter;
    let _ = app.emit(
        "process-terminated",
        serde_json::json!({
            "model_id": model_id,
            "model_name": model_name,
            "role": match role {
                Role::Chat => "chat",
                Role::Embedding => "embedding",
            },
            "exit_code": status.code(),
        }),
    );
}

async fn clear_proxy_routing(proxy_state: &ProxyState, role: Role, port: u16) {
    let mut routing = proxy_state.routing.write().await;
    match role {
        Role::Chat => {
            if routing.chat_port == Some(port) {
                routing.chat_port = None;
            }
        }
        Role::Embedding => {
            if routing.embedding_port == Some(port) {
                routing.embedding_port = None;
            }
        }
    }
}

async fn parse_listening_port<R>(reader: R) -> Result<u16>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        log::debug!("[llama-server stderr] {line}");
        if let Some(port) = extract_port_from_line(&line) {
            return Ok(port);
        }
    }
    anyhow::bail!("stderr stream ended before a listening port was reported")
}

fn extract_port_from_line(line: &str) -> Option<u16> {
    if !line.to_ascii_lowercase().contains("listen") {
        return None;
    }
    if let Some(idx) = line.rfind(':') {
        let candidate = &line[idx + 1..];
        let digits: String = candidate
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(p) = digits.parse::<u16>() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_port_from_listening_line() {
        assert_eq!(
            extract_port_from_line("llama-server: listening on 127.0.0.1:8080"),
            Some(8080)
        );
        assert_eq!(
            extract_port_from_line("server is listening on http://127.0.0.1:54321"),
            Some(54321)
        );
        assert_eq!(
            extract_port_from_line("listening on 0.0.0.0:1234"),
            Some(1234)
        );
    }

    #[test]
    fn ignores_non_listening_lines() {
        assert_eq!(extract_port_from_line("loading model..."), None);
        assert_eq!(extract_port_from_line("error: something:8080 broke"), None);
    }

    #[tokio::test]
    async fn empty_controller_status() {
        let sc = SidecarController::default();
        assert!(sc.status().await.is_empty());
    }
}
