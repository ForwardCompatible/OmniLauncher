//! Database layer.
//!
//! Two SQLite databases live in the app-data directory, both opened in WAL mode
//! via a `deadpool-sqlite` pool (see `pool.rs`):
//!
//!   * `System.db`        — application settings, hardware profile, flag dictionary.
//!   * `ModelRegistry.db` — immutable GGUF model metadata + mutable per-model launch flags.
//!
//! Schemas mirror AGENTS.md exactly. Tables are created idempotently on every
//! startup via `migrate()`; `seed::run()` populates the flag dictionary.

pub mod pool;
pub mod registry_ops;
pub mod registry_schema;
pub mod seed;
pub mod system_schema;

use anyhow::{Context, Result};
use deadpool_sqlite::Pool;
use rusqlite::OptionalExtension;
use serde::Serialize;

pub use pool::DbPools;

/// A flag-dictionary entry (tooltip data for the UI).
#[derive(Debug, Serialize)]
pub struct FlagEntry {
    pub category: String,
    pub flag_name: String,
    pub cli_argument: String,
    pub default_value: Option<String>,
    pub description: String,
}

/// Read all flag_dictionary entries ordered by id.
pub async fn list_flag_dictionary(system_pool: &Pool) -> Result<Vec<FlagEntry>> {
    let conn = system_pool
        .get()
        .await
        .context("Failed to acquire System.db connection")?;
    let entries = conn
        .interact(|c| -> rusqlite::Result<Vec<FlagEntry>> {
            let mut stmt = c.prepare(
                "SELECT category, flag_name, cli_argument, default_value, description
                 FROM flag_dictionary
                 ORDER BY id",
            )?;
            let mapped = stmt.query_map([], |row| {
                Ok(FlagEntry {
                    category: row.get(0)?,
                    flag_name: row.get(1)?,
                    cli_argument: row.get(2)?,
                    default_value: row.get(3)?,
                    description: row.get(4)?,
                })
            })?;
            let mut out = Vec::new();
            for row in mapped {
                out.push(row?);
            }
            Ok(out)
        })
        .await
        .map_err(|e| anyhow::anyhow!("Panic during flag_dictionary load: {e}"))?
        .context("Failed to SELECT flag_dictionary")?;
    Ok(entries)
}

/// Fields we persist into the `hardware_profile` singleton row.
///
/// Kept separate from `hardware::HardwareProfile` so the DB layer doesn't
/// import the hardware module (and so we don't serialize `gpu_present`, which
/// is derived from `total_vram_mb > 0` and needn't be stored).
#[derive(Clone)]
pub struct HardwareProfileRow {
    pub gpu_name: String,
    pub total_vram_mb: i64,
    pub total_system_ram_mb: i64,
    pub cpu_physical_cores: i64,
    pub cpu_logical_threads: i64,
    pub last_scanned_at: String,
}

impl HardwareProfileRow {
    /// The single source of truth for whether a valid CUDA GPU is physically
    /// installed and usable. All command-layer and conversion code must call
    /// this method instead of duplicating the heuristic.
    pub fn has_usable_gpu(&self) -> bool {
        self.total_vram_mb > 0 && !self.gpu_name.starts_with("CPU-only")
    }
}

/// Run idempotent schema creation against both databases, then seed defaults.
///
/// WAL mode and per-connection pragmas are applied by the pool's `post_create`
/// hook, so they hold for every connection — not just the migration ones.
///
/// Called synchronously from `setup()`. The deadpool-sqlite pool's async API is
/// driven here via `tauri::async_runtime::block_on`.
pub fn migrate_and_seed(pools: &DbPools) -> Result<()> {
    tauri::async_runtime::block_on(async {
        apply_schema(&pools.system, "System.db", system_schema::apply).await?;
        apply_schema(&pools.registry, "ModelRegistry.db", registry_schema::apply).await?;
        seed::run(&pools.system).await?;
        Ok(())
    })
}

/// Acquire a pooled connection and run a schema-apply function on it.
///
/// `deadpool-sqlite`'s `interact` requires `FnOnce(&mut Connection)`, but the
/// schema functions only need `&Connection` (they execute DDL, which doesn't
/// require `&mut`). The closure here bridges that: it receives `&mut` and
/// forwards a shared ref.
async fn apply_schema(
    pool: &Pool,
    db_name: &str,
    f: fn(&rusqlite::Connection) -> rusqlite::Result<()>,
) -> Result<()> {
    let conn = pool
        .get()
        .await
        .with_context(|| format!("Failed to acquire connection to {db_name}"))?;
    conn.interact(move |c| f(c))
        .await
        .map_err(|e| anyhow::anyhow!("Panic in {db_name} migration interaction: {e}"))?
        .with_context(|| format!("Schema migration failed for {db_name}"))?;
    Ok(())
}

/// Persist a hardware scan into the `hardware_profile` singleton row (id=1).
/// Overwrites all fields. Called at startup (via `block_on`) and from the
/// async `rescan_hardware` command directly.
pub async fn save_hardware_profile(system_pool: &Pool, row: &HardwareProfileRow) -> Result<()> {
    let row = row.clone();
    let conn = system_pool
        .get()
        .await
        .context("Failed to acquire System.db connection")?;
    conn.interact(move |c| {
        c.execute(
            "UPDATE hardware_profile SET
                gpu_name            = ?1,
                total_vram_mb       = ?2,
                total_system_ram_mb = ?3,
                cpu_physical_cores  = ?4,
                cpu_logical_threads = ?5,
                last_scanned_at     = ?6
             WHERE id = 1",
            (
                &row.gpu_name,
                row.total_vram_mb,
                row.total_system_ram_mb,
                row.cpu_physical_cores,
                row.cpu_logical_threads,
                &row.last_scanned_at,
            ),
        )
    })
    .await
    .map_err(|e| anyhow::anyhow!("Panic during hardware_profile save: {e}"))?
    .context("Failed to UPDATE hardware_profile")?;
    Ok(())
}

/// Full app-settings row. Returned by the `get_app_settings` command and
/// used at startup for proxy port resolution.
#[derive(Debug, Clone)]
pub struct FullAppSettings {
    pub models_directory: Option<String>,
    pub multimodal_directory: Option<String>,
    pub master_port: i64,
    pub auto_port_increment: bool,
    pub theme: String,
}

/// Read the full `app_settings` row (all columns).
pub async fn load_full_app_settings(system_pool: &Pool) -> Result<FullAppSettings> {
    let conn = system_pool
        .get()
        .await
        .context("Failed to acquire System.db connection for app_settings")?;
    let row = conn
        .interact(|c| {
            c.query_row(
                "SELECT models_directory, multimodal_directory, master_port,
                        auto_port_increment, theme
                 FROM app_settings WHERE id = 1",
                [],
                |r| {
                    Ok(FullAppSettings {
                        models_directory: r.get(0)?,
                        multimodal_directory: r.get(1)?,
                        master_port: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                        auto_port_increment: r.get::<_, Option<bool>>(3)?.unwrap_or(true),
                        theme: r.get::<_, Option<String>>(4)?.unwrap_or_else(|| "dark".into()),
                    })
                },
            )
        })
        .await
        .map_err(|e| anyhow::anyhow!("Panic during full app_settings load: {e}"))?
        .context("Failed to SELECT full app_settings")?;
    Ok(row)
}

/// Writable app-settings fields. All optional so partial updates are possible —
/// `None` means "leave unchanged".
#[derive(Debug, Clone)]
pub struct AppSettingsUpdate {
    pub models_directory: Option<String>,
    pub multimodal_directory: Option<String>,
    pub master_port: Option<i64>,
    pub auto_port_increment: Option<bool>,
}

/// Persist app-settings changes to the singleton row. Only non-None fields are
/// updated; the rest keep their existing values.
pub async fn save_app_settings(
    system_pool: &Pool,
    update: &AppSettingsUpdate,
) -> Result<()> {
    let update = update.clone();
    let conn = system_pool
        .get()
        .await
        .context("Failed to acquire System.db connection for app_settings save")?;
    conn.interact(move |c| {
        // Build a dynamic UPDATE that only touches the provided fields.
        let mut sets: Vec<&str> = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(v) = &update.models_directory {
            sets.push("models_directory = ?");
            params.push(Box::new(v.clone()));
        }
        if let Some(v) = &update.multimodal_directory {
            sets.push("multimodal_directory = ?");
            params.push(Box::new(v.clone()));
        }
        if let Some(v) = update.master_port {
            sets.push("master_port = ?");
            params.push(Box::new(v));
        }
        if let Some(v) = update.auto_port_increment {
            sets.push("auto_port_increment = ?");
            params.push(Box::new(v));
        }
        if sets.is_empty() {
            return Ok(0); // nothing to update
        }
        let sql = format!("UPDATE app_settings SET {} WHERE id = 1", sets.join(", "));
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        c.execute(&sql, param_refs.as_slice())
    })
    .await
    .map_err(|e| anyhow::anyhow!("Panic during app_settings save: {e}"))?
    .context("Failed to UPDATE app_settings")?;
    Ok(())
}

/// Read the cached `hardware_profile` singleton row. Returns `None` if the row
/// exists but was never populated (all hardware columns NULL), which only
/// happens before the first successful scan.
pub async fn load_hardware_profile(system_pool: &Pool) -> Result<Option<HardwareProfileRow>> {
    let conn = system_pool.get().await.context("Failed to acquire System.db connection")?;
    let row = conn
        .interact(|c| {
            c.query_row(
                "SELECT gpu_name, total_vram_mb, total_system_ram_mb,
                        cpu_physical_cores, cpu_logical_threads, last_scanned_at
                 FROM hardware_profile WHERE id = 1",
                [],
                |r| {
                    Ok(HardwareProfileRow {
                        gpu_name: r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        total_vram_mb: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                        total_system_ram_mb: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                        cpu_physical_cores: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                        cpu_logical_threads: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                        last_scanned_at: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    })
                },
            )
            .optional()
        })
        .await
        .map_err(|e| anyhow::anyhow!("Panic during hardware_profile load: {e}"))?
        .context("Failed to SELECT hardware_profile")?;
    Ok(row)
}
