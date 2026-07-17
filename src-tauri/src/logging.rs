//! Central logging configuration.
//!
//! Implements the AGENTS.md "Central Logging System" requirement:
//!   * Writes a unified, rotating flat-file log to the OS log directory.
//!   * Forwards Rust logs to the frontend webview console.
//!   * Also emits to stdout for dev visibility.
//!
//! Frontend errors are bridged into this same log via the @tauri-apps/plugin-log
//! JS counterpart (installed on the Svelte side).

use tauri_plugin_log::{Builder, RotationStrategy, Target, TargetKind};

/// Build the logging plugin. Mounted in `lib::run()`.
pub fn build_plugin() -> Builder {
    Builder::new()
        .targets([
            Target::new(TargetKind::Stdout),
            Target::new(TargetKind::Webview),
            Target::new(TargetKind::LogDir {
                file_name: Some("omnilauncher".into()),
            }),
        ])
        .level(log::LevelFilter::Info)
        .max_file_size(5_000_000) // 5 MiB before rotation
        .rotation_strategy(RotationStrategy::KeepAll)
}
