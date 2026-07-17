//! `flag_dictionary` command — thin wrapper over the db layer.

use std::sync::Arc;

use tauri::State;

use crate::db::{list_flag_dictionary, DbPools, FlagEntry};

#[tauri::command]
pub async fn get_flag_dictionary(
    pools: State<'_, Arc<DbPools>>,
) -> Result<Vec<FlagEntry>, String> {
    list_flag_dictionary(&pools.system)
        .await
        .map_err(|e| e.to_string())
}
