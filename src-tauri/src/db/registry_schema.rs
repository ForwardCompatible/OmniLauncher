//! Schema for `ModelRegistry.db`.
//!
//! Two tables, matching AGENTS.md exactly:
//!   * `models_metadata` — immutable GGUF header data (one row per .gguf file).
//!   * `model_settings`  — mutable per-model launch flags (one row per model).

use rusqlite::{Connection, Result};

/// Apply the ModelRegistry.db schema. Idempotent.
pub fn apply(conn: &Connection) -> Result<()> {
    models_metadata_table(conn)?;
    model_settings_table(conn)?;
    Ok(())
}

/// `models_metadata` — one row per .gguf file discovered under `models/`.
/// Populated by the GGUF parser + registry sync layer (Layer 3).
///
/// `role` tags a model as `'chat'` or `'embedding'` so the process manager
/// knows which proxy route to bind it to. NULL = untagged (appears in both
/// dropdowns). Auto-set on first insert based on `pooling_type` + `chat_template`.
///
/// `pooling_type` is the raw `{arch}.pooling_type` from the GGUF header —
/// present on embedding models, absent on chat models.
fn models_metadata_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS models_metadata (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            filename       TEXT NOT NULL UNIQUE,
            filepath       TEXT NOT NULL UNIQUE,
            filesize_bytes INTEGER NOT NULL,
            architecture   TEXT NOT NULL,
            model_name     TEXT NOT NULL,
            context_length INTEGER NOT NULL,
            layer_count    INTEGER NOT NULL,
            quantization   TEXT NOT NULL,
            chat_template  TEXT,
            author         TEXT,
            role           TEXT,
            pooling_type   TEXT
        );",
    )?;
    // Idempotent migrations for pre-existing DBs.
    add_column_if_absent(conn, "models_metadata", "role", "TEXT")?;
    add_column_if_absent(conn, "models_metadata", "pooling_type", "TEXT")?;
    Ok(())
}

/// `model_settings` — mutable per-model launch flags. Bound directly to the
/// Loader page UI components. `model_id` FK cascades on delete.
///
/// Layer 6.1 additions: `cpu_mode`, `mlock`, `no_mmap`, `threads`,
/// `threads_batch` — exposed in the expanded Advanced section.
fn model_settings_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS model_settings (
            id                  INTEGER PRIMARY KEY AUTOINCREMENT,
            model_id            INTEGER NOT NULL UNIQUE,
            vram_allocation_mb  INTEGER,
            ctx_size            INTEGER,
            batch_size          INTEGER,
            ubatch_size         INTEGER,
            flash_attn          BOOLEAN,
            cache_type_k        TEXT,
            cache_type_v        TEXT,
            cpu_mode            BOOLEAN DEFAULT 0,
            mlock               BOOLEAN DEFAULT 0,
            no_mmap             BOOLEAN DEFAULT 0,
            threads             INTEGER,
            threads_batch       INTEGER,
            pooling_type_override TEXT,
            embd_normalize      INTEGER,
            rerank              BOOLEAN DEFAULT 0,
            -- Sampling params (chat-only; null = auto/omit)
            temp                REAL,
            top_k               INTEGER,
            top_p               REAL,
            min_p               REAL,
            repeat_penalty      REAL,
            repeat_last_n       INTEGER,
            seed                INTEGER,
            presence_penalty    REAL,
            frequency_penalty   REAL,
            typical_p           REAL,
            xtc_probability     REAL,
            xtc_threshold       REAL,
            mirostat            INTEGER,
            mirostat_lr         REAL,
            mirostat_ent        REAL,
            dry_multiplier      REAL,
            dry_base            REAL,
            dry_allowed_length  INTEGER,
            -- Context & server
            predict             INTEGER,
            context_shift       BOOLEAN,
            parallel            INTEGER,
            cont_batching       BOOLEAN,
            cache_prompt        BOOLEAN,
            timeout             INTEGER,
            -- RoPE
            rope_scaling        TEXT,
            rope_freq_base      REAL,
            -- Reasoning
            reasoning_format    TEXT,
            FOREIGN KEY (model_id) REFERENCES models_metadata(id) ON DELETE CASCADE
        );

        CREATE INDEX IF NOT EXISTS idx_model_settings_model_id
            ON model_settings(model_id);",
    )?;
    // Migrations for pre-existing DBs.
    add_column_if_absent(conn, "model_settings", "cpu_mode", "BOOLEAN DEFAULT 0")?;
    add_column_if_absent(conn, "model_settings", "mlock", "BOOLEAN DEFAULT 0")?;
    add_column_if_absent(conn, "model_settings", "no_mmap", "BOOLEAN DEFAULT 0")?;
    add_column_if_absent(conn, "model_settings", "threads", "INTEGER")?;
    add_column_if_absent(conn, "model_settings", "threads_batch", "INTEGER")?;
    add_column_if_absent(conn, "model_settings", "pooling_type_override", "TEXT")?;
    add_column_if_absent(conn, "model_settings", "embd_normalize", "INTEGER")?;
    add_column_if_absent(conn, "model_settings", "rerank", "BOOLEAN DEFAULT 0")?;
    // Sampling params
    add_column_if_absent(conn, "model_settings", "temp", "REAL")?;
    add_column_if_absent(conn, "model_settings", "top_k", "INTEGER")?;
    add_column_if_absent(conn, "model_settings", "top_p", "REAL")?;
    add_column_if_absent(conn, "model_settings", "min_p", "REAL")?;
    add_column_if_absent(conn, "model_settings", "repeat_penalty", "REAL")?;
    add_column_if_absent(conn, "model_settings", "repeat_last_n", "INTEGER")?;
    add_column_if_absent(conn, "model_settings", "seed", "INTEGER")?;
    add_column_if_absent(conn, "model_settings", "presence_penalty", "REAL")?;
    add_column_if_absent(conn, "model_settings", "frequency_penalty", "REAL")?;
    add_column_if_absent(conn, "model_settings", "typical_p", "REAL")?;
    add_column_if_absent(conn, "model_settings", "xtc_probability", "REAL")?;
    add_column_if_absent(conn, "model_settings", "xtc_threshold", "REAL")?;
    add_column_if_absent(conn, "model_settings", "mirostat", "INTEGER")?;
    add_column_if_absent(conn, "model_settings", "mirostat_lr", "REAL")?;
    add_column_if_absent(conn, "model_settings", "mirostat_ent", "REAL")?;
    add_column_if_absent(conn, "model_settings", "dry_multiplier", "REAL")?;
    add_column_if_absent(conn, "model_settings", "dry_base", "REAL")?;
    add_column_if_absent(conn, "model_settings", "dry_allowed_length", "INTEGER")?;
    // Context & server
    add_column_if_absent(conn, "model_settings", "predict", "INTEGER")?;
    add_column_if_absent(conn, "model_settings", "context_shift", "BOOLEAN")?;
    add_column_if_absent(conn, "model_settings", "parallel", "INTEGER")?;
    add_column_if_absent(conn, "model_settings", "cont_batching", "BOOLEAN")?;
    add_column_if_absent(conn, "model_settings", "cache_prompt", "BOOLEAN")?;
    add_column_if_absent(conn, "model_settings", "timeout", "INTEGER")?;
    // RoPE
    add_column_if_absent(conn, "model_settings", "rope_scaling", "TEXT")?;
    add_column_if_absent(conn, "model_settings", "rope_freq_base", "REAL")?;
    // Reasoning
    add_column_if_absent(conn, "model_settings", "reasoning_format", "TEXT")?;
    Ok(())
}

/// Idempotently add a column to a table if it doesn't already exist.
/// SQLite has no `ADD COLUMN IF NOT EXISTS`, so we check PRAGMA table_info.
fn add_column_if_absent(
    conn: &Connection,
    table: &str,
    column: &str,
    type_def: &str,
) -> Result<()> {
    let has_col = {
        let sql = format!("PRAGMA table_info({table})");
        let mut stmt = conn.prepare(&sql)?;
        let mut found = false;
        let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
        for name in rows {
            if name? == column {
                found = true;
            }
        }
        found
    };
    if !has_col {
        let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {type_def}");
        conn.execute(&sql, [])?;
    }
    Ok(())
}
