//! Hardware detection — populates the `hardware_profile` singleton row.
//!
//! Two backends:
//!   * `nvml-wrapper` for the NVIDIA GPU (name + total VRAM). Dynamically loads
//!     `libnvidia-ml.so.1` (Linux) or `nvml.dll` (Windows) from the NVIDIA
//!     driver at runtime; fails cleanly to `None` on CPU-only hosts.
//!   * `sysinfo` for total system RAM + CPU physical/logical core counts.
//!
//! ## CPU-only safety valve
//!
//! AGENTS.md: "If the hardware scan detects no usable VRAM, Rust acts as a
//! safety valve. It forces `--n-gpu-layers 0` and explicitly strips all
//! parameter-fitting flags to prevent CUDA initialization panics, allowing a
//! safe default to system RAM."
//!
//! Every GPU scan failure (no driver, NVML init error, device read error,
//! zero VRAM) collapses to `gpu_present = false`. The Process Manager layer
//! will branch on this boolean to assemble the launch command correctly.
//!
//! ## Nothing is hardcoded
//!
//! Every value here comes from a live scan of the actual host. This row is the
//! single source of truth that the VRAM Translation Engine reads to compute
//! `--fit-target` at launch time.

use nvml_wrapper::Nvml;
use sysinfo::System;

use crate::db::HardwareProfileRow;

/// The detected hardware, mirrored 1:1 into the `hardware_profile` row.
///
/// `gpu_present` is the explicit boolean the launch layer branches on; it is
/// derived from the scan (`vram_mb > 0`) and exposed directly so downstream
/// code never has to re-derive it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HardwareProfile {
    /// GPU marketing name, e.g. "NVIDIA GeForce RTX 3050 Laptop GPU".
    /// `"CPU-only (no NVIDIA GPU)"` when no usable GPU was detected.
    pub gpu_name: String,
    /// Total VRAM in MiB. `0` when CPU-only.
    pub total_vram_mb: i64,
    /// Total system RAM in MiB.
    pub total_system_ram_mb: i64,
    /// Physical CPU cores.
    pub cpu_physical_cores: i64,
    /// Logical CPU threads (SMT/hyperthreading count).
    pub cpu_logical_threads: i64,
    /// RFC3339 UTC timestamp of when this scan ran, e.g.
    /// `"2026-07-08T04:21:11Z"`. Lexicographically sortable.
    pub last_scanned_at: String,
    /// `true` when an NVIDIA GPU with >0 VRAM was detected.
    /// `false` arms the CPU-only safety valve.
    pub gpu_present: bool,
}

/// Internal: the GPU facts gathered from NVML.
struct GpuInfo {
    name: String,
    total_vram_bytes: u64,
}

/// Run a full hardware scan.
///
/// GPU scan failures are logged at WARN level and fall through to CPU-only
/// mode rather than propagating — the app must still boot on a box with no GPU.
pub fn scan() -> HardwareProfile {
    let gpu = scan_gpu();
    let (ram_mb, physical, logical) = scan_cpu_ram();

    let (gpu_name, total_vram_mb, gpu_present) = match gpu {
        Some(g) => {
            let vram_mb = (g.total_vram_bytes / (1024 * 1024)) as i64;
            // Treat zero VRAM as CPU-only too — defensive against misreporting.
            let present = vram_mb > 0;
            let name = if present {
                g.name
            } else {
                "CPU-only (GPU reported 0 VRAM)".to_string()
            };
            (name, vram_mb, present)
        }
        None => ("CPU-only (no NVIDIA GPU)".to_string(), 0, false),
    };

    let profile = HardwareProfile {
        gpu_name,
        total_vram_mb,
        total_system_ram_mb: ram_mb,
        cpu_physical_cores: physical,
        cpu_logical_threads: logical,
        last_scanned_at: now_rfc3339_utc(),
        gpu_present,
    };

    if profile.gpu_present {
        log::info!(
            "Hardware scan: {}, {} MiB VRAM, {} MiB RAM, {} phys / {} logical cores",
            profile.gpu_name,
            profile.total_vram_mb,
            profile.total_system_ram_mb,
            profile.cpu_physical_cores,
            profile.cpu_logical_threads
        );
    } else {
        log::warn!(
            "Hardware scan: no NVIDIA GPU detected ({}). Running in CPU-only mode — \
             safety valve armed (--n-gpu-layers 0, --fit/--fit-target will be stripped).",
            profile.gpu_name
        );
    }

    profile
}

/// Convert the scan result into the DB row shape (drops `gpu_present`, which is
/// derived from `total_vram_mb > 0` and needn't be stored).
impl From<&HardwareProfile> for HardwareProfileRow {
    fn from(p: &HardwareProfile) -> Self {
        HardwareProfileRow {
            gpu_name: p.gpu_name.clone(),
            total_vram_mb: p.total_vram_mb,
            total_system_ram_mb: p.total_system_ram_mb,
            cpu_physical_cores: p.cpu_physical_cores,
            cpu_logical_threads: p.cpu_logical_threads,
            last_scanned_at: p.last_scanned_at.clone(),
        }
    }
}

/// Convert a DB row back into HardwareProfile, using the centralized
/// `has_usable_gpu()` method for the `gpu_present` derivation.
impl From<&HardwareProfileRow> for HardwareProfile {
    fn from(row: &HardwareProfileRow) -> Self {
        HardwareProfile {
            gpu_name: row.gpu_name.clone(),
            total_vram_mb: row.total_vram_mb,
            total_system_ram_mb: row.total_system_ram_mb,
            cpu_physical_cores: row.cpu_physical_cores,
            cpu_logical_threads: row.cpu_logical_threads,
            last_scanned_at: row.last_scanned_at.clone(),
            gpu_present: row.has_usable_gpu(),
        }
    }
}

/// Probe the primary NVIDIA GPU via NVML.
///
/// Returns `None` on any failure (no driver, NVML init error, device read
/// error). The caller treats `None` as "CPU-only mode". Each failure is logged
/// at DEBUG so a misconfigured host is diagnosable without spamming normal logs.
fn scan_gpu() -> Option<GpuInfo> {
    let nvml = match Nvml::init() {
        Ok(n) => n,
        Err(e) => {
            log::debug!("NVML init failed — treating host as CPU-only: {e}");
            return None;
        }
    };

    // Report multi-GPU hosts without changing the schema (single primary device).
    match nvml.device_count() {
        Ok(count) if count > 1 => {
            log::info!(
                "Multiple NVIDIA GPUs detected ({count}); using device 0 as primary. \
                 Multi-GPU aggregation is not supported by the MVP."
            );
        }
        Ok(_) => {}
        Err(e) => log::debug!("NVML device_count() failed: {e}"),
    }

    let dev = nvml.device_by_index(0).ok()?;
    let name = match dev.name() {
        Ok(n) => n,
        Err(e) => {
            log::debug!("NVML device.name() failed: {e}");
            return None;
        }
    };
    let total_vram_bytes = match dev.memory_info() {
        Ok(m) => m.total,
        Err(e) => {
            log::debug!("NVML device.memory_info() failed: {e}");
            return None;
        }
    };

    Some(GpuInfo {
        name,
        total_vram_bytes,
    })
}

/// Probe total system RAM + CPU counts via sysinfo.
///
/// `System::new_all()` performs a one-shot refresh that fully populates
/// `total_memory()` and core counts. The well-known "call twice with a sleep"
/// requirement applies only to `cpu_usage()` percentages, which we don't read.
fn scan_cpu_ram() -> (i64, i64, i64) {
    let sys = System::new_all();
    let ram_mb = (sys.total_memory() / 1024 / 1024) as i64;
    let logical = sys.cpus().len() as i64;
    // `physical_core_count` is an associated function in sysinfo 0.39.
    let physical = System::physical_core_count()
        .map(|p| p as i64)
        .unwrap_or(logical);
    (ram_mb, physical, logical)
}

/// Format the current time as RFC3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`).
///
/// Hand-rolled from `std::time` to avoid pulling in `chrono` for a single
/// timestamp field, per the MVP's "no unnecessary deps" rule. Uses the
/// civil-from-days algorithm (Howard Hinnant). Second precision, UTC only.
fn now_rfc3339_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let days = (secs / 86_400) as i64;
    let rem = (secs % 86_400) as i64;
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;

    // Civil-from-days: convert days-since-epoch (1970-01-01) to Y/M/D.
    // https://howardhinnant.github.io/date_algorithms.html
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, m, d, hour, minute, second
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The date math is the only non-trivial logic; sanity-check it produces a
    /// plausible, parseable timestamp near "now".
    #[test]
    fn rfc3339_timestamp_is_well_formed() {
        let ts = now_rfc3339_utc();
        assert_eq!(ts.len(), 20, "expected YYYY-MM-DDTHH:MM:SSZ (20 chars)");
        assert_eq!(ts.as_bytes()[4], b'-');
        assert_eq!(ts.as_bytes()[7], b'-');
        assert_eq!(ts.as_bytes()[10], b'T');
        assert_eq!(ts.as_bytes()[13], b':');
        assert_eq!(ts.as_bytes()[16], b':');
        assert_eq!(ts.as_bytes()[19], b'Z');
        // Year should be the 2020s.
        assert!(ts.starts_with("202"), "year looked wrong: {ts}");
    }

    /// The live scan must never panic — even on a box with no GPU. We can't
    /// assert specific hardware values, but we can assert it returns.
    #[test]
    fn scan_does_not_panic() {
        let _ = scan();
    }
}
