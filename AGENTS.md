# OmniLauncher — Cross-Platform Development Specification

## Overview & Core Architecture

**Target OS:** Ubuntu Linux (primary) and Windows Native (secondary).

**App Base:** Rust (Backend) + Svelte 5 (Frontend, pure JavaScript).

**Framework:** Tauri v2.

**Core Binary:** CUDA-accelerated `llama-server` (x86_64). Source differs per OS — each ships the same CLI surface, only the build origin differs.
- **Linux:** A pinned ai-dock CUDA build (`b9893`, CUDA 12.8.1). Bundled directly in the project structure as a Tauri resource. Includes sibling `.so` libraries.
- **Windows:** The official ggml-org/llama.cpp CUDA build (`b9821`, CUDA 12.4). Downloaded during installation to `%APPDATA%/com.omnilauncher.app/binaries/` along with the matching CUDA 12.4 runtime DLLs (`cudart64_12.dll`, `cublas64_12.dll`, ...). Not bundled in the installer.

### Coding Standards

- Prioritize memory safety and strict typing (Rust).
- Utilize `tauri::command` for all backend-to-frontend communication.
- All SQLite operations must use `rusqlite` with connection pooling (deadpool-sqlite) to support WAL mode.
- Do not introduce any external dependencies not strictly required for the MVP.
- **Pure JavaScript frontend** — no TypeScript. Types expressed as JSDoc `@typedef` annotations.
- **Centralized API layer** — all `invoke()` calls live in `src/lib/commands.js`.
- **Infrastructure encapsulation** — all process management goes through `SidecarController`. No raw tokio/std process calls in command handlers.
- **Flag dictionary separation** — `flag_dictionary` (System.db) feeds ONLY tooltip text. `model_settings` (ModelRegistry.db) drives all configurable flags.

### Cross-Platform Rules

- All file paths must use `std::path::PathBuf` — no hardcoded `/` or `\` separators.
- OS-specific logic gated behind `#[cfg(target_os = "...")]` blocks.
- `nvml-wrapper` and `sysinfo` are cross-platform and require no `cfg` gates.

### Execution & Hardware Branching

CUDA binary used universally.

**GPU Present:** Hybrid mode via `--fit on` + `-ngl auto` + `--fit-target <margin>`. The fit engine dynamically allocates layers between GPU VRAM and system RAM.

**CPU-Only (Zero VRAM):** Safety valve forces `-ngl 0` and `--fit off`, stripping all parameter-fitting flags to prevent CUDA initialization panics.

---

## Database Architecture

**Engine:** SQLite with WAL mode. Per-connection pragmas applied in deadpool's `post_create` hook: `foreign_keys=ON`, `journal_mode=WAL`, `synchronous=NORMAL`, `busy_timeout=10000`.

### System Database (System.db)

#### `app_settings` (Singleton, ID=1)
| Column | Type | Description |
|--------|------|-------------|
| `models_directory` | TEXT | Path to models directory |
| `multimodal_directory` | TEXT | Default: `models/multimodal` |
| `master_port` | INTEGER | Master port for reverse proxy (default: 52715) |
| `auto_port_increment` | BOOLEAN | Auto-increment port if busy |
| `theme` | TEXT | UI theme setting |

#### `hardware_profile` (Singleton, ID=1)
Cached on first launch. Manual rescan available on Settings page. Uses `cpu_physical_cores > 0` as the cache-validity indicator.

| Column | Type | Description |
|--------|------|-------------|
| `gpu_name` | TEXT | GPU model name |
| `total_vram_mb` | INTEGER | Total VRAM in MiB |
| `total_system_ram_mb` | INTEGER | Total system RAM in MiB |
| `cpu_physical_cores` | INTEGER | Physical CPU cores |
| `cpu_logical_threads` | INTEGER | Logical CPU threads |
| `last_scanned_at` | TIMESTAMP | Last hardware scan timestamp |

**`has_usable_gpu()`** method on `HardwareProfileRow` is the single source of truth for GPU availability. All code must call this — no duplicate `starts_with("CPU-only")` checks.

#### `flag_dictionary`
Strictly for tooltip text. Decoupled from configuration. ~40 entries across 8 categories.

### Model Registry Database (ModelRegistry.db)

#### `models_metadata`
Immutable GGUF header data. Includes auto-detected `role` ('chat'/'embedding'/NULL) and `pooling_type` from the GGUF `{arch}.pooling_type` field.

#### `model_settings`
42 nullable columns, one per launch flag. All default to `None` (= "auto" — flag omitted). `#[derive(Default)]` on the Rust struct.

---

## Sidecar Controller (Process Management)

Encapsulates all `llama-server` lifecycle. Clean `start`/`stop`/`status`/`shutdown_all` interface.

### Linux
- Binary: bundled as Tauri resource at `resources/llama-server/llama-server`
- Environment: `LD_LIBRARY_PATH` set to binary's directory for sibling `.so` resolution
- Termination: `libc::kill(pid, SIGTERM)` → grace → `SIGKILL`

### Windows
- Binary: downloaded during install to `%APPDATA%/com.omnilauncher.app/binaries/llama-server.exe`
- Environment: binary directory prepended to `PATH` for side-by-side DLL resolution
- Spawn flags: `CREATE_NO_WINDOW` (0x08000000) to prevent console popups
- Termination: `OpenProcess` + `TerminateProcess` via `windows-sys` crate (no POSIX signals on Windows)

---

## UI Layout & Routing

**Shell:** AppShell with fixed header, collapsible left NavRail, scrollable main content, fixed footer. Height locked to viewport.

**Loader Page:** Two separate component files — `ChatModelCard.svelte` and `EmbeddingModelCard.svelte`. Each renders only its role-relevant flags. Shared flags (VRAM, ctx_size, threads, batch, cache types) written independently in each file.

**Error Queue:** Array of `{message, timestamp, severity}` items. All refresh functions route failures to the queue. No silent error swallowing.

---

## API Traffic Routing (Reverse Proxy)

axum-based reverse proxy on `127.0.0.1:{master_port}`. Path-based routing to dynamically assigned backend ports. SSE streaming pass-through. 503 with OpenAI error shape when no backend is routed.

---

## VRAM Translation Engine

`--fit-target = total_vram_mb - user_allocation_mb` (clamped to ≥256). Uses `-ngl auto` (not 999) so the `--fit` engine can dynamically reduce layer count to fit within the VRAM budget.

---

## llama-server Flag System

42 launch flags across categories: Common (sampling), Sampling-Extended, Context & Batch, Performance, KV Cache, Server Config, RoPE, Reasoning, Embedding-Specific. All default to "auto" (omitted from launch command). Only explicitly set values are emitted.

---

## Known Deviations from Original MVP Spec

1. `model_settings` has 42 columns (original spec: 7) — intentional expansion for full flag coverage
2. `models_metadata` has `role` and `pooling_type` columns for auto-detection
3. Hardware scan cached (not re-run every startup) — checks `cpu_physical_cores > 0`
4. `master_port` defaults to 52715 (spec said "high-numbered" without specifying)
5. Uses `log` crate only (spec mentioned `tracing`/`log`)
6. Process spawning via `tokio::process::Command` (not `tauri-plugin-shell` sidecar API)
7. Frontend is pure JavaScript (spec said "Svelte" without specifying TypeScript)
8. `--flash-attn` defaults to `on` (spec said `off`)
