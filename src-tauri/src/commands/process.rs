//! Process-launch commands — thin wrappers over the SidecarController.
//! No tokio types, no binary path resolution, no process internals leak here.

use std::sync::Arc;

use serde::Deserialize;
use tauri::State;

use crate::db::registry_ops;
use crate::db::{load_hardware_profile, DbPools};
use crate::hardware::HardwareProfile;
use crate::process::{self, compute_default_settings, Role};
use crate::proxy::ProxyState;
use crate::sidecar::SidecarController;

#[derive(Debug, Deserialize)]
pub struct LaunchArgs {
    pub model_id: i64,
    /// "chat" or "embedding" — which proxy route to bind this backend to.
    pub role: String,
}

/// Launch a model as a chat or embedding backend.
#[tauri::command]
pub async fn launch_model(
    app: tauri::AppHandle,
    pools: State<'_, Arc<DbPools>>,
    controller: State<'_, Arc<SidecarController>>,
    proxy_state: State<'_, ProxyState>,
    args: LaunchArgs,
) -> Result<process::LaunchReport, String> {
    let role = Role::from_db_str(&args.role)
        .ok_or_else(|| format!("invalid role '{}': expected 'chat' or 'embedding'", args.role))?;

    // Load model metadata.
    let models = registry_ops::list_models(&pools.registry)
        .await
        .map_err(|e| e.to_string())?;
    let model = models
        .into_iter()
        .find(|m| m.id == args.model_id)
        .ok_or_else(|| format!("model id {} not found", args.model_id))?;

    // Load settings (saved row or computed defaults).
    let settings = match registry_ops::load_model_settings(&pools.registry, args.model_id)
        .await
        .map_err(|e| e.to_string())?
    {
        Some(s) => s,
        None => {
            let hw = load_hardware(&pools).await?;
            compute_default_settings(&model, &hw)
        }
    };

    // Load hardware for the VRAM Translation Engine.
    let hw = load_hardware(&pools).await?;

    // Build the CLI args via the VRAM Translation Engine.
    let cli_args = process::build_args(&model.filepath, &settings, &hw, role);

    // Start via the encapsulated sidecar controller.
    controller
        .start(&app, proxy_state.inner(), args.model_id, &model.model_name, role, cli_args)
        .await
        .map_err(|e| e.to_string())
}

/// Stop a running model's backend process.
#[tauri::command]
pub async fn stop_model(
    controller: State<'_, Arc<SidecarController>>,
    proxy_state: State<'_, ProxyState>,
    model_id: i64,
) -> Result<(), String> {
    controller
        .stop(proxy_state.inner(), model_id)
        .await
        .map_err(|e| e.to_string())
}

/// List currently-running backends.
#[tauri::command]
pub async fn get_process_status(
    controller: State<'_, Arc<SidecarController>>,
) -> Result<Vec<crate::sidecar::ProcessInfo>, String> {
    Ok(controller.status().await)
}

/// Load the hardware profile into the HardwareProfile struct used by build_args.
async fn load_hardware(pools: &Arc<DbPools>) -> Result<HardwareProfile, String> {
    let row = load_hardware_profile(&pools.system)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no hardware profile — run a hardware scan first".to_string())?;
    Ok(HardwareProfile::from(&row))
}
