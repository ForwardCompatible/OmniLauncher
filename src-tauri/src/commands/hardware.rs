//! `hardware_profile` commands — read the cached row, or rescan live.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::db::{load_hardware_profile, DbPools, HardwareProfileRow};

/// The shape the frontend receives. Mirrors the DB row plus the derived
/// `gpu_present` boolean the launch logic will branch on.
#[derive(Debug, Serialize)]
pub struct HardwareProfileDto {
    pub gpu_name: String,
    pub total_vram_mb: i64,
    pub total_system_ram_mb: i64,
    pub cpu_physical_cores: i64,
    pub cpu_logical_threads: i64,
    pub last_scanned_at: String,
    /// `true` when an NVIDIA GPU with >0 VRAM was detected. Drives the
    /// CPU-only safety valve downstream.
    pub gpu_present: bool,
}

impl From<HardwareProfileRow> for HardwareProfileDto {
    fn from(r: HardwareProfileRow) -> Self {
        let gpu_present = r.has_usable_gpu();
        HardwareProfileDto {
            gpu_name: r.gpu_name,
            total_vram_mb: r.total_vram_mb,
            total_system_ram_mb: r.total_system_ram_mb,
            cpu_physical_cores: r.cpu_physical_cores,
            cpu_logical_threads: r.cpu_logical_threads,
            last_scanned_at: r.last_scanned_at,
            gpu_present,
        }
    }
}

/// Read the cached hardware profile from System.db. Cheap; does not rescan.
#[tauri::command]
pub async fn get_hardware_profile(
    pools: State<'_, Arc<DbPools>>,
) -> Result<HardwareProfileDto, String> {
    let row = load_hardware_profile(&pools.system)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "hardware_profile row not populated yet".to_string())?;
    Ok(HardwareProfileDto::from(row))
}

/// Run a fresh hardware scan, persist it, and broadcast a `hardware-updated`
/// event so any open UI surfaces can refresh. Returns the new profile.
#[tauri::command]
pub async fn rescan_hardware(
    app: tauri::AppHandle,
    pools: State<'_, Arc<DbPools>>,
) -> Result<HardwareProfileDto, String> {
    let profile = crate::hardware::scan();
    let row = HardwareProfileRow::from(&profile);
    crate::db::save_hardware_profile(&pools.system, &row)
        .await
        .map_err(|e| e.to_string())?;

    let dto = HardwareProfileDto::from(row);

    // Broadcast to all windows (per AGENTS.md "Backend → Frontend" event pattern).
    use tauri::Emitter;
    app.emit("hardware-updated", &dto)
        .map_err(|e| e.to_string())?;

    Ok(dto)
}
