//! Database operations for the model registry (`ModelRegistry.db`).
//!
//! - `upsert_model`: insert-or-replace a parsed model into `models_metadata`.
//! - `delete_model_by_filename`: remove a stale row (cascades to
//!   `model_settings` via the FK set in Layer 1).
//! - `list_models`: read all rows for the UI.
//! - `reconcile_models`: full sync — upsert a new set, delete rows whose files
//!   are no longer on disk.

use anyhow::{Context, Result};
use deadpool_sqlite::Pool;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::registry::ModelRecord;

/// Lightweight view of a model row, for the UI / command layer.
#[derive(Debug, Serialize)]
pub struct ModelSummary {
    pub id: i64,
    pub filename: String,
    pub filepath: String,
    pub filesize_bytes: i64,
    pub architecture: String,
    pub model_name: String,
    pub context_length: i64,
    pub layer_count: i64,
    pub quantization: String,
    pub chat_template: Option<String>,
    pub author: Option<String>,
    /// `'chat'`, `'embedding'`, or NULL (untagged). Auto-detected on insert
    /// from pooling_type + chat_template; preserved across rescans.
    pub role: Option<String>,
    /// Raw `{arch}.pooling_type` from GGUF — present on embedding models.
    pub pooling_type: Option<String>,
}

/// Result of a reconcile operation — surfaced to the UI and log.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReconcileReport {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub failed: usize,
}

/// Insert-or-replace a single model. Returns the row id.
///
/// On INSERT (new model): `role` is auto-detected from the GGUF metadata:
///   - `pooling_type` present → `'embedding'`
///   - No `pooling_type` but `chat_template` present → `'chat'`
///   - Neither → `NULL` (appears in both dropdowns)
///
/// On CONFLICT (existing model, registry rescan): `role` is NOT overwritten —
/// the user's tag is preserved. `pooling_type` IS updated (it's immutable
/// header data, not a user preference).
pub async fn upsert_model(pool: &Pool, rec: &ModelRecord) -> Result<i64> {
    let rec = rec.clone();
    let conn = pool
        .get()
        .await
        .context("Failed to acquire ModelRegistry.db connection")?;

    // Auto-detect role from the parsed metadata.
    let auto_role: Option<&str> = if rec.metadata.pooling_type.is_some() {
        Some("embedding")
    } else if rec.metadata.chat_template.is_some() {
        Some("chat")
    } else {
        None
    };

    let id = conn
        .interact(move |c| {
            c.query_row(
                "INSERT INTO models_metadata (
                    filename, filepath, filesize_bytes,
                    architecture, model_name, context_length, layer_count,
                    quantization, chat_template, author, role, pooling_type
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(filename) DO UPDATE SET
                    filepath       = excluded.filepath,
                    filesize_bytes = excluded.filesize_bytes,
                    architecture   = excluded.architecture,
                    model_name     = excluded.model_name,
                    context_length = excluded.context_length,
                    layer_count    = excluded.layer_count,
                    quantization   = excluded.quantization,
                    chat_template  = excluded.chat_template,
                    author         = excluded.author,
                    pooling_type   = excluded.pooling_type
                 RETURNING id",
                (
                    &rec.filename,
                    rec.filepath.to_string_lossy(),
                    rec.filesize_bytes,
                    &rec.metadata.architecture,
                    &rec.metadata.model_name,
                    rec.metadata.context_length,
                    rec.metadata.layer_count,
                    &rec.metadata.quantization,
                    rec.metadata.chat_template,
                    rec.metadata.author,
                    auto_role,
                    rec.metadata.pooling_type,
                ),
                |row| row.get(0),
            )
        })
        .await
        .map_err(|e| anyhow::anyhow!("Panic during upsert_model: {e}"))?
        .context("Failed to upsert model")?;
    Ok(id)
}

/// Delete a model row by filename. Cascades to `model_settings` (FK ON DELETE
/// CASCADE, set in Layer 1).
pub async fn delete_model_by_filename(pool: &Pool, filename: &str) -> Result<()> {
    let filename = filename.to_string();
    let conn = pool
        .get()
        .await
        .context("Failed to acquire ModelRegistry.db connection")?;
    conn.interact(move |c| c.execute("DELETE FROM models_metadata WHERE filename = ?1", [filename]))
        .await
        .map_err(|e| anyhow::anyhow!("Panic during delete_model_by_filename: {e}"))?
        .context("Failed to delete stale model row")?;
    Ok(())
}

/// Read all model rows, ordered by filename for stable display.
pub async fn list_models(pool: &Pool) -> Result<Vec<ModelSummary>> {
    let conn = pool
        .get()
        .await
        .context("Failed to acquire ModelRegistry.db connection")?;
    let rows = conn
        .interact(|c| {
            let mut stmt = c.prepare(
                "SELECT id, filename, filepath, filesize_bytes,
                        architecture, model_name, context_length, layer_count,
                        quantization, chat_template, author, role, pooling_type
                 FROM models_metadata
                 ORDER BY filename",
            )?;
            let mapped = stmt.query_map([], |r| {
                Ok(ModelSummary {
                    id: r.get(0)?,
                    filename: r.get(1)?,
                    filepath: r.get(2)?,
                    filesize_bytes: r.get(3)?,
                    architecture: r.get(4)?,
                    model_name: r.get(5)?,
                    context_length: r.get(6)?,
                    layer_count: r.get(7)?,
                    quantization: r.get(8)?,
                    chat_template: r.get(9)?,
                    author: r.get(10)?,
                    role: r.get(11)?,
                    pooling_type: r.get(12)?,
                })
            })?;
            let mut out = Vec::new();
            for row in mapped {
                out.push(row?);
            }
            Ok::<_, rusqlite::Error>(out)
        })
        .await
        .map_err(|e| anyhow::anyhow!("Panic during list_models: {e}"))?
        .context("Failed to list models")?;
    Ok(rows)
}

/// Full registry reconciliation: upsert every parsed record, delete rows whose
/// files are no longer on disk.
///
/// `all_filenames_on_disk` must include EVERY `.gguf` file found on disk —
/// both parsed AND skipped (unchanged) ones. Skipped files have no record to
/// upsert, but their DB row must be preserved. Without this set, the reconcile
/// logic would treat skipped files as "vanished" and delete their rows.
pub async fn reconcile_models(
    pool: &Pool,
    records: &[ModelRecord],
    all_filenames_on_disk: &std::collections::HashSet<String>,
) -> Result<ReconcileReport> {
    // Snapshot existing filenames so we can detect removals.
    let before: Vec<String> = {
        let conn = pool.get().await.context("DB connection for reconcile")?;
        conn.interact(|c| {
            let mut stmt = c.prepare("SELECT filename FROM models_metadata")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok::<_, rusqlite::Error>(out)
        })
        .await
        .map_err(|e| anyhow::anyhow!("Panic listing existing filenames: {e}"))?
        .context("Failed to read existing model filenames")?
    };
    let before_set: std::collections::HashSet<&str> =
        before.iter().map(|s| s.as_str()).collect();

    let mut report = ReconcileReport::default();

    // Upsert only the newly-parsed records (skipped files are left untouched).
    for rec in records {
        match upsert_model(pool, rec).await {
            Ok(_) => {
                if before_set.contains(rec.filename.as_str()) {
                    report.updated += 1;
                } else {
                    report.added += 1;
                }
            }
            Err(e) => {
                log::warn!("Failed to upsert {}: {e:#}", rec.filename);
                report.failed += 1;
            }
        }
    }

    // Remove rows whose files vanished from disk.
    // Uses all_filenames_on_disk (which includes skipped files) so unchanged
    // models are NOT deleted.
    for old_filename in &before {
        if !all_filenames_on_disk.contains(old_filename.as_str()) {
            if let Err(e) = delete_model_by_filename(pool, old_filename).await {
                log::warn!("Failed to delete stale row {}: {e:#}", old_filename);
                report.failed += 1;
            } else {
                report.removed += 1;
            }
        }
    }

    Ok(report)
}

/// Fetch a map of filename → filesize_bytes for all models in the registry.
/// Used by `scan_with_report` to skip parsing unchanged files (same name +
/// same size = same GGUF header = no need to re-read multi-GB files).
pub async fn list_known_file_sizes(pool: &Pool) -> Result<std::collections::HashMap<String, i64>> {
    let conn = pool
        .get()
        .await
        .context("DB connection for known-file-sizes")?;
    conn.interact(|c| {
        let mut stmt = c.prepare("SELECT filename, filesize_bytes FROM models_metadata")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut out = std::collections::HashMap::new();
        for r in rows {
            let (filename, size) = r?;
            out.insert(filename, size);
        }
        Ok::<_, rusqlite::Error>(out)
    })
    .await
    .map_err(|e| anyhow::anyhow!("Panic reading known file sizes: {e}"))?
    .context("Failed to read known file sizes")
}

/// Set the role tag on a model ('chat', 'embedding', or NULL to clear).
/// Used by the UI's role selector; read by the process manager.
pub async fn set_model_role(pool: &Pool, model_id: i64, role: Option<&str>) -> Result<()> {
    let role = role.map(|s| s.to_string());
    let conn = pool
        .get()
        .await
        .context("Failed to acquire ModelRegistry.db connection")?;
    conn.interact(move |c| {
        c.execute(
            "UPDATE models_metadata SET role = ?1 WHERE id = ?2",
            (role, model_id),
        )
    })
    .await
    .map_err(|e| anyhow::anyhow!("Panic during set_model_role: {e}"))?
    .context("Failed to update model role")?;
    Ok(())
}

/// The mutable per-model launch flags (mirrors the `model_settings` table).
/// Used by the process manager to assemble CLI args; edited by the Loader page.
/// All fields default to None (= "auto" — flag omitted, llama-server uses its
/// built-in default). Fields are only emitted in build_args when explicitly set.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelSettings {
    // ── Primary controls ──
    pub vram_allocation_mb: Option<i64>,
    pub ctx_size: Option<i64>,
    pub flash_attn: Option<bool>,
    pub cpu_mode: Option<bool>,
    // ── Performance ──
    pub batch_size: Option<i64>,
    pub ubatch_size: Option<i64>,
    pub threads: Option<i64>,
    pub threads_batch: Option<i64>,
    pub mlock: Option<bool>,
    pub no_mmap: Option<bool>,
    // ── KV Cache ──
    pub cache_type_k: Option<String>,
    pub cache_type_v: Option<String>,
    pub cache_prompt: Option<bool>,
    // ── Sampling (chat-only) ──
    pub temp: Option<f64>,
    pub top_k: Option<i64>,
    pub top_p: Option<f64>,
    pub min_p: Option<f64>,
    pub repeat_penalty: Option<f64>,
    pub repeat_last_n: Option<i64>,
    pub seed: Option<i64>,
    pub presence_penalty: Option<f64>,
    pub frequency_penalty: Option<f64>,
    pub typical_p: Option<f64>,
    pub xtc_probability: Option<f64>,
    pub xtc_threshold: Option<f64>,
    pub mirostat: Option<i64>,
    pub mirostat_lr: Option<f64>,
    pub mirostat_ent: Option<f64>,
    pub dry_multiplier: Option<f64>,
    pub dry_base: Option<f64>,
    pub dry_allowed_length: Option<i64>,
    // ── Context & Server ──
    pub predict: Option<i64>,
    pub context_shift: Option<bool>,
    pub parallel: Option<i64>,
    pub cont_batching: Option<bool>,
    pub timeout: Option<i64>,
    // ── RoPE ──
    pub rope_scaling: Option<String>,
    pub rope_freq_base: Option<f64>,
    // ── Reasoning (chat-only) ──
    pub reasoning_format: Option<String>,
    // ── Embedding-specific ──
    pub pooling_type_override: Option<String>,
    pub embd_normalize: Option<i64>,
    pub rerank: Option<bool>,
}

/// Load a model's saved settings. Returns None if the user hasn't customized
/// them yet (the caller computes defaults in that case).
pub async fn load_model_settings(pool: &Pool, model_id: i64) -> Result<Option<ModelSettings>> {
    let conn = pool
        .get()
        .await
        .context("Failed to acquire ModelRegistry.db connection")?;
    let row = conn
        .interact(move |c| {
            c.query_row(
                "SELECT vram_allocation_mb, ctx_size, flash_attn, cpu_mode,
                        batch_size, ubatch_size, threads, threads_batch,
                        mlock, no_mmap, cache_type_k, cache_type_v, cache_prompt,
                        temp, top_k, top_p, min_p, repeat_penalty, repeat_last_n, seed,
                        presence_penalty, frequency_penalty, typical_p,
                        xtc_probability, xtc_threshold,
                        mirostat, mirostat_lr, mirostat_ent,
                        dry_multiplier, dry_base, dry_allowed_length,
                        predict, context_shift, parallel, cont_batching, timeout,
                        rope_scaling, rope_freq_base, reasoning_format,
                        pooling_type_override, embd_normalize, rerank
                 FROM model_settings WHERE model_id = ?1",
                [model_id],
                |r| {
                    Ok(ModelSettings {
                        vram_allocation_mb: r.get(0)?,
                        ctx_size: r.get(1)?,
                        flash_attn: r.get(2)?,
                        cpu_mode: r.get(3)?,
                        batch_size: r.get(4)?,
                        ubatch_size: r.get(5)?,
                        threads: r.get(6)?,
                        threads_batch: r.get(7)?,
                        mlock: r.get(8)?,
                        no_mmap: r.get(9)?,
                        cache_type_k: r.get(10)?,
                        cache_type_v: r.get(11)?,
                        cache_prompt: r.get(12)?,
                        temp: r.get(13)?,
                        top_k: r.get(14)?,
                        top_p: r.get(15)?,
                        min_p: r.get(16)?,
                        repeat_penalty: r.get(17)?,
                        repeat_last_n: r.get(18)?,
                        seed: r.get(19)?,
                        presence_penalty: r.get(20)?,
                        frequency_penalty: r.get(21)?,
                        typical_p: r.get(22)?,
                        xtc_probability: r.get(23)?,
                        xtc_threshold: r.get(24)?,
                        mirostat: r.get(25)?,
                        mirostat_lr: r.get(26)?,
                        mirostat_ent: r.get(27)?,
                        dry_multiplier: r.get(28)?,
                        dry_base: r.get(29)?,
                        dry_allowed_length: r.get(30)?,
                        predict: r.get(31)?,
                        context_shift: r.get(32)?,
                        parallel: r.get(33)?,
                        cont_batching: r.get(34)?,
                        timeout: r.get(35)?,
                        rope_scaling: r.get(36)?,
                        rope_freq_base: r.get(37)?,
                        reasoning_format: r.get(38)?,
                        pooling_type_override: r.get(39)?,
                        embd_normalize: r.get(40)?,
                        rerank: r.get(41)?,
                    })
                },
            )
            .optional()
        })
        .await
        .map_err(|e| anyhow::anyhow!("Panic during load_model_settings: {e}"))?
        .context("Failed to load model_settings")?;
    Ok(row)
}

/// Save (or replace) a model's settings. The Loader page Save button writes here.
pub async fn save_model_settings(pool: &Pool, model_id: i64, s: &ModelSettings) -> Result<()> {
    let s = s.clone();
    let conn = pool
        .get()
        .await
        .context("Failed to acquire ModelRegistry.db connection")?;
    conn.interact(move |c| {
        // Use params_from_iter to avoid the tuple-size limit (rusqlite caps at ~16).
        let params: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(model_id),
            Box::new(s.vram_allocation_mb),
            Box::new(s.ctx_size),
            Box::new(s.flash_attn),
            Box::new(s.cpu_mode),
            Box::new(s.batch_size),
            Box::new(s.ubatch_size),
            Box::new(s.threads),
            Box::new(s.threads_batch),
            Box::new(s.mlock),
            Box::new(s.no_mmap),
            Box::new(&s.cache_type_k),
            Box::new(&s.cache_type_v),
            Box::new(s.cache_prompt),
            Box::new(s.temp),
            Box::new(s.top_k),
            Box::new(s.top_p),
            Box::new(s.min_p),
            Box::new(s.repeat_penalty),
            Box::new(s.repeat_last_n),
            Box::new(s.seed),
            Box::new(s.presence_penalty),
            Box::new(s.frequency_penalty),
            Box::new(s.typical_p),
            Box::new(s.xtc_probability),
            Box::new(s.xtc_threshold),
            Box::new(s.mirostat),
            Box::new(s.mirostat_lr),
            Box::new(s.mirostat_ent),
            Box::new(s.dry_multiplier),
            Box::new(s.dry_base),
            Box::new(s.dry_allowed_length),
            Box::new(s.predict),
            Box::new(s.context_shift),
            Box::new(s.parallel),
            Box::new(s.cont_batching),
            Box::new(s.timeout),
            Box::new(&s.rope_scaling),
            Box::new(s.rope_freq_base),
            Box::new(&s.reasoning_format),
            Box::new(&s.pooling_type_override),
            Box::new(s.embd_normalize),
            Box::new(s.rerank),
        ];
        c.execute(
            "INSERT INTO model_settings
                (model_id, vram_allocation_mb, ctx_size, flash_attn, cpu_mode,
                 batch_size, ubatch_size, threads, threads_batch,
                 mlock, no_mmap, cache_type_k, cache_type_v, cache_prompt,
                 temp, top_k, top_p, min_p, repeat_penalty, repeat_last_n, seed,
                 presence_penalty, frequency_penalty, typical_p,
                 xtc_probability, xtc_threshold,
                 mirostat, mirostat_lr, mirostat_ent,
                 dry_multiplier, dry_base, dry_allowed_length,
                 predict, context_shift, parallel, cont_batching, timeout,
                 rope_scaling, rope_freq_base, reasoning_format,
                 pooling_type_override, embd_normalize, rerank)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                     ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                     ?, ?, ?)
             ON CONFLICT(model_id) DO UPDATE SET
                vram_allocation_mb = excluded.vram_allocation_mb,
                ctx_size           = excluded.ctx_size,
                flash_attn         = excluded.flash_attn,
                cpu_mode           = excluded.cpu_mode,
                batch_size         = excluded.batch_size,
                ubatch_size        = excluded.ubatch_size,
                threads            = excluded.threads,
                threads_batch      = excluded.threads_batch,
                mlock              = excluded.mlock,
                no_mmap            = excluded.no_mmap,
                cache_type_k       = excluded.cache_type_k,
                cache_type_v       = excluded.cache_type_v,
                cache_prompt       = excluded.cache_prompt,
                temp               = excluded.temp,
                top_k              = excluded.top_k,
                top_p              = excluded.top_p,
                min_p              = excluded.min_p,
                repeat_penalty     = excluded.repeat_penalty,
                repeat_last_n      = excluded.repeat_last_n,
                seed               = excluded.seed,
                presence_penalty   = excluded.presence_penalty,
                frequency_penalty  = excluded.frequency_penalty,
                typical_p          = excluded.typical_p,
                xtc_probability    = excluded.xtc_probability,
                xtc_threshold      = excluded.xtc_threshold,
                mirostat           = excluded.mirostat,
                mirostat_lr        = excluded.mirostat_lr,
                mirostat_ent       = excluded.mirostat_ent,
                dry_multiplier     = excluded.dry_multiplier,
                dry_base           = excluded.dry_base,
                dry_allowed_length = excluded.dry_allowed_length,
                predict            = excluded.predict,
                context_shift      = excluded.context_shift,
                parallel           = excluded.parallel,
                cont_batching      = excluded.cont_batching,
                timeout            = excluded.timeout,
                rope_scaling       = excluded.rope_scaling,
                rope_freq_base     = excluded.rope_freq_base,
                reasoning_format   = excluded.reasoning_format,
                pooling_type_override = excluded.pooling_type_override,
                embd_normalize      = excluded.embd_normalize,
                rerank              = excluded.rerank",
            rusqlite::params_from_iter(params.iter()),
        )
    })
    .await
    .map_err(|e| anyhow::anyhow!("Panic during save_model_settings: {e}"))?
    .context("Failed to save model_settings")?;
    Ok(())
}


/// Check whether a model_settings row exists for the given model.
pub async fn model_has_settings(pool: &Pool, model_id: i64) -> Result<bool> {
    let conn = pool
        .get()
        .await
        .context("Failed to acquire ModelRegistry.db connection")?;
    let exists = conn
        .interact(move |c| {
            c.query_row(
                "SELECT 1 FROM model_settings WHERE model_id = ?1 LIMIT 1",
                [model_id],
                |_| Ok(1),
            )
            .is_ok()
        })
        .await
        .map_err(|e| anyhow::anyhow!("Panic during model_has_settings: {e}"))?;
    Ok(exists)
}

/// Batch query: fetch the set of model IDs that have saved settings. Replaces
/// the N+1 pattern of calling `model_has_settings` per model in a loop.
pub async fn models_with_settings(pool: &Pool) -> Result<std::collections::HashSet<i64>> {
    let conn = pool
        .get()
        .await
        .context("DB connection for models_with_settings")?;
    conn.interact(|c| {
        let mut stmt = c.prepare("SELECT model_id FROM model_settings")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        let mut out = std::collections::HashSet::new();
        for r in rows {
            out.insert(r?);
        }
        Ok::<_, rusqlite::Error>(out)
    })
    .await
    .map_err(|e| anyhow::anyhow!("Panic during models_with_settings: {e}"))?
    .context("Failed to read model_settings IDs")
}
