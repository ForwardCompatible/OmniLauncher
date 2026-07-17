//! Tauri command bridge (Frontend → Backend).
//!
//! All user-driven actions invoke `#[tauri::command]`s here. The Svelte
//! frontend never touches the OS directly — it only calls these commands via
//! `@tauri-apps/api`.

pub mod app_settings;
pub mod flags;
pub mod hardware;
pub mod models;
pub mod process;
pub mod proxy;
pub mod registry;

/// Trivial smoke-test command.
#[tauri::command]
pub fn ping() -> String {
    log::info!("ping() called from the frontend");
    "pong".to_string()
}
