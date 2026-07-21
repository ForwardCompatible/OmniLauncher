//! Model registry commands — list models, rescan the directory.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::db::{load_full_app_settings, registry_ops, DbPools};
use crate::registry as registry_scan;

/// A model row plus whether the user has saved per-model launch settings.
#[derive(Debug, Serialize)]
pub struct ModelDto {
    #[serde(flatten)]
    pub summary: registry_ops::ModelSummary,
    pub has_settings: bool,
}

/// Result returned by `resync_registry` for the UI.
#[derive(Debug, Serialize)]
pub struct ResyncReportDto {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub failed: usize,
    pub total: usize,
}

/// List all known models (read from cache; does not rescan).
#[tauri::command]
pub async fn get_models(pools: State<'_, Arc<DbPools>>) -> Result<Vec<ModelDto>, String> {
    let summaries = registry_ops::list_models(&pools.registry)
        .await
        .map_err(|e| e.to_string())?;
    // Batch: one query for all model IDs that have settings (replaces the
    // former N+1 pattern of one query per model).
    let with_settings = registry_ops::models_with_settings(&pools.registry)
        .await
        .map_err(|e| e.to_string())?;

    let out = summaries
        .into_iter()
        .map(|s| {
            let has_settings = with_settings.contains(&s.id);
            ModelDto { summary: s, has_settings }
        })
        .collect();
    Ok(out)
}

/// Re-scan the models directory, reconcile the DB, and broadcast
/// `registry-updated`. Returns the reconcile report.
#[tauri::command]
pub async fn resync_registry(
    app: tauri::AppHandle,
    pools: State<'_, Arc<DbPools>>,
) -> Result<ResyncReportDto, String> {
    // Read the models directory from app_settings via the db layer.
    let app_settings = load_full_app_settings(&pools.system)
        .await
        .map_err(|e| e.to_string())?;
    let models_dir = std::path::PathBuf::from(
        app_settings
            .models_directory
            .ok_or_else(|| "models_directory not set in app_settings".to_string())?,
    );

    let known_files = registry_ops::list_known_file_sizes(&pools.registry)
        .await
        .map_err(|e| e.to_string())?;
    let (records, all_filenames, scan_report) =
        registry_scan::scan_with_report(&models_dir, &known_files)
            .map_err(|e| e.to_string())?;

    let reconcile = registry_ops::reconcile_models(&pools.registry, &records, &all_filenames)
        .await
        .map_err(|e| e.to_string())?;

    log::info!(
        "Registry rescan: {} added, {} updated, {} removed, {} failed ({} files couldn't be parsed)",
        reconcile.added,
        reconcile.updated,
        reconcile.removed,
        reconcile.failed,
        scan_report.failed.len()
    );

    let dto = ResyncReportDto {
        added: reconcile.added,
        updated: reconcile.updated,
        removed: reconcile.removed,
        failed: reconcile.failed,
        total: records.len(),
    };

    use tauri::Emitter;
    let _ = app.emit("registry-updated", &dto);

    Ok(dto)
}
