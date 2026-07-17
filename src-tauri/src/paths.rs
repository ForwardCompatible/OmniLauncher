//! Filesystem path resolution and on-disk layout bootstrap.
//!
//! OmniLauncher stores all mutable state (databases, downloaded models, logs)
//! under a single per-user app-data directory. On Linux this is
//! `~/.local/share/OmniLauncher/` (resolved via Tauri's `path` API, which
//! respects XDG on Linux).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tauri::Manager;

/// Resolve the per-user app-data directory, creating it if necessary.
///
/// Example on Linux: `~/.local/share/OmniLauncher/`
pub fn app_data_dir(app: &tauri::App) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .context("Tauri could not resolve the app_data_dir")?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create app data dir at {}", dir.display()))?;
    Ok(dir)
}

/// The resolved models layout: the models directory + multimodal subdirectory.
pub struct ModelsLayout {
    /// The top-level models directory (may be a symlink to the project tree).
    pub models_dir: PathBuf,
    /// The multimodal subdirectory inside models_dir.
    pub multimodal_dir: PathBuf,
}

/// Ensure the models directory layout exists, per AGENTS.md:
///   `<data_dir>/models/`
///   `<data_dir>/models/multimodal/`
///
/// In **dev mode** (when the project tree exists), the app-data `models/` is
/// symlinked to `<project_root>/models/` so developers can drop `.gguf` files
/// into the visible project folder and have them immediately discovered — no
/// copying needed.
///
/// In **packaged mode** (no project tree), a real directory is created in the
/// app-data dir.
pub fn ensure_models_layout(data_dir: &Path) -> Result<ModelsLayout> {
    let app_data_models = data_dir.join("models");

    // Try to locate the project-tree models dir (CARGO_MANIFEST_DIR is the
    // src-tauri/ dir at compile time; the project root is its parent).
    let project_models = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|root| root.join("models"));

    // If the project-tree models/ exists, symlink the app-data path to it.
    // This makes dropped files instantly visible to the scanner.
    if let Some(ref project_dir) = project_models {
        if project_dir.is_dir() {
            link_models_dir(&app_data_models, project_dir)?;
        }
    }

    // Ensure multimodal subdirectory exists (inside the resolved models dir,
    // which may be a symlink — so we create through it).
    let multimodal_dir = app_data_models.join("multimodal");
    std::fs::create_dir_all(&multimodal_dir).with_context(|| {
        format!(
            "Failed to create multimodal dir at {}",
            multimodal_dir.display()
        )
    })?;

    Ok(ModelsLayout {
        models_dir: app_data_models,
        multimodal_dir,
    })
}

/// Make `app_data_models` point at `project_dir` via symlink. If the app-data
/// path already exists as a symlink to the right target, this is a no-op.
/// If it exists as a real directory (from a prior run), we leave it as-is
/// rather than risk deleting user files.
fn link_models_dir(app_data_models: &Path, project_dir: &Path) -> Result<()> {
    // Already a symlink? Check if it points at the right place.
    if let Ok(existing_target) = std::fs::read_link(app_data_models) {
        if existing_target == project_dir {
            return Ok(()); // already linked correctly
        }
        // Points elsewhere — leave it alone, don't fight the user.
        log::debug!(
            "models symlink exists but points at {}, expected {}",
            existing_target.display(),
            project_dir.display()
        );
        return Ok(());
    }

    // Not a symlink. Does the app-data path exist as a real dir?
    if app_data_models.exists() {
        // It's a real directory with potentially user files. Leave it.
        log::debug!(
            "models dir exists as real dir at {}, not converting to symlink",
            app_data_models.display()
        );
        return Ok(());
    }

    // Neither exists nor is a symlink — safe to create the symlink.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(project_dir, app_data_models).with_context(|| {
            format!(
                "Failed to symlink {} → {}",
                app_data_models.display(),
                project_dir.display()
            )
        })?;
    }

    #[cfg(windows)]
    {
        // Windows: symlink_dir requires Developer Mode or admin privileges.
        // If it fails, the caller's fallback (create a real directory) handles it.
        match std::os::windows::fs::symlink_dir(project_dir, app_data_models) {
            Ok(()) => {}
            Err(e) => {
                log::debug!(
                    "Windows symlink_dir failed (may need Developer Mode): {e}. \
                     A real directory will be created instead."
                );
                return Ok(()); // Graceful fallback — caller creates a real dir
            }
        }
    }
    log::info!(
        "Symlinked models dir: {} → {}",
        app_data_models.display(),
        project_dir.display()
    );
    Ok(())
}
