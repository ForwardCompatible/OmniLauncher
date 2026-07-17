//! OmniLauncher — Tauri v2 backend entry point.
//!
//! Layer 1 (Foundation) responsibilities:
//!   * Register the logging plugin (stdout + webview + rotating file).
//!   * Initialize the two SQLite databases (System.db, ModelRegistry.db) in WAL mode.
//!   * Bootstrap the on-disk models/multimodal directory layout.
//!   * Expose a minimal command bridge for the smoke-test UI.
//!
//! Layers 2+ (hardware scan, GGUF registry, reverse proxy, process manager,
//! full UI) are deliberately out of scope here; see AGENTS.md.

mod commands;
pub mod db;
mod gguf;
pub mod hardware;
mod logging;
pub mod monitor;
mod paths;
pub mod process;
pub mod proxy;
mod registry;
pub mod sidecar;

use std::sync::Arc;

use tauri::Manager;

use crate::db::DbPools;
use crate::monitor::HardwareMonitor;
use crate::sidecar::SidecarController;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(logging::build_plugin().build())
        .setup(setup)
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::app_settings::get_app_settings,
            commands::app_settings::save_app_settings_cmd,
            commands::flags::get_flag_dictionary,
            commands::hardware::get_hardware_profile,
            commands::hardware::rescan_hardware,
            commands::hardware::get_hardware_stats,
            commands::registry::get_models,
            commands::registry::resync_registry,
            commands::proxy::get_proxy_status,
            commands::proxy::set_routing,
            commands::models::set_model_role,
            commands::models::get_model_settings,
            commands::models::save_model_settings,
            commands::process::launch_model,
            commands::process::stop_model,
            commands::process::get_process_status,
        ])
        .build(tauri::generate_context!())
        .expect("error while building OmniLauncher")
        .run(|app_handle, event| {
            // On app exit, kill all tracked child processes so they "die
            // strictly with the parent application" (AGENTS.md line 185).
            // Uses the synchronous variant — block_on would risk deadlock if
            // the exit handler runs on the tokio worker thread.
            if let tauri::RunEvent::Exit = event {
                if let Some(controller) = app_handle.try_state::<Arc<SidecarController>>() {
                    log::info!("App exiting — shutting down child processes");
                    controller.shutdown_all_blocking();
                }
            }
        });
}

/// One-time application setup. Runs before the first window is shown.
fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("OmniLauncher starting up (Layer 1: Foundation)");

    // 1. Resolve the app-data directory and ensure the models layout exists.
    let data_dir = paths::app_data_dir(app)?;
    let layout = paths::ensure_models_layout(&data_dir)?;
    log::info!("App data directory: {}", data_dir.display());
    log::info!("Models directory: {}", layout.models_dir.display());

    // 2. Open + migrate + seed both databases in WAL mode.
    let pools = Arc::new(DbPools::open(&data_dir)?);
    db::migrate_and_seed(&pools)?;
    log::info!("Databases initialized (System.db, ModelRegistry.db) in WAL mode");

    // 2b. Persist the resolved models/multimodal paths to app_settings so the
    //     Settings page shows them instead of empty fields.
    let paths_update = db::AppSettingsUpdate {
        models_directory: Some(layout.models_dir.to_string_lossy().into_owned()),
        multimodal_directory: Some(layout.multimodal_dir.to_string_lossy().into_owned()),
        master_port: None,
        auto_port_increment: None,
    };
    tauri::async_runtime::block_on(db::save_app_settings(&pools.system, &paths_update))?;

    // 3. Hardware scan — cached on first launch, only re-run on manual rescan.
    //    Check if a previous scan exists (cpu_physical_cores > 0 indicates a
    //    completed scan, regardless of whether the machine has a GPU).
    let cached = tauri::async_runtime::block_on(db::load_hardware_profile(&pools.system))?;
    let has_cache = cached
        .as_ref()
        .map(|r| r.cpu_physical_cores > 0)
        .unwrap_or(false);

    if has_cache {
        let row = cached.unwrap();
        log::info!(
            "Using cached hardware profile (last scanned: {})",
            row.last_scanned_at
        );
    } else {
        log::info!("Running initial hardware scan...");
        let profile = hardware::scan();
        let row = db::HardwareProfileRow::from(&profile);
        tauri::async_runtime::block_on(db::save_hardware_profile(&pools.system, &row))?;
        log::info!("Hardware profile persisted to System.db");
    }

    // 4. Start the live hardware monitor — samples CPU/RAM/VRAM every 2s and
    //    emits `hardware-stats` events to the frontend footer. Constructed
    //    once; the sampler loop owns its Nvml + System handles for the app
    //    lifetime. Mirrors the proxy.rs spawn pattern.
    let monitor = Arc::new(HardwareMonitor::new());
    let monitor_for_task = monitor.clone();
    let app_handle_for_monitor = app.handle().clone();
    monitor_for_task.run(app_handle_for_monitor);
    app.manage(monitor);

    // 5. Sync the model registry: scan `models/`, parse each `.gguf`, upsert
    //    into models_metadata, drop stale rows. Per AGENTS.md "Startup: Registry
    //    syncs with OmniLauncher/models".
    let models_dir = data_dir.join("models");
    let (records, scan_report) = registry::scan_with_report(&models_dir)?;
    let reconcile = tauri::async_runtime::block_on(db::registry_ops::reconcile_models(
        &pools.registry,
        &records,
    ))?;
    log::info!(
        "Registry sync: {} added, {} updated, {} removed, {} DB failure(s), {} unparseable file(s){}",
        reconcile.added,
        reconcile.updated,
        reconcile.removed,
        reconcile.failed,
        scan_report.failed.len(),
        if records.is_empty() && scan_report.failed.is_empty() {
            " (models dir empty — drop a .gguf in ~/.local/share/com.omnilauncher.app/models/)"
        } else {
            ""
        }
    );

    // 6. Start the reverse proxy. Read the master port + auto-increment flag
    //    from app_settings, build the shared routing state (both backends
    //    start None — Layer 5 fills them in), spawn the axum server, and
    //    register the state so commands (and Layer 5) can write routing.
    let settings = tauri::async_runtime::block_on(db::load_full_app_settings(&pools.system))?;
    let proxy_state = proxy::ProxyState::new();
    let proxy_state_for_task = proxy_state.clone();
    let master_port = settings.master_port.max(1) as u16; // 0/unset → 1, resolved below
    let auto_inc = settings.auto_port_increment;

    tauri::async_runtime::spawn(async move {
        if let Err(e) = proxy::serve_with_state(master_port, auto_inc, proxy_state_for_task).await {
            log::error!("Reverse proxy failed to start: {e}");
        }
    });
    app.manage(proxy_state);

    // 7. Sidecar controller — manages all llama-server child processes via a
    //    clean start/stop/status interface. Hides tokio::process internals.
    app.manage(Arc::new(SidecarController::default()));

    // 8. Hand the pools to Tauri's managed state so commands can use them.
    app.manage(pools);

    Ok(())
}
