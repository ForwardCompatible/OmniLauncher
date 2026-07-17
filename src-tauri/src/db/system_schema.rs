//! Schema for `System.db`.
//!
//! Three tables, matching AGENTS.md exactly:
//!   * `app_settings`     — singleton (id=1) of application-level settings.
//!   * `hardware_profile` — singleton (id=1) of detected hardware.
//!   * `flag_dictionary`  — informational tooltip data for llama-server flags.

use rusqlite::{Connection, Result};

/// Apply the System.db schema. Idempotent.
pub fn apply(conn: &Connection) -> Result<()> {
    app_settings_table(conn)?;
    hardware_profile_table(conn)?;
    flag_dictionary_table(conn)?;
    Ok(())
}

/// `app_settings` — singleton table. Row id=1 is created on first migrate and
/// never deleted; updates are always in-place.
fn app_settings_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS app_settings (
            id                    INTEGER PRIMARY KEY,
            models_directory     TEXT,
            multimodal_directory TEXT,
            master_port          INTEGER NOT NULL DEFAULT 0,
            auto_port_increment  BOOLEAN NOT NULL DEFAULT 1,
            theme                TEXT    NOT NULL DEFAULT 'dark'
        );

        -- Seed the singleton row if it doesn't exist.
        -- master_port 52715: a high-numbered port (49152+ dynamic range) that's
        -- unlikely to collide with common dev tools. Auto-increment will bump
        -- it +1 if busy (per AGENTS.md Port Management).
        INSERT OR IGNORE INTO app_settings (
            id, models_directory, multimodal_directory, master_port,
            auto_port_increment, theme
        ) VALUES (
            1, NULL, NULL, 52715, 1, 'dark'
        );",
    )?;
    Ok(())
}

/// `hardware_profile` — singleton table. All hardware columns are NULL until
/// the hardware-scan layer (Layer 2) populates them. Nothing is hardcoded;
/// `--fit-target` reads from this row at launch time.
fn hardware_profile_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS hardware_profile (
            id                  INTEGER PRIMARY KEY,
            gpu_name            TEXT,
            total_vram_mb       INTEGER,
            total_system_ram_mb INTEGER,
            cpu_physical_cores  INTEGER,
            cpu_logical_threads INTEGER,
            last_scanned_at     TIMESTAMP
        );

        INSERT OR IGNORE INTO hardware_profile (id) VALUES (1);",
    )?;
    Ok(())
}

/// `flag_dictionary` — purely informational. Feeds UI hover tooltips and is
/// decoupled from dynamic UI component rendering (per AGENTS.md). Seeded by
/// `seed::run()` from the AGENTS.md flag tables.
fn flag_dictionary_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS flag_dictionary (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            category      TEXT NOT NULL,
            flag_name     TEXT NOT NULL,
            cli_argument  TEXT NOT NULL,
            default_value TEXT,
            description   TEXT NOT NULL
        );",
    )?;
    Ok(())
}
