//! Model registry — scans the models directory and produces the set of
//! currently-installed models, parsed from their GGUF headers.
//!
//! This module is pure filesystem + parsing; it does NOT touch the database.
//! The DB sync (upsert parsed records, delete stale rows) lives in
//! `db::registry_ops` and is driven by the `resync_registry` command, which
//! calls `scan_with_report()` then `db::reconcile_models(...)`.
//!
//! Per AGENTS.md "Startup: Registry syncs with OmniLauncher/models":
//!   * Walk `models/` for `.gguf` files.
//!   * Ignore `models/multimodal/` (vision projectors; out of scope for MVP).
//!   * Parse failures are logged at WARN but don't abort the whole scan.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::gguf::{self, GgufMetadata};

/// One discovered model: filesystem facts + parsed GGUF metadata.
#[derive(Debug, Clone)]
pub struct ModelRecord {
    pub filename: String,
    pub filepath: PathBuf,
    pub filesize_bytes: i64,
    pub metadata: GgufMetadata,
}

/// Summary of a scan — returned to the caller for logging / UI display.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ScanReport {
    /// Discovered (and successfully parsed) `.gguf` files.
    pub found: usize,
    /// Files that failed to parse (corrupt, truncated, not actually GGUF).
    pub failed: Vec<String>,
}


/// Build a ScanReport from a directory scan + a list of parse failures.
/// Convenience for the command layer.
///
/// `known_files` maps filename → filesize_bytes for models already in the DB.
/// When a file's stat'd size matches the known value, `gguf::parse` is skipped
/// (the header can't have changed if the file hasn't). This avoids multi-GB
/// reads for unchanged models at startup.
///
/// Returns `(records, all_filenames, report)` where `all_filenames` includes
/// BOTH parsed and skipped files. The caller passes this to `reconcile_models`
/// so skipped files are correctly treated as "still present" (not deleted).
pub fn scan_with_report(
    models_dir: &Path,
    known_files: &std::collections::HashMap<String, i64>,
) -> Result<(Vec<ModelRecord>, std::collections::HashSet<String>, ScanReport)> {
    if !models_dir.is_dir() {
        return Ok((Vec::new(), Default::default(), ScanReport::default()));
    }

    let multimodal_dir = models_dir.join("multimodal");
    let mut records = Vec::new();
    let mut all_filenames = std::collections::HashSet::new();
    let mut failed = Vec::new();

    for entry in walk_gguf_files(models_dir) {
        if entry.starts_with(&multimodal_dir) {
            continue;
        }
        match parse_one_or_skip(&entry, known_files) {
            Ok(Some(rec)) => {
                all_filenames.insert(rec.filename.clone());
                records.push(rec);
            }
            Ok(None) => {
                // Skipped (unchanged) — but we still need to record the filename
                // so reconcile_models knows it's still on disk.
                if let Some(name) = entry.file_name().and_then(|n| n.to_str()) {
                    all_filenames.insert(name.to_string());
                }
            }
            Err(e) => {
                log::warn!("Failed to parse {}: {e:#}", entry.display());
                failed.push(entry.display().to_string());
            }
        }
    }

    let report = ScanReport {
        found: records.len(),
        failed,
    };
    Ok((records, all_filenames, report))
}

/// Parse a `.gguf` file, OR skip parsing if the file is known-unchanged
/// (filename + filesize match an existing DB row). Returns `Ok(None)` when
/// skipped — the caller simply doesn't include it in the records list, and
/// `reconcile_models` will leave the existing DB row untouched (it's still
/// in the `before_set` and still in `seen`).
fn parse_one_or_skip(
    path: &Path,
    known_files: &std::collections::HashMap<String, i64>,
) -> Result<Option<ModelRecord>> {
    let filesize_bytes = std::fs::metadata(path)
        .with_context(|| format!("stat failed for {}", path.display()))?
        .len() as i64;
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("invalid filename (non-UTF8): {}", path.display()))?;

    // Skip parsing if the file hasn't changed since the last scan.
    // Same filename + same byte count = same GGUF header = no need to re-read.
    if let Some(&known_size) = known_files.get(&filename) {
        if known_size == filesize_bytes {
            log::debug!("Skipping unchanged model: {filename}");
            return Ok(None);
        }
    }

    let metadata = gguf::parse(path)?;
    Ok(Some(ModelRecord {
        filename,
        filepath: path.to_path_buf(),
        filesize_bytes,
        metadata,
    }))
}

/// Recursively collect all `*.gguf` file paths under `root`, sorted for
/// deterministic ordering. Errors reading individual entries are logged and
/// skipped — we never abort the whole walk because one unreadable subdir.
///
/// **Symlinks are followed** (via `metadata()` rather than `symlink_metadata()`)
/// because symlinking multi-GB model files into the models dir is the standard
/// way to avoid duplicating them. Broken symlinks (dangling target) are logged
/// at WARN and skipped.
fn walk_gguf_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let read = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) => {
                log::warn!("Skipping unreadable dir {}: {e}", dir.display());
                continue;
            }
        };
        for entry in read.flatten() {
            let path = entry.path();
            // `metadata()` follows symlinks; `symlink_metadata()` would not.
            // We want the target's type so symlinked models are discovered.
            let md = match std::fs::metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    // Dangling symlink or permission issue — log and move on.
                    log::warn!("Skipping {} (metadata error): {e}", path.display());
                    continue;
                }
            };
            if md.is_dir() {
                stack.push(path);
            } else if md.is_file() && path.extension().and_then(|e| e.to_str()) == Some("gguf") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    /// scan_models on a non-existent dir returns an empty vec, not an error.
    #[test]
    fn scan_missing_dir_is_empty_not_error() {
        let tmp = tempdir();
        let nonexistent = tmp.join("does-not-exist");
        let (result, _, _) = scan_with_report(&nonexistent, &Default::default()).expect("should not error");
        assert!(result.is_empty());
    }

    /// An empty models dir yields no records.
    #[test]
    fn scan_empty_dir_yields_nothing() {
        let tmp = tempdir();
        let models = tmp.join("models");
        fs::create_dir_all(&models).unwrap();
        let (result, _, _) = scan_with_report(&models, &Default::default()).unwrap();
        assert!(result.is_empty());
    }

    /// Non-.gguf files are silently ignored (partial downloads, READMEs, etc.).
    #[test]
    fn ignores_non_gguf_files() {
        let tmp = tempdir();
        let models = tmp.join("models");
        fs::create_dir_all(&models).unwrap();
        fs::write(models.join("README.txt"), b"hello").unwrap();
        fs::write(models.join("model.part"), b"partial").unwrap();
        let (result, _, _) = scan_with_report(&models, &Default::default()).unwrap();
        assert!(result.is_empty(), "non-gguf files should be ignored");
    }

    /// A corrupt file renamed to .gguf fails to parse but is recorded as a
    /// failure rather than aborting the scan.
    #[test]
    fn corrupt_gguf_is_recorded_as_failure() {
        let tmp = tempdir();
        let models = tmp.join("models");
        fs::create_dir_all(&models).unwrap();
        fs::write(models.join("broken.gguf"), b"this is not a real gguf file").unwrap();

        let (records, _, report) = scan_with_report(&models, &Default::default()).unwrap();
        assert!(records.is_empty(), "no valid records");
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].ends_with("broken.gguf"));
    }

    /// Files under models/multimodal/ are skipped (out of scope for MVP).
    #[test]
    fn skips_multimodal_subdir() {
        let tmp = tempdir();
        let models = tmp.join("models");
        let multimodal = models.join("multimodal");
        fs::create_dir_all(&multimodal).unwrap();
        // Drop a fake gguf in multimodal/ — it should be skipped.
        fs::write(multimodal.join("projector.gguf"), b"fake multimodal").unwrap();
        // And a corrupt one in models/ root to ensure the walk still visits it.
        fs::write(models.join("root.gguf"), b"not real").unwrap();

        let (_records, _, report) = scan_with_report(&models, &Default::default()).unwrap();
        // The corrupt root.gguf is recorded as a failure; the multimodal one is
        // not even attempted (skipped), so it doesn't appear in failed.
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].ends_with("root.gguf"));
    }

    /// Symlinked .gguf files MUST be discovered (following the link to the
    /// target). This is how users avoid duplicating multi-GB model files.
    #[test]
    #[cfg(unix)]
    fn follows_symlinked_gguf_files() {
        use std::os::unix::fs::symlink;
        let tmp = tempdir();
        let models = tmp.join("models");
        let elsewhere = tmp.join("elsewhere");
        let real_file = elsewhere.join("real.gguf");
        fs::create_dir_all(&models).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        // A real (corrupt) gguf living outside models/ — symlinked in.
        fs::write(&real_file, b"not a real gguf but has the extension").unwrap();
        symlink(&real_file, models.join("linked.gguf")).unwrap();

        let (_records, _, report) = scan_with_report(&models, &Default::default()).unwrap();
        // The symlink was followed: the target was parsed and failed (it's not
        // a real GGUF), so it shows up in `failed` — proving the link was
        // traversed rather than silently skipped.
        assert_eq!(report.failed.len(), 1, "symlinked gguf should be scanned");
        assert!(report.failed[0].ends_with("linked.gguf"));
    }

    /// A dangling symlink (target doesn't exist) is logged and skipped, not
    /// treated as a parse failure or a crash.
    #[test]
    #[cfg(unix)]
    fn dangling_symlink_is_skipped_not_fatal() {
        use std::os::unix::fs::symlink;
        let tmp = tempdir();
        let models = tmp.join("models");
        fs::create_dir_all(&models).unwrap();
        symlink("/nonexistent/target.gguf", models.join("dangling.gguf")).unwrap();

        let (records, _, report) = scan_with_report(&models, &Default::default()).unwrap();
        // No records, no parse failures — the dangling link was just skipped.
        assert!(records.is_empty());
        assert!(report.failed.is_empty(),
            "dangling symlink should be skipped, not recorded as failed: {report:?}");
    }

    /// Create a unique temp dir for a test. Leaks on test failure but that's
    /// fine for short-lived unit tests.
    fn tempdir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "omnilauncher-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&p).unwrap();
        // Touch a sentinel so we know it exists.
        let _ = std::fs::File::create(p.join(".sentinel")).map(|mut f| {
            let _ = f.write_all(b"");
        });
        p
    }
}
