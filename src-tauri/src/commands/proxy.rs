//! Reverse proxy commands — expose routing state to the UI, and let the
//! smoke-test (and Layer 5 later) set the chat/embedding backend ports.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
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

/// Which role a routing update targets.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteRole {
    Chat,
    Embedding,
}

#[derive(Debug, Deserialize)]
pub struct SetRoutingArgs {
    pub role: RouteRole,
    /// `Some(port)` routes the role to that backend; `None` clears it (e.g.
    /// when the backing process dies in Layer 5).
    pub port: Option<u16>,
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

/// Set or clear the backend port for a given role.
///
/// In Layer 4 this is a dev/test hook for verifying routing against a stub
/// upstream. Layer 5's process manager will call the same `proxy_state.routing`
/// write path internally when it launches or kills a llama-server process.
#[tauri::command]
pub async fn set_routing(
    proxy_state: State<'_, ProxyState>,
    args: SetRoutingArgs,
) -> Result<(), String> {
    let mut routing = proxy_state.routing.write().await;
    match args.role {
        RouteRole::Chat => {
            log::info!(
                "Proxy routing: chat backend → {}",
                args.port.map(|p| p.to_string()).unwrap_or_else(|| "(none)".into())
            );
            routing.chat_port = args.port;
        }
        RouteRole::Embedding => {
            log::info!(
                "Proxy routing: embedding backend → {}",
                args.port.map(|p| p.to_string()).unwrap_or_else(|| "(none)".into())
            );
            routing.embedding_port = args.port;
        }
    }
    Ok(())
}
