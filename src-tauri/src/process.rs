//! Pure launch-logic module — `compute_default_settings` and `build_args`
//! (the VRAM Translation Engine). No process spawning, no tokio, no I/O.
//!
//! Process lifecycle (spawn/stop/status) lives in `sidecar.rs`. This module
//! is purely: given a model + settings + hardware, produce the CLI arg vector.

use crate::db::registry_ops::{ModelSettings, ModelSummary};
use crate::hardware::HardwareProfile;

/// Default context-size cap for auto-computed settings. Even if the model
/// supports a longer context, this keeps the default conservative to avoid
/// excessive RAM/VRAM allocation. Users can override per-model.
const DEFAULT_CTX_SIZE_CAP: i64 = 4096;

/// VRAM allocation fraction: the default is 80% of total VRAM (8/10). The user
/// can override this per-model via the VRAM slider.
const VRAM_ALLOCATION_NUM: i64 = 8;
const VRAM_ALLOCATION_DENOM: i64 = 10;

/// Minimum `--fit-target` in MiB. Clamped to prevent the fit engine from
/// receiving an impossibly small target when the user sets near-100% allocation.
const FIT_TARGET_FLOOR_MIB: i64 = 256;

/// Which proxy route a launched model serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Chat,
    Embedding,
}

impl Role {
    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "chat" => Some(Role::Chat),
            "embedding" => Some(Role::Embedding),
            _ => None,
        }
    }
}

/// Result of a successful launch (returned by the sidecar controller).
#[derive(Debug, serde::Serialize)]
pub struct LaunchReport {
    pub model_id: i64,
    pub model_name: String,
    pub role: Role,
    pub port: u16,
    pub pid: u32,
    pub args: Vec<String>,
}

/// Reasonable defaults for a model the user hasn't customized yet.
/// Uses ModelSettings::default() (all None = auto/omit) and overrides only
/// the computed fields that depend on hardware + model metadata.
pub fn compute_default_settings(model: &ModelSummary, hw: &HardwareProfile) -> ModelSettings {
    let ctx_size = model.context_length.min(DEFAULT_CTX_SIZE_CAP);

    let vram_allocation_mb = if hw.gpu_present {
        (hw.total_vram_mb * VRAM_ALLOCATION_NUM / VRAM_ALLOCATION_DENOM).max(0)
    } else {
        0
    };

    ModelSettings {
        vram_allocation_mb: Some(vram_allocation_mb),
        ctx_size: Some(ctx_size),
        flash_attn: Some(true),
        cpu_mode: Some(false),
        ..ModelSettings::default()
    }
}

/// Assemble the full `llama-server` CLI arg vector from model + settings +
/// hardware. This is the VRAM Translation Engine per AGENTS.md.
///
/// ## GPU path (gpu_present && allocation > 0 && !cpu_mode)
/// Emits `-ngl 999`, `--fit on`, `--fit-target <total − allocation>` (MiB).
///
/// ## CPU-only safety valve
/// Forces `-ngl 0`, `--fit off`, no `--fit-target`.
pub fn build_args(
    model_filepath: &str,
    settings: &ModelSettings,
    hw: &HardwareProfile,
    role: Role,
) -> Vec<String> {
    let mut args = vec![
        "--model".into(),
        model_filepath.to_string(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        "0".into(),
    ];

    // Embedding-specific flags.
    if role == Role::Embedding {
        args.push("--embeddings".into());
        if let Some(p) = &settings.pooling_type_override {
            args.push("--pooling".into());
            args.push(p.clone());
        }
        if let Some(n) = settings.embd_normalize {
            args.push("--embd-normalize".into());
            args.push(n.to_string());
        }
        if settings.rerank.unwrap_or(false) {
            args.push("--rerank".into());
        }
    }

    // Context + batch.
    if let Some(ctx) = settings.ctx_size {
        args.push("--ctx-size".into());
        args.push(ctx.to_string());
    }
    if let Some(b) = settings.batch_size {
        args.push("--batch-size".into());
        args.push(b.to_string());
    }
    if let Some(ub) = settings.ubatch_size {
        args.push("--ubatch-size".into());
        args.push(ub.to_string());
    }

    // Flash attention.
    match settings.flash_attn {
        Some(true) => {
            args.push("--flash-attn".into());
            args.push("on".into());
        }
        Some(false) => {
            args.push("--flash-attn".into());
            args.push("off".into());
        }
        None => {}
    }

    // KV cache quantization.
    if let Some(k) = &settings.cache_type_k {
        args.push("--cache-type-k".into());
        args.push(k.clone());
    }
    if let Some(v) = &settings.cache_type_v {
        args.push("--cache-type-v".into());
        args.push(v.clone());
    }

    // ── VRAM Translation Engine core ──
    let allocation = settings.vram_allocation_mb.unwrap_or(0).max(0);
    let cpu_mode = settings.cpu_mode.unwrap_or(false);
    let gpu_mode = !cpu_mode && hw.gpu_present && allocation > 0;

    if gpu_mode {
        let margin = (hw.total_vram_mb - allocation).max(FIT_TARGET_FLOOR_MIB);
        // Use "auto" (not 999) so the --fit engine can dynamically reduce the
        // layer count to fit within the VRAM budget. A hardcoded 999 causes the
        // fit engine to abort ("n_gpu_layers already set by user") and the
        // model OOMs trying to load all layers at once.
        args.push("-ngl".into());
        args.push("auto".into());
        args.push("--fit".into());
        args.push("on".into());
        args.push("--fit-target".into());
        args.push(margin.to_string());
    } else {
        args.push("-ngl".into());
        args.push("0".into());
        args.push("--fit".into());
        args.push("off".into());
    }

    // Threads.
    if let Some(t) = settings.threads {
        if t > 0 {
            args.push("--threads".into());
            args.push(t.to_string());
        }
    }
    if let Some(tb) = settings.threads_batch {
        if tb > 0 {
            args.push("--threads-batch".into());
            args.push(tb.to_string());
        }
    }

    // Binary flags.
    if settings.mlock.unwrap_or(false) {
        args.push("--mlock".into());
    }
    if settings.no_mmap.unwrap_or(false) {
        args.push("--no-mmap".into());
    }
    if settings.cache_prompt.unwrap_or(false) {
        args.push("--cache-prompt".into());
    }

    // ── Sampling parameters (chat-only) ──
    if role == Role::Chat {
        if let Some(v) = settings.temp {
            args.push("--temp".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.top_k {
            args.push("--top-k".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.top_p {
            args.push("--top-p".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.min_p {
            args.push("--min-p".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.repeat_penalty {
            args.push("--repeat-penalty".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.repeat_last_n {
            args.push("--repeat-last-n".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.seed {
            args.push("--seed".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.presence_penalty {
            args.push("--presence-penalty".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.frequency_penalty {
            args.push("--frequency-penalty".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.typical_p {
            args.push("--typical-p".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.xtc_probability {
            args.push("--xtc-probability".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.xtc_threshold {
            args.push("--xtc-threshold".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.mirostat {
            args.push("--mirostat".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.mirostat_lr {
            args.push("--mirostat-lr".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.mirostat_ent {
            args.push("--mirostat-ent".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.dry_multiplier {
            args.push("--dry-multiplier".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.dry_base {
            args.push("--dry-base".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.dry_allowed_length {
            args.push("--dry-allowed-length".into());
            args.push(v.to_string());
        }
        if let Some(v) = settings.predict {
            args.push("--predict".into());
            args.push(v.to_string());
        }
        if settings.context_shift.unwrap_or(false) {
            args.push("--context-shift".into());
        }
        if let Some(v) = &settings.reasoning_format {
            args.push("--reasoning-format".into());
            args.push(v.clone());
        }
    }

    // ── Server config (both roles) ──
    if let Some(v) = settings.parallel {
        args.push("--parallel".into());
        args.push(v.to_string());
    }
    if settings.cont_batching.unwrap_or(false) {
        args.push("--cont-batching".into());
    }
    if let Some(v) = settings.timeout {
        args.push("--timeout".into());
        args.push(v.to_string());
    }

    // ── RoPE (both roles) ──
    if let Some(v) = &settings.rope_scaling {
        args.push("--rope-scaling".into());
        args.push(v.clone());
    }
    if let Some(v) = settings.rope_freq_base {
        args.push("--rope-freq-base".into());
        args.push(v.to_string());
    }

    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu_hw(vram_mb: i64) -> HardwareProfile {
        HardwareProfile {
            gpu_name: "Test GPU".into(),
            total_vram_mb: vram_mb,
            total_system_ram_mb: 32000,
            cpu_physical_cores: 8,
            cpu_logical_threads: 16,
            last_scanned_at: "2026-07-09T00:00:00Z".into(),
            gpu_present: vram_mb > 0,
        }
    }

    fn cpu_hw() -> HardwareProfile {
        HardwareProfile {
            gpu_name: "CPU-only (no NVIDIA GPU)".into(),
            total_vram_mb: 0,
            total_system_ram_mb: 32000,
            cpu_physical_cores: 8,
            cpu_logical_threads: 16,
            last_scanned_at: "2026-07-09T00:00:00Z".into(),
            gpu_present: false,
        }
    }

    fn model(ctx: i64) -> ModelSummary {
        ModelSummary {
            id: 1,
            filename: "test.gguf".into(),
            filepath: "/tmp/test.gguf".into(),
            filesize_bytes: 1000,
            architecture: "llama".into(),
            model_name: "Test".into(),
            context_length: ctx,
            layer_count: 32,
            quantization: "Q4_K_M".into(),
            chat_template: None,
            author: None,
            role: None,
            pooling_type: None,
        }
    }

    fn settings(vram: i64) -> ModelSettings {
        ModelSettings {
            vram_allocation_mb: Some(vram),
            ctx_size: Some(4096),
            flash_attn: Some(true),
            cpu_mode: Some(false),
            ..ModelSettings::default()
        }
    }

    fn val_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(|s| s.as_str())
    }

    // ── compute_default_settings tests ──

    #[test]
    fn gpu_defaults_allocate_80_percent_vram() {
        let m = model(4096);
        let hw = gpu_hw(6144);
        let s = compute_default_settings(&m, &hw);
        assert_eq!(s.vram_allocation_mb, Some(4915));
        assert_eq!(s.ctx_size, Some(4096));
        assert_eq!(s.flash_attn, Some(true));
    }

    #[test]
    fn ctx_size_capped_when_model_context_is_huge() {
        let m = model(131072);
        let hw = gpu_hw(8192);
        let s = compute_default_settings(&m, &hw);
        assert_eq!(s.ctx_size, Some(4096));
    }

    #[test]
    fn cpu_only_defaults_give_zero_vram() {
        let m = model(4096);
        let s = compute_default_settings(&m, &cpu_hw());
        assert_eq!(s.vram_allocation_mb, Some(0));
        assert_eq!(s.ctx_size, Some(4096));
        assert_eq!(s.flash_attn, Some(true));
    }

    // ── build_args tests ──

    #[test]
    fn gpu_path_emits_fit_target_as_total_minus_allocation() {
        let hw = gpu_hw(6144);
        let s = settings(4096);
        let args = build_args("/models/test.gguf", &s, &hw, Role::Chat);
        assert_eq!(val_after(&args, "-ngl"), Some("auto"));
        assert_eq!(val_after(&args, "--fit"), Some("on"));
        assert_eq!(val_after(&args, "--fit-target"), Some("2048"));
    }

    #[test]
    fn gpu_path_includes_model_host_port_and_context() {
        let hw = gpu_hw(8192);
        let s = settings(4096);
        let args = build_args("/path/to/model.gguf", &s, &hw, Role::Chat);
        assert_eq!(val_after(&args, "--model"), Some("/path/to/model.gguf"));
        assert_eq!(val_after(&args, "--host"), Some("127.0.0.1"));
        assert_eq!(val_after(&args, "--port"), Some("0"));
        assert_eq!(val_after(&args, "--ctx-size"), Some("4096"));
        // batch_size and ubatch_size now default to None (auto/omit).
        assert!(!args.iter().any(|a| a == "--batch-size"));
        assert!(!args.iter().any(|a| a == "--ubatch-size"));
    }

    #[test]
    fn flash_attn_emitted_as_on_off_not_bare_flag() {
        let hw = gpu_hw(8192);
        let mut s = settings(4096);
        s.flash_attn = Some(true);
        let args = build_args("/m.gguf", &s, &hw, Role::Chat);
        assert_eq!(val_after(&args, "--flash-attn"), Some("on"));

        s.flash_attn = Some(false);
        let args = build_args("/m.gguf", &s, &hw, Role::Chat);
        assert_eq!(val_after(&args, "--flash-attn"), Some("off"));
    }

    #[test]
    fn cpu_only_safety_valve_forces_ngl_0_and_fit_off() {
        let s = settings(0);
        let args = build_args("/m.gguf", &s, &cpu_hw(), Role::Chat);
        assert_eq!(val_after(&args, "-ngl"), Some("0"));
        assert_eq!(val_after(&args, "--fit"), Some("off"));
        assert!(!args.iter().any(|a| a == "--fit-target"));
    }

    #[test]
    fn gpu_present_but_zero_allocation_also_triggers_safety_valve() {
        let hw = gpu_hw(8192);
        let s = settings(0);
        let args = build_args("/m.gguf", &s, &hw, Role::Chat);
        assert_eq!(val_after(&args, "-ngl"), Some("0"));
        assert_eq!(val_after(&args, "--fit"), Some("off"));
        assert!(!args.iter().any(|a| a == "--fit-target"));
    }

    #[test]
    fn fit_target_clamps_to_floor_when_allocation_exceeds_total() {
        let hw = gpu_hw(6144);
        let s = settings(8192);
        let args = build_args("/m.gguf", &s, &hw, Role::Chat);
        assert_eq!(val_after(&args, "--fit"), Some("on"));
        let margin: i64 = val_after(&args, "--fit-target").unwrap().parse().unwrap();
        assert!(margin >= 256);
    }

    #[test]
    fn kv_cache_types_emitted_when_set() {
        let hw = gpu_hw(8192);
        let mut s = settings(4096);
        s.cache_type_k = Some("q8_0".into());
        s.cache_type_v = Some("q4_0".into());
        let args = build_args("/m.gguf", &s, &hw, Role::Chat);
        assert_eq!(val_after(&args, "--cache-type-k"), Some("q8_0"));
        assert_eq!(val_after(&args, "--cache-type-v"), Some("q4_0"));
    }

    #[test]
    fn full_gpu_arg_vector_matches_expected_shape() {
        // Minimal args when everything is "auto" (None):
        // only model, host, port, ctx-size, flash-attn, ngl, fit, fit-target.
        let hw = gpu_hw(6144);
        let s = settings(4915);
        let args = build_args("/home/ryan/models/test.gguf", &s, &hw, Role::Chat);
        let expected = vec![
            "--model", "/home/ryan/models/test.gguf",
            "--host", "127.0.0.1",
            "--port", "0",
            "--ctx-size", "4096",
            "--flash-attn", "on",
            "-ngl", "auto",
            "--fit", "on",
            "--fit-target", "1229",
        ];
        assert_eq!(args, expected);
    }

    #[test]
    fn embedding_role_adds_embeddings_flag() {
        let hw = gpu_hw(8192);
        let s = settings(4096);
        let args = build_args("/m.gguf", &s, &hw, Role::Embedding);
        assert!(args.iter().any(|a| a == "--embeddings"));
    }

    #[test]
    fn chat_role_does_not_add_embeddings_flag() {
        let hw = gpu_hw(8192);
        let s = settings(4096);
        let args = build_args("/m.gguf", &s, &hw, Role::Chat);
        assert!(!args.iter().any(|a| a == "--embeddings"));
        assert!(!args.iter().any(|a| a == "--pooling"));
        assert!(!args.iter().any(|a| a == "--embd-normalize"));
        assert!(!args.iter().any(|a| a == "--rerank"));
    }

    #[test]
    fn embedding_specific_flags_emitted_when_set() {
        let hw = gpu_hw(8192);
        let mut s = settings(4096);
        s.pooling_type_override = Some("mean".into());
        s.embd_normalize = Some(2);
        s.rerank = Some(true);
        let args = build_args("/m.gguf", &s, &hw, Role::Embedding);
        assert_eq!(val_after(&args, "--pooling"), Some("mean"));
        assert_eq!(val_after(&args, "--embd-normalize"), Some("2"));
        assert!(args.iter().any(|a| a == "--rerank"));
    }

    #[test]
    fn embedding_defaults_omit_pooling_and_normalize() {
        let hw = gpu_hw(8192);
        let s = settings(4096);
        let args = build_args("/m.gguf", &s, &hw, Role::Embedding);
        assert!(!args.iter().any(|a| a == "--pooling"));
        assert!(!args.iter().any(|a| a == "--embd-normalize"));
        assert!(!args.iter().any(|a| a == "--rerank"));
        assert!(args.iter().any(|a| a == "--embeddings"));
    }

    #[test]
    fn cpu_mode_overrides_gpu_and_forces_safety_valve() {
        let hw = gpu_hw(8192);
        let mut s = settings(4096);
        s.cpu_mode = Some(true);
        let args = build_args("/m.gguf", &s, &hw, Role::Chat);
        assert_eq!(val_after(&args, "-ngl"), Some("0"));
        assert_eq!(val_after(&args, "--fit"), Some("off"));
        assert!(!args.iter().any(|a| a == "--fit-target"));
    }

    #[test]
    fn mlock_and_no_mmap_emitted_when_enabled() {
        let hw = gpu_hw(8192);
        let mut s = settings(4096);
        s.mlock = Some(true);
        s.no_mmap = Some(true);
        let args = build_args("/m.gguf", &s, &hw, Role::Chat);
        assert!(args.iter().any(|a| a == "--mlock"));
        assert!(args.iter().any(|a| a == "--no-mmap"));
    }

    #[test]
    fn threads_emitted_when_set() {
        let hw = gpu_hw(8192);
        let mut s = settings(4096);
        s.threads = Some(8);
        s.threads_batch = Some(12);
        let args = build_args("/m.gguf", &s, &hw, Role::Chat);
        assert_eq!(val_after(&args, "--threads"), Some("8"));
        assert_eq!(val_after(&args, "--threads-batch"), Some("12"));
    }

    #[test]
    fn threads_omitted_when_none_or_zero() {
        let hw = gpu_hw(8192);
        let s = settings(4096);
        let args = build_args("/m.gguf", &s, &hw, Role::Chat);
        assert!(!args.iter().any(|a| a == "--threads"));
        let mut s2 = settings(4096);
        s2.threads = Some(0);
        let args2 = build_args("/m.gguf", &s2, &hw, Role::Chat);
        assert!(!args2.iter().any(|a| a == "--threads"));
    }

    #[test]
    fn role_from_db_str_parses_known_values() {
        assert_eq!(Role::from_db_str("chat"), Some(Role::Chat));
        assert_eq!(Role::from_db_str("embedding"), Some(Role::Embedding));
        assert_eq!(Role::from_db_str(""), None);
        assert_eq!(Role::from_db_str("unknown"), None);
    }
}
