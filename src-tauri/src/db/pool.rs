//! Connection pooling for the two SQLite databases.
//!
//! We use `deadpool-sqlite`, which wraps each `rusqlite::Connection` in a
//! `SyncWrapper` and runs closures on a dedicated blocking thread via
//! `.interact()`. This is the correct pattern for rusqlite (whose
//! `Connection` is `!Sync`) under Tauri's async runtime.
//!
//! WAL mode and the other pragmas are set in the pool's `post_create` hook so
//! they apply to *every* pooled connection, not just the first one. Per the
//! SQLite docs, `journal_mode=WAL` is persistent on the file but reasserting it
//! is cheap and harmless; `foreign_keys`, `synchronous`, and `busy_timeout` are
//! per-connection and MUST be set here.

use std::path::Path;

use anyhow::{Context, Result};
use deadpool_sqlite::{Config, Hook, Pool, Runtime};

/// The two database pools, held in Tauri managed state.
pub struct DbPools {
    /// `System.db` — app settings, hardware profile, flag dictionary.
    pub system: Pool,
    /// `ModelRegistry.db` — GGUF metadata + per-model launch flags.
    pub registry: Pool,
}

impl DbPools {
    /// Open both pools against `data_dir`. The DB files are created on first
    /// connect if they don't already exist.
    pub fn open(data_dir: &Path) -> Result<Self> {
        let system = open_pool(&data_dir.join("System.db"))?;
        let registry = open_pool(&data_dir.join("ModelRegistry.db"))?;
        Ok(Self { system, registry })
    }
}

/// Build a deadpool pool for a single DB file, with WAL pragmas baked in.
fn open_pool(path: &Path) -> Result<Pool> {
    let cfg = Config::new(path);

    let pool = cfg
        .builder(Runtime::Tokio1)
        .with_context(|| format!("Failed to build pool for {}", path.display()))?
        .max_size(8)
        .post_create(Hook::async_fn(|conn, _metrics| {
            Box::pin(async move {
                // `conn` is a `SyncWrapper<rusqlite::Connection>`; run the
                // pragma setup on its blocking thread via `interact`.
                conn.interact(apply_pragmas)
                    .await
                    .map_err(|e| deadpool_sqlite::HookError::message(e.to_string()))?
                    .map_err(deadpool_sqlite::HookError::Backend)?;
                Ok(())
            })
        }))
        .build()
        .with_context(|| format!("Failed to create pool for {}", path.display()))?;

    Ok(pool)
}

/// Apply per-connection pragmas. Runs on the connection's blocking thread.
///
/// - `foreign_keys` is per-connection and OFF by default — must set here.
/// - `journal_mode=WAL` is persistent on the file; reasserted per-conn safely.
/// - `synchronous=NORMAL` is the recommended setting for WAL.
/// - `busy_timeout` makes writers briefly block instead of erroring on lock
///   contention.
fn apply_pragmas(conn: &mut rusqlite::Connection) -> Result<(), rusqlite::Error> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    // 10s — NTFS file locking is more aggressive than Linux fcntl under WAL.
    // This is the SQLite team's recommendation for Windows; harmless on Linux.
    conn.pragma_update(None, "busy_timeout", 10000)?;
    Ok(())
}
