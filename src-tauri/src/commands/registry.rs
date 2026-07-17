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

    let mut out = Vec::with_capacity(summaries.len());
    for s in summaries {
        let id = s.id;
        let has_settings = registry_ops::model_has_settings(&pools.registry, id)
            .await
            .map_err(|e| e.to_string())?;
        out.push(ModelDto {
            summary: s,
            has_settings,
        });
    }
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

    let (records, scan_report) = registry_scan::scan_with_report(&models_dir)
        .map_err(|e| e.to_string())?;

    let reconcile = registry_ops::reconcile_models(&pools.registry, &records)
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
