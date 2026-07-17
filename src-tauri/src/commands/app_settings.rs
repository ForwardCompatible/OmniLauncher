//! `app_settings` commands — read and write the singleton settings row.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::{save_app_settings, AppSettingsUpdate, DbPools};

/// The full app settings row, serialized to the frontend.
#[derive(Debug, Serialize)]
pub struct AppSettings {
    pub models_directory: Option<String>,
    pub multimodal_directory: Option<String>,
    pub master_port: i64,
    pub auto_port_increment: bool,
    pub theme: String,
}

/// The writable subset the Settings page POSTs back.
#[derive(Debug, Deserialize)]
pub struct SaveAppSettingsArgs {
    pub models_directory: Option<String>,
    pub multimodal_directory: Option<String>,
    pub master_port: Option<i64>,
    pub auto_port_increment: Option<bool>,
}

/// Read the full app settings row (all columns).
#[tauri::command]
pub async fn get_app_settings(pools: State<'_, Arc<DbPools>>) -> Result<AppSettings, String> {
    let row = crate::db::load_full_app_settings(&pools.system)
        .await
        .map_err(|e| e.to_string())?;
    Ok(AppSettings {
        models_directory: row.models_directory,
        multimodal_directory: row.multimodal_directory,
        master_port: row.master_port,
        auto_port_increment: row.auto_port_increment,
        theme: row.theme,
    })
}

/// Persist changed app settings (partial update — only non-None fields written).
#[tauri::command]
pub async fn save_app_settings_cmd(
    pools: State<'_, Arc<DbPools>>,
    args: SaveAppSettingsArgs,
) -> Result<(), String> {
    let update = AppSettingsUpdate {
        models_directory: args.models_directory,
        multimodal_directory: args.multimodal_directory,
        master_port: args.master_port,
        auto_port_increment: args.auto_port_increment,
    };
    save_app_settings(&pools.system, &update)
        .await
        .map_err(|e| e.to_string())
}
