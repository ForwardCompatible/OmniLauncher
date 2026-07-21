//! Per-model commands: settings load/save.
//!
//! These are the data-side commands the Loader page binds to.
//! The launch/stop commands live in `commands::process`.

use std::sync::Arc;

use tauri::State;

use crate::db::{registry_ops, DbPools};
use crate::hardware::HardwareProfile;
use crate::process::compute_default_settings;

/// Load a model's saved settings, or compute defaults if none saved yet.
///
/// Returns the effective settings the process manager would use at launch —
/// either the user's customized row or the computed defaults. The UI uses this
/// to populate the Loader page's flag controls.
#[tauri::command]
pub async fn get_model_settings(
    pools: State<'_, Arc<DbPools>>,
    model_id: i64,
) -> Result<registry_ops::ModelSettings, String> {
    // First try a saved row.
    if let Some(saved) = registry_ops::load_model_settings(&pools.registry, model_id)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(saved);
    }

    // No saved row: compute defaults from the model metadata + hardware.
    let models = registry_ops::list_models(&pools.registry)
        .await
        .map_err(|e| e.to_string())?;
    let model = models
        .into_iter()
        .find(|m| m.id == model_id)
        .ok_or_else(|| format!("model id {model_id} not found"))?;

    // Read the hardware profile (cached from the last scan).
    let hw_row = crate::db::load_hardware_profile(&pools.system)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| {
            // No hardware scan yet — treat as CPU-only so defaults are safe.
            log::warn!("No hardware profile found; computing CPU-only defaults");
            crate::db::HardwareProfileRow {
                gpu_name: "CPU-only (no scan yet)".into(),
                total_vram_mb: 0,
                total_system_ram_mb: 0,
                cpu_physical_cores: 0,
                cpu_logical_threads: 0,
                last_scanned_at: String::new(),
            }
        });

    let hw = HardwareProfile::from(&hw_row);

    Ok(compute_default_settings(&model, &hw))
}

/// Save a model's settings (the Loader page Save button).
#[tauri::command]
pub async fn save_model_settings(
    pools: State<'_, Arc<DbPools>>,
    model_id: i64,
    settings: registry_ops::ModelSettings,
) -> Result<(), String> {
    registry_ops::save_model_settings(&pools.registry, model_id, &settings)
        .await
        .map_err(|e| e.to_string())
}
