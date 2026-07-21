//! Tauri commands for the Models page: HuggingFace OAuth, search, and downloads.
//!
//! 10 commands split into auth (5) and hub operations (5). Auth uses the
//! device-code flow from [`crate::hf_auth`]; the frontend drives the poll loop.

use std::sync::Arc;

use serde::Serialize;
use tauri::{Emitter, State};
use tokio::sync::watch;

use crate::db::{self, DbPools};
use crate::download_manager::{DownloadId, SharedDownloadManager};
use crate::hf_auth::{self, DeviceAuthInfo, HfCredentials, PollOutcome};
use crate::huggingface::{HfClient, HfFile, HfRepoGgufMeta, HfSearchPage};
use crate::registry::ModelRecord;

// ════════════════════════════════════════════════════════════════
//  AUTH (device-code flow)
// ════════════════════════════════════════════════════════════════

/// Begin the device-code flow. Returns the code + URL for the user to visit.
/// The frontend opens `verification_uri` in the system browser via `window.open`
/// (Tauri routes external URLs to the OS default browser); the backend does not
/// open the browser itself, avoiding a runtime-only trait-method dependency.
#[tauri::command]
pub async fn hf_auth_start(hf: State<'_, Arc<HfClient>>) -> Result<DeviceAuthInfo, String> {
    let client = hf.client_clone().await;
    hf_auth::request_device_code(&client)
        .await
        .map_err(|e| e.to_string())
}

/// Poll once for the token. The frontend calls this every `POLL_INTERVAL`
/// seconds until it gets a terminal outcome (`granted` / `expired` / `denied`).
#[tauri::command]
pub async fn hf_auth_poll(
    hf: State<'_, Arc<HfClient>>,
    device_code: String,
) -> Result<PollOutcome, String> {
    let client = hf.client_clone().await;
    let outcome = hf_auth::poll_for_token(&client, &device_code)
        .await
        .map_err(|e| e.to_string())?;
    // On granted, refresh the hub client so it picks up the new token.
    if matches!(outcome, PollOutcome::Granted { .. }) {
        hf.refresh_token().await;
    }
    Ok(outcome)
}

/// Abort an in-flight device-code session (user closed the dialog). The
/// device_code simply expires server-side after 5 min; nothing to cancel
/// locally except the frontend's poll loop. Provided for API symmetry.
#[tauri::command]
pub async fn hf_auth_cancel() -> Result<(), String> {
    Ok(())
}

/// Current auth status — drives the "Connected as X" / "Sign in" badge.
#[derive(Serialize)]
pub struct HfAuthStatus {
    pub connected: bool,
    pub username: Option<String>,
    /// Unix epoch seconds when the token expires, if known. The frontend uses
    /// this to show a "reconnect" prompt before expiry.
    pub expires_at: Option<u64>,
    pub keychain_unavailable: bool,
}

#[tauri::command]
pub async fn hf_auth_status() -> Result<HfAuthStatus, String> {
    if !HfCredentials::is_available() {
        return Ok(HfAuthStatus {
            connected: false,
            username: None,
            expires_at: None,
            keychain_unavailable: true,
        });
    }
    let token = HfCredentials::load_token();
    let username = HfCredentials::load_username();
    Ok(HfAuthStatus {
        connected: token.is_some(),
        username,
        // We don't persist expires_at in the keychain (it'd need a second
        // entry); the frontend treats expiry as "best effort" — if a request
        // 401s, it triggers re-auth. Returning None here is honest.
        expires_at: None,
        keychain_unavailable: false,
    })
}

/// Clear the stored token + username (sign out).
#[tauri::command]
pub async fn hf_auth_logout(hf: State<'_, Arc<HfClient>>) -> Result<(), String> {
    HfCredentials::clear_token().map_err(|e| e.to_string())?;
    HfCredentials::clear_username().map_err(|e| e.to_string())?;
    hf.refresh_token().await;
    log::info!("HuggingFace signed out");
    Ok(())
}

// ════════════════════════════════════════════════════════════════
//  HUB OPERATIONS (search, list, download)
// ════════════════════════════════════════════════════════════════

#[tauri::command]
pub async fn hf_search(
    hf: State<'_, Arc<HfClient>>,
    query: String,
    sort: Option<String>,
    pipeline_tag: Option<String>,
    gguf_only: Option<bool>,
    cursor: Option<String>,
) -> Result<HfSearchPage, String> {
    let sort = sort.as_deref().unwrap_or("downloads");
    let gguf = gguf_only.unwrap_or(true);
    hf.search(
        &query,
        sort,
        pipeline_tag.as_deref(),
        gguf,
        cursor.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())
}

/// Return type for hf_list_files — the files plus repo-level GGUF metadata.
#[derive(Serialize)]
pub struct HfFilesResponse {
    pub files: Vec<HfFile>,
    #[serde(flatten)]
    pub meta: HfRepoGgufMeta,
}

#[tauri::command]
pub async fn hf_list_files(
    hf: State<'_, Arc<HfClient>>,
    repo_id: String,
) -> Result<HfFilesResponse, String> {
    let (files, meta) = hf.list_files(&repo_id).await.map_err(|e| e.to_string())?;
    Ok(HfFilesResponse { files, meta })
}

/// Fetch one file's size via HEAD (lazy, on repo expand).
#[tauri::command]
pub async fn hf_file_size(
    hf: State<'_, Arc<HfClient>>,
    repo_id: String,
    filename: String,
) -> Result<u64, String> {
    hf.head_file_size(&repo_id, &filename)
        .await
        .map_err(|e| e.to_string())
}

/// Fetch a repo's README (model card) as raw markdown. Lazily called when the
/// user clicks "Model Info" — never fetched during search.
#[tauri::command]
pub async fn hf_readme(
    hf: State<'_, Arc<HfClient>>,
    repo_id: String,
) -> Result<String, String> {
    hf.fetch_readme(&repo_id).await.map_err(|e| e.to_string())
}

// ── Download ──

#[derive(Serialize, Clone)]
struct DownloadCompletedPayload {
    id: DownloadId,
    repo_id: String,
    filename: String,
    model_name: String,
}

#[derive(Serialize, Clone)]
struct DownloadFailedPayload {
    id: DownloadId,
    repo_id: String,
    filename: String,
    message: String,
}

/// Start downloading a file. Returns a download id immediately; progress and
/// completion flow via `download-progress` / `download-completed` /
/// `download-failed` events.
#[tauri::command]
pub async fn hf_download(
    app: tauri::AppHandle,
    hf: State<'_, Arc<HfClient>>,
    pools: State<'_, Arc<DbPools>>,
    dm: State<'_, SharedDownloadManager>,
    repo_id: String,
    filename: String,
) -> Result<DownloadId, String> {
    let settings = db::load_full_app_settings(&pools.system)
        .await
        .map_err(|e| e.to_string())?;
    let models_dir = std::path::PathBuf::from(
        settings
            .models_directory
            .ok_or_else(|| "models_directory is not set".to_string())?,
    );

    let final_path = models_dir.join(&filename);
    if final_path.exists() {
        return Err(format!("{filename} already exists in the models directory"));
    }
    let part_path = models_dir.join(format!("{filename}.part"));

    let (id, mut rx) = dm.register().await;

    let hf_clone = Arc::clone(&hf);
    let pools_clone = Arc::clone(&pools);
    let dm_clone = Arc::clone(&dm);
    let app_clone = app.clone();
    let repo = repo_id.clone();
    let file = filename.clone();

    tauri::async_runtime::spawn(async move {
        let result = run_download(
            &hf_clone, &pools_clone, &app_clone, id, &repo, &file, &part_path, &final_path, &mut rx,
        )
        .await;
        match &result {
            Ok(model_name) => {
                let _ = app_clone.emit(
                    "download-completed",
                    DownloadCompletedPayload {
                        id,
                        repo_id: repo.clone(),
                        filename: file.clone(),
                        model_name: model_name.clone(),
                    },
                );
                log::info!("Download {id} completed: {file}");
            }
            Err(e) => {
                let _ = app_clone.emit(
                    "download-failed",
                    DownloadFailedPayload {
                        id,
                        repo_id: repo.clone(),
                        filename: file.clone(),
                        message: e.to_string(),
                    },
                );
                log::warn!("Download {id} failed: {e}");
            }
        }
        dm_clone.finish(id).await;
    });

    Ok(id)
}

/// The download → validate → register pipeline. Returns the model_name on success.
async fn run_download(
    hf: &HfClient,
    pools: &DbPools,
    app: &tauri::AppHandle,
    id: DownloadId,
    repo_id: &str,
    filename: &str,
    part_path: &std::path::Path,
    final_path: &std::path::Path,
    cancel_rx: &mut watch::Receiver<bool>,
) -> anyhow::Result<String> {
    let app_for_progress = app.clone();
    let id_for_progress = id;

    let bytes = hf
        .download(
            repo_id,
            filename,
            part_path,
            |downloaded, total| {
                let _ = app_for_progress.emit(
                    "download-progress",
                    serde_json::json!({
                        "id": id_for_progress,
                        "downloaded_bytes": downloaded,
                        "total_bytes": total,
                    }),
                );
            },
            || *cancel_rx.borrow(),
        )
        .await?;

    // Validate before promoting: must parse as real GGUF.
    if let Err(e) = crate::gguf::parse(part_path) {
        let _ = tokio::fs::remove_file(part_path).await;
        return Err(e)
            .context(format!("Downloaded {filename} is not a valid GGUF ({bytes} bytes)"));
    }
    tokio::fs::rename(part_path, final_path)
        .await
        .with_context(|| format!("Could not move {filename} into place"))?;

    // Parse validated file, upsert one registry row, notify the Loader page.
    let metadata = crate::gguf::parse(final_path)?;
    let filesize_bytes = std::fs::metadata(final_path)?.len() as i64;
    let record = ModelRecord {
        filename: filename.to_string(),
        filepath: final_path.to_path_buf(),
        filesize_bytes,
        metadata,
    };
    let model_name = record.metadata.model_name.clone();
    db::registry_ops::upsert_model(&pools.registry, &record).await?;
    let _ = app.emit("registry-updated", ());
    Ok(model_name)
}

/// Cancel an in-flight download by id.
#[tauri::command]
pub async fn hf_cancel_download(
    dm: State<'_, SharedDownloadManager>,
    id: DownloadId,
) -> Result<bool, String> {
    Ok(dm.cancel(id).await)
}

use anyhow::Context as _;
