//! Live hardware telemetry sampler.
//!
//! A long-lived background task reads CPU%, RAM, and (optionally) VRAM every
//! `INTERVAL` and emits a `hardware-stats` Tauri event carrying a
//! [`HardwareStats`] payload. The footer renders from this; no frontend polling.
//!
//! This is a *separate* layer from [`crate::hardware`], which is the one-shot
//! scan that populates the persisted `hardware_profile` row. Live stats here are
//! ephemeral — never persisted — and re-derive GPU presence on every tick via
//! the long-lived `Nvml` handle.
//!
//! ## Cross-platform
//!
//! `nvml-wrapper` loads `libnvidia-ml.so.1` (Linux) or `nvml.dll` (Windows) at
//! runtime; both expose the same `memory_info()` API. `sysinfo`'s
//! `total_memory()` / `available_memory()` / `global_cpu_usage()` are confirmed
//! present on both backends.
//!
//! ## Send/Sync
//!
//! [`nvml_wrapper::Nvml`] is `Send + Sync` (compile-time asserted by the crate
//! via `static_assertions::assert_impl_all!`), so it can move into a
//! `tauri::async_runtime::spawn` task and be held there for the app lifetime.

use std::sync::Arc;
use std::time::Duration;

use nvml_wrapper::Nvml;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use tokio::sync::RwLock;

use tauri::Emitter;

/// How often the sampler ticks and emits `hardware-stats`.
const INTERVAL: Duration = Duration::from_secs(2);

/// One telemetry sample. Serialized over the wire to the frontend, which
/// formats the `X/Total GB` display string. VRAM fields are `None` on hosts
/// with no usable NVIDIA GPU.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HardwareStats {
    /// Aggregate CPU utilization, 0.0–100.0.
    pub cpu_usage_percent: f32,
    /// Used system RAM in MiB (= total − available).
    pub ram_used_mb: u64,
    /// Total system RAM in MiB.
    pub ram_total_mb: u64,
    /// Used VRAM in MiB. `None` when CPU-only or NVML read fails.
    pub vram_used_mb: Option<u64>,
    /// Total VRAM in MiB. `None` when CPU-only or NVML read fails.
    pub vram_total_mb: Option<u64>,
}

/// Long-lived monitor. Constructed once at startup; the sampler loop runs in a
/// spawned task and writes [`latest`](Self::latest) on every tick. Commands read
/// `latest()` for on-demand access (e.g. before the first event arrives).
pub struct HardwareMonitor {
    latest: Arc<RwLock<HardwareStats>>,
}

impl HardwareMonitor {
    /// Construct without starting the loop. The initial `latest` is a zeroed
    /// placeholder; the real first sample arrives within one `INTERVAL`.
    pub fn new() -> Self {
        Self {
            latest: Arc::new(RwLock::new(HardwareStats {
                cpu_usage_percent: 0.0,
                ram_used_mb: 0,
                ram_total_mb: 0,
                vram_used_mb: None,
                vram_total_mb: None,
            })),
        }
    }

    /// Read the most recent sample. Cheap read lock, no sampling work.
    pub async fn latest(&self) -> HardwareStats {
        self.latest.read().await.clone()
    }

    /// Spawn the sampling loop. Runs until the app exits; errors are logged,
    /// not surfaced. The `Nvml` and `System` handles are constructed here (once)
    /// and owned by the task for its lifetime.
    ///
    /// CPU-only hosts (`Nvml::init()` fails) get `None` for both VRAM fields on
    /// every tick — graceful degradation, not an error.
    pub fn run(self: Arc<Self>, app: tauri::AppHandle) {
        // NVML: init once. Failure is expected on CPU-only hosts; log once at
        // DEBUG and proceed with gpu = None for the loop's lifetime.
        let nvml = match Nvml::init() {
            Ok(n) => {
                log::info!("Hardware monitor: NVML initialized, tracking live VRAM");
                Some(n)
            }
            Err(e) => {
                log::debug!("Hardware monitor: NVML unavailable (CPU-only): {e}");
                None
            }
        };

        // sysinfo: construct empty, then refresh CPU+memory explicitly per tick.
        // We avoid `new_all()` because it refreshes far more (disks, processes)
        // than this tight poll loop needs.
        let mut sys =
            System::new_with_specifics(RefreshKind::nothing().with_cpu(CpuRefreshKind::everything()));

        // Seed the first CPU sample so the *next* tick's `global_cpu_usage()`
        // reflects a real delta. The first emitted value will still be 0 (or
        // near it) because CPU% is the delta between the last two refreshes.
        sys.refresh_cpu_usage();
        sys.refresh_memory_specifics(MemoryRefreshKind::everything());

        let latest = self.latest.clone();
        let mut cpu_seeded = false;

        tauri::async_runtime::spawn(async move {
            loop {
                let stats = sample(&nvml, &mut sys);

                if !cpu_seeded && stats.cpu_usage_percent > 0.0 {
                    cpu_seeded = true;
                } else if !cpu_seeded {
                    log::info!(
                        "Hardware monitor: first CPU sample seeded; accurate from the next tick"
                    );
                }

                *latest.write().await = stats.clone();
                let _ = app.emit("hardware-stats", &stats);

                tokio::time::sleep(INTERVAL).await;
            }
        });
    }
}

impl Default for HardwareMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Read one telemetry sample. Refreshes CPU+memory on `sys` (mutating it) and
/// reads VRAM from `nvml` if present.
fn sample(nvml: &Option<Nvml>, sys: &mut System) -> HardwareStats {
    // Refresh order: CPU first (so global_cpu_usage reflects the delta since
    // the last refresh_cpu_usage call), then memory.
    sys.refresh_cpu_usage();
    sys.refresh_memory_specifics(MemoryRefreshKind::everything());

    let ram_total_mb = sys.total_memory() / 1024 / 1024;
    let ram_available_mb = sys.available_memory() / 1024 / 1024;
    let ram_used_mb = ram_total_mb.saturating_sub(ram_available_mb);
    let cpu_usage_percent = sys.global_cpu_usage();

    let (vram_used_mb, vram_total_mb) = read_vram(nvml);

    HardwareStats {
        cpu_usage_percent,
        ram_used_mb,
        ram_total_mb,
        vram_used_mb,
        vram_total_mb,
    }
}

/// Read VRAM from the primary GPU. Returns `None` on CPU-only hosts or any
/// NVML read failure. Failures are logged at DEBUG (not every tick at WARN —
/// would spam a transiently-unavailable GPU).
fn read_vram(nvml: &Option<Nvml>) -> (Option<u64>, Option<u64>) {
    let Some(nvml) = nvml else {
        return (None, None);
    };
    let dev = match nvml.device_by_index(0) {
        Ok(d) => d,
        Err(e) => {
            log::debug!("NVML device_by_index failed: {e}");
            return (None, None);
        }
    };
    match dev.memory_info() {
        Ok(info) => (
            Some(info.used / 1024 / 1024),
            Some(info.total / 1024 / 1024),
        ),
        Err(e) => {
            log::debug!("NVML memory_info failed: {e}");
            (None, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sample` must never panic, even with no GPU. We can't assert specific
    /// values (they depend on the host running the test), but the zero-VRAM /
    /// CPU-only path must be exercised.
    #[test]
    fn sample_does_not_panic_without_gpu() {
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing().with_cpu(CpuRefreshKind::everything()),
        );
        sys.refresh_cpu_usage();
        let stats = sample(&None, &mut sys);
        assert_eq!(stats.vram_used_mb, None);
        assert_eq!(stats.vram_total_mb, None);
        assert!(stats.ram_total_mb > 0, "host should report some RAM");
    }
}
