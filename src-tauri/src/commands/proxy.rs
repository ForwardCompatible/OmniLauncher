//! Reverse proxy commands — expose routing state to the UI.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::db::{load_full_app_settings, DbPools};
use crate::proxy::ProxyState;

#[derive(Debug, Serialize)]
pub struct ProxyStatusDto {
    /// The configured master port from app_settings (the *requested* port —
    /// the actually-bound port may differ if auto-increment fired; see logs).
    pub master_port: i64,
    pub auto_port_increment: bool,
    pub chat_port: Option<u16>,
    pub embedding_port: Option<u16>,
}

/// Read the current proxy status: configured master port, auto-increment flag,
/// and the live chat/embedding backend ports (None = no model running).
#[tauri::command]
pub async fn get_proxy_status(
    pools: State<'_, Arc<DbPools>>,
    proxy_state: State<'_, ProxyState>,
) -> Result<ProxyStatusDto, String> {
    let settings = load_full_app_settings(&pools.system)
        .await
        .map_err(|e| e.to_string())?;
    let routing = proxy_state.routing.read().await;
    Ok(ProxyStatusDto {
        master_port: settings.master_port,
        auto_port_increment: settings.auto_port_increment,
        chat_port: routing.chat_port,
        embedding_port: routing.embedding_port,
    })
}
