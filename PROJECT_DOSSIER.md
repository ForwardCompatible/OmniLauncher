# OmniLauncher — Comprehensive Project Context Dossier

*Generated 2026-07-14. Definitive technical reference for AI collaborator initialization.*

---

## Table of Contents

1. [Architecture & Stack Overview](#1-architecture--stack-overview)
2. [Structural Map](#2-structural-map)
3. [Refactor & Optimization Status](#3-refactor--optimization-status)
4. [Known Technical Debt & Dependencies](#4-known-technical-debt--dependencies)
5. [Project Rules & Logic](#5-project-rules--logic-from-agentsmd)
6. [Deviations from AGENTS.md](#6-deviations-from-agentsmd)

---

## 1. Architecture & Stack Overview

### Core Technologies

| Layer | Technology | Version |
|-------|-----------|---------|
| Desktop framework | Tauri v2 | 2.11 |
| Backend language | Rust | 1.95+ (edition 2021) |
| Frontend framework | Svelte 5 (runes mode) | ^5.0.0 |
| Frontend bundler | Vite | ^5.4.0 |
| Frontend language | **Pure JavaScript** (no TypeScript — fully purged) | ES modules |
| Database | SQLite (WAL mode, via rusqlite 0.38 + deadpool-sqlite 0.13) | bundled SQLite |
| Async runtime | Tokio | 1.52 (`features = ["full"]`) |
| Reverse proxy | axum 0.8 + reqwest 0.13 (`default-features=false, features=["http2","stream","json"]`) | |
| GPU detection | nvml-wrapper 0.12 | |
| System info | sysinfo 0.39 | |
| Logging | tauri-plugin-log 2.8 + log 0.4 (NOT tracing) | |
| Process signals | libc 0.2 | |
| Error handling | anyhow 1.0 (thiserror 2.0 is in Cargo.toml but UNUSED) | |
| Linux binary | llama-server (ai-dock CUDA build b9893, CUDA 12.8.1) | bundled as Tauri resource |
| Windows binary | llama-server (official ggml-org/llama.cpp b9821, CUDA 12.4) | downloaded to AppData at setup |

### Full Cargo.toml Dependency Listing

**Dependencies:**
- `tauri = "2.11"` (features = [])
- `tauri-plugin-log = "2.8"`
- `log = "0.4"`
- `serde = { version = "1.0", features = ["derive"] }`
- `serde_json = "1.0"`
- `rusqlite = { version = "0.38", features = ["bundled"] }` — pinned to match deadpool-sqlite 0.13's requirement
- `deadpool-sqlite = "0.13"`
- `anyhow = "1.0"`
- `thiserror = "2.0"` — **declared but unused**; all error handling is `anyhow`
- `dirs = "6.0"`
- `nvml-wrapper = "0.12"` — dynamically loads `libnvidia-ml.so.1` from the driver
- `sysinfo = "0.39"`
- `tokio = { version = "1.52", features = ["full"] }`
- `axum = "0.8"`
- `reqwest = { version = "0.13", default-features = false, features = ["http2", "stream", "json"] }`
- `libc = "0.2"` — used by SidecarController for kill() syscalls (Linux only)

**Dev-dependencies:**
- `async-stream = "0.3"`
- `serde_json = "1.0"`
- `libc = "0.2"`

**Build-dependencies:**
- `tauri-build = { version = "2.6", features = [] }`

### package.json Dependencies

**devDependencies:**
- `@sveltejs/vite-plugin-svelte: ^4.0.0`
- `@tauri-apps/cli: ^2.0.0`
- `svelte: ^5.0.0`
- `vite: ^5.4.0`

**dependencies:**
- `@tauri-apps/api: ^2.0.0`
- `@tauri-apps/plugin-log: ^2.0.0`

No TypeScript packages. No `tsconfig.json`. No `.ts` files.

### Mandatory Clean Code Rules (Enforced — Non-Negotiable)

1. **No Redundant Validation** — Rust validates inputs; the frontend handles errors returned by commands. It does not duplicate validation logic.
2. **Centralized API Layer** — All `invoke()` calls live in `src/lib/commands.js`. No component calls `invoke()` directly.
3. **Infrastructure Encapsulation** — All process management, system scanning, or disk I/O must go through internal Managers (e.g., `SidecarController`). No raw `std` or `tokio` process calls in command handlers.
4. **JSDoc Governance** — Any data structure crossing the FFI boundary must be defined as a `@typedef` in `src/lib/types.js`. Do not define inline objects in components.
5. **Pure JavaScript** — The frontend must remain pure JavaScript Svelte 5 via Vite. No TypeScript files, configs, or `<script lang="ts">` tags.
6. **Flag Dictionary Separation** — The `flag_dictionary` table (System.db) feeds ONLY tooltip text for info icons. It is NOT used for configuration rendering. The `model_settings` table (ModelRegistry.db) drives all configurable flags.

### tauri.conf.json Key Configuration

- `productName`: `"OmniLauncher"`, `identifier`: `"com.omnilauncher.app"`
- Window: 1280x800, min 900x600, resizable, label `"main"`
- `security.csp`: `null` (disabled)
- `bundle.resources`: `["resources/llama-server"]` — bundles the binary + .so libs
- `bundle.linux.deb.depends`: `["libnccl2"]` — NCCL runtime requirement
- `build.devUrl`: `http://localhost:5173`, `build.frontendDist`: `../dist`
- Capabilities: `core:default`, `log:default` for window `"main"`

---

## 2. Structural Map

### Full Directory Tree

```
OmniLauncher/
├── AGENTS.md                          # Original project specification (238 lines)
├── PROJECT_DOSSIER.md                 # This file
├── package.json                       # Svelte 5 + Vite, no TS deps
├── vite.config.js                     # Vite config (pure JS)
├── svelte.config.js                   # Svelte preprocess (vitePreprocess)
├── index.html                         # Mounts /src/main.js
├── models/                            # Symlinked to app-data dir in dev
│   ├── .gitkeep
│   ├── Qwen3.5-9B.Q5_K_S.gguf        # Chat model (5.9 GB, Q5_K_S)
│   ├── Qwen3-Embedding-0.6B-f16.gguf  # Embedding model (1.2 GB, F16)
│   └── multimodal/
│       ├── .gitkeep
│       └── gemma-4-26B-A4B-it/        # Vision model (excluded from chat scan)
│           ├── gemma-4-26B-A4B-it-UD-Q5_K_M.gguf
│           └── mmproj-F16.gguf
├── src/                               # Frontend (pure JS, ~1,178 lines total)
│   ├── main.js                        # 9 lines — mounts AppShell to #app
│   ├── AppShell.svelte                # 89 lines — shell: header, NavRail, footer, page router
│   ├── app.css                        # Dark theme via CSS custom properties
│   ├── lib/
│   │   ├── types.js                   # 149 lines — 11 @typedef declarations + CACHE_TYPES constant
│   │   ├── commands.js                # 101 lines — 13 invoke() wrappers (the API layer)
│   │   └── stores.svelte.js           # 84 lines — Svelte 5 module-level $state runes
│   ├── components/
│   │   ├── ModelCard.svelte           # 534 lines — the reusable model card (OVER 400-line target)
│   │   └── NavRail.svelte             # 37 lines — collapsible left navigation rail
│   └── pages/
│       ├── Loader.svelte              # 17 lines — two side-by-side ModelCards (chat | embedding)
│       └── Settings.svelte            # 138 lines — paths, network, hardware rescan
└── src-tauri/                         # Rust backend (~4,755 lines total)
    ├── Cargo.toml
    ├── Cargo.lock
    ├── tauri.conf.json
    ├── build.rs                       # 3 lines — tauri_build::build()
    ├── capabilities/default.json      # Permissions: core:default, log:default
    ├── gen/schemas/                   # Auto-generated Tauri schemas
    ├── icons/                         # App icons (placeholder RGBA PNGs)
    ├── resources/llama-server/        # Bundled CUDA binary + .so libs
    │   ├── llama-server               # 14 KB thin launcher binary
    │   ├── libggml-cuda.so.0.15.3     # ~167 MB — the actual CUDA kernels
    │   ├── libggml-base.so.0.15.3
    │   ├── libggml-cpu.so.0.15.3
    │   ├── libllama-server-impl.so
    │   ├── libllama-common.so.0.0.1
    │   ├── libllama.so.0.0.1
    │   ├── libmtmd.so.0.0.1
    │   ├── VERSION.txt                # Build provenance
    │   └── PROVENANCE.txt             # Download source, SHA-256, CUDA version
    ├── src/
    │   ├── main.rs                    # 24 lines — thin entry, WebKit compositing fix
    │   ├── lib.rs                     # 152 lines — crate root: run(), setup(), 17 commands
    │   ├── gguf.rs                    # 543 lines — hand-rolled GGUF header parser
    │   ├── hardware.rs                # 257 lines — NVML + sysinfo hardware scan
    │   ├── logging.rs                 # 26 lines — tauri-plugin-log config
    │   ├── paths.rs                   # 121 lines — app-data dir + models symlink bootstrap
    │   ├── process.rs                 # 580 lines — VRAM Translation Engine (pure logic)
    │   ├── proxy.rs                   # 389 lines — axum reverse proxy, SSE streaming
    │   ├── registry.rs                # 264 lines — filesystem scan + GGUF parse
    │   ├── sidecar.rs                 # 457 lines — encapsulated SidecarController
    │   ├── commands/                  # Tauri command bridge (7 submodules)
    │   │   ├── mod.rs                 # 20 lines — module declarations + ping()
    │   │   ├── app_settings.rs        # 59 lines — get/save app settings
    │   │   ├── flags.rs               # 46 lines — get_flag_dictionary
    │   │   ├── hardware.rs            # 74 lines — get/rescan hardware
    │   │   ├── models.rs              # 102 lines — role/settings commands
    │   │   ├── process.rs             # 106 lines — launch/stop/status (thin wrappers over SidecarController)
    │   │   ├── proxy.rs               # 85 lines — get_proxy_status/set_routing
    │   │   └── registry.rs            # 134 lines — get_models/resync_registry
    │   └── db/                        # SQLite layer
    │       ├── mod.rs                 # 236 lines — facade: DbPools, migrations, load/save helpers
    │       ├── pool.rs                # 75 lines — deadpool-sqlite pool, WAL pragmas
    │       ├── registry_ops.rs        # 511 lines — ModelSettings (42 fields), CRUD, reconcile
    │       ├── registry_schema.rs     # 179 lines — DDL for models_metadata + model_settings
    │       ├── seed.rs                # 365 lines — flag_dictionary seed (~40 entries)
    │       └── system_schema.rs       # 80 lines — DDL for app_settings, hardware_profile, flag_dictionary
    └── tests/
        ├── proxy_integration.rs       # 4 tests (proxy forwarding, 503, 404, SSE)
        ├── real_launch.rs             # 1 ignored test (real embedding round-trip)
        └── spawn_check.rs             # 4 tests (binary version, no orphan, controller state)
```

### Rust Module Dependency Graph

```
main.rs ─► lib (omnilauncher_lib)
lib.rs ─► db, sidecar, process, proxy, hardware, paths, registry, gguf, logging, commands
hardware ─► db::HardwareProfileRow
process ─► db::registry_ops, hardware
registry ─► gguf
sidecar ─► process, proxy
commands::process      ─► db, hardware, process, proxy, sidecar
commands::models       ─► db, hardware, process
commands::registry     ─► db, registry
commands::hardware     ─► db, hardware
commands::proxy        ─► db, proxy
commands::app_settings ─► db
commands::flags        ─► db
db::registry_ops       ─► registry::ModelRecord
```

No circular dependencies exist. Leaf modules (no internal `use crate::` imports): `gguf`, `logging`, `paths`, `db::pool`, `db::registry_schema`, `db::system_schema`, `db::seed`.

### Module Visibility

- **`pub mod`** (accessible from tests/external): `db`, `hardware`, `process`, `proxy`, `sidecar`
- **`mod`** (private to crate): `commands`, `gguf`, `logging`, `paths`, `registry`
- `sidecar` was made `pub` to allow integration test access

### Registered Tauri Commands (17)

From `lib.rs` `invoke_handler`:
1. `ping` — smoke test
2. `get_app_settings` — reads full app_settings row
3. `save_app_settings_cmd` — partial update of app_settings
4. `get_flag_dictionary` — reads tooltip data
5. `get_hardware_profile` — reads cached hardware_profile
6. `rescan_hardware` — runs live scan, persists, emits `hardware-updated`
7. `get_models` — lists models_metadata + has_settings flag
8. `resync_registry` — rescans models dir, reconciles, emits `registry-updated`
9. `get_proxy_status` — master_port + live chat/embedding ports
10. `set_routing` — dev/test hook for proxy routing
11. `set_model_role` — sets role tag on a model
12. `get_model_settings` — returns saved or computed-default settings
13. `save_model_settings` — persists per-model launch flags
14. `launch_model` — builds args, starts via SidecarController
15. `stop_model` — stops via SidecarController
16. `get_process_status` — lists running processes

### setup() Sequence (lib.rs setup function)

1. Resolve app-data dir + ensure models layout (symlinks to project tree in dev)
2. Open + migrate + seed both databases (WAL mode, deadpool-sqlite pool)
3. Persist resolved models/multimodal paths to `app_settings`
4. Hardware scan (NVML + sysinfo) -> persist to `hardware_profile`
5. Registry sync (scan `models/`, parse GGUF headers, upsert, remove stale)
6. Start reverse proxy (axum, master port from DB, auto-increment)
7. Register `SidecarController` as managed state
8. Register `DbPools` as managed state

### Tauri Events (Backend -> Frontend)

- `process-terminated` — emitted by `sidecar.rs::watch_exit()` when a child exits
- `hardware-updated` — emitted by `commands::hardware::rescan_hardware()`
- `registry-updated` — emitted by `commands::registry::resync_registry()`

All three are listened for in `AppShell.svelte::onMount()`.

---

## 3. Refactor & Optimization Status

### Completed Refactors

| Refactor | Description | Status |
|----------|-------------|--------|
| TypeScript -> Pure JavaScript | All `.ts` files renamed to `.js`, all `lang="ts"` removed, types converted to JSDoc `@typedef` | COMPLETE |
| `Supervisor` -> `SidecarController` | Old process supervisor deleted; new encapsulated controller with `start`/`stop`/`status`/`shutdown_all`/`shutdown_all_blocking` | COMPLETE |
| `concat!` binary path -> `resource_dir()` | Replaced hardcoded path macro with Tauri's `app.path().resource_dir()` | COMPLETE |
| `block_on` deadlock in exit handler | Replaced `block_on(controller.shutdown_all())` with synchronous `shutdown_all_blocking()` using `try_lock` | COMPLETE |
| XSS via `{@html}` | All 21 `{@html infoHtml(...)}` calls replaced with safe `<span title={...}>` elements | COMPLETE |
| `ModelSettings::default()` derive | Added `#[derive(Default)]`; `compute_default_settings` and test fixtures use `..ModelSettings::default()` | COMPLETE |
| Dead code purge | Removed: `proxy::serve()`, `scan_models()`, stale `#[allow(dead_code)]` annotations, `ReconcileReport.unchanged`, `AppSettingsRow`/`load_app_settings` overlap, `ping()`, `setModelRole()`, `isModelRunning` | COMPLETE |
| OS-aware scaffolding | `#[cfg(target_os = "linux")]` blocks gate binary resolution, command building, and signal sending. Windows/macOS produce `compile_error!` | COMPLETE |

### Build & Test Status

- **Rust**: `cargo build` — zero compiler warnings. `cargo test` — **55 tests pass** (46 unit + 4 proxy integration + 4 spawn check + 1 ignored real_launch)
- **Frontend**: `npm run build` — clean (18 a11y warnings from `vite-plugin-svelte`, no errors)

### In-Flight / Incomplete Work

1. **~20 Advanced flag controls not rendered in UI**: The `ModelSettings` struct and form `$state` have 42 fields. The backend's `build_args()` correctly emits all of them. But `ModelCard.svelte` only renders ~22 of them as interactive UI controls. The remaining ~20 fields (extended sampling: presence/frequency penalty, mirostat, dry sampling; server config: parallel/cont-batching/timeout; RoPE: rope-scaling/rope-freq-base; reasoning: reasoning-format) exist in the form state and round-trip through save/load without loss, but have no visible UI control. A user cannot currently set these from the frontend.

2. **`ModelCard.svelte` is 534 lines** — over the 400-line target. Extraction candidates identified but not executed:
   - Sampling grid section -> separate component
   - Advanced panel sub-sections -> separate components
   - A reusable `<FlagField>` component for the ~18 repeated label-row + info-icon blocks

3. **`ModelCard.svelte:247` latent bug**: A `{#if "string literal"}` condition that is always truthy instead of a proper tooltip lookup — leftover from the XSS remediation refactor where `{@html infoHtml()}` was replaced. The info-icon for CPU Mode always renders with a hardcoded title rather than reading from the flag dictionary.

---

## 4. Known Technical Debt & Pending Items

### Unresolved Audit Items (Ranked by Severity)

| # | Item | Severity | Location | Description |
|---|------|----------|----------|-------------|
| 1 | `gpu_present` heuristic duplicated 3x | **Medium** | `commands/process.rs:96`, `commands/models.rs:78`, `commands/hardware.rs:28` | The string sniff `starts_with("CPU-only")` is copy-pasted in three command files instead of being a `From<HardwareProfileRow> for HardwareProfile` impl. Fragile — depends on the exact prefix the hardware scanner writes. |
| 2 | Raw SQL in command layer | **Medium** | `commands/flags.rs:22`, `commands/registry.rs:42-59`, `commands/registry.rs:109-133` | Three command handlers bypass the `db::` layer with inline `conn.interact()` calls. Should go through `registry_ops` or dedicated DB helpers. |
| 3 | `ModelCard.svelte:247` always-truthy `{#if}` | **Bug** | `ModelCard.svelte:247` | `{#if "Forces --n-gpu-layers 0..."}` uses a string literal as condition (always true). Should be a tooltip lookup. |
| 4 | `unsafe` block in `gguf.rs` is avoidable | **Low** | `gguf.rs:283` | `unsafe { std::slice::from_raw_parts(buf.as_ptr(), n) }` reconstructs a slice that `Cursor::fill_buf()` already returns safely. |
| 5 | `_routing_type_check` dead stub | **Low** | `tests/proxy_integration.rs:178` | Sentinel function to suppress unused-import warning. Should just drop the import. |
| 6 | Hardcoded `/home/ryan/` path | **Low** | `tests/real_launch.rs:24` | Developer-specific absolute path to model file. Should use env var or app-data resolution. |
| 7 | `proxy_streams_sse_unbuffered` doesn't test timing | **Low** | `tests/proxy_integration.rs:113` | Test name claims to verify unbuffered streaming but only checks that all chunks arrive (a buffered proxy would pass). |
| 8 | `handler_404_returns_not_found` tests a stub | **Low** | `proxy.rs:330` | Calls a local `handler_404_sync()` helper, not the real async `handler_404()`. Tautological. |
| 9 | `thiserror = "2.0"` unused dependency | **Low** | `Cargo.toml` | All error handling uses `anyhow`. No `#[derive(thiserror::Error)]` exists anywhere. |
| 10 | 16x a11y label warnings | **Low** | `ModelCard.svelte` (throughout) | `<label>` elements in `.label-row` divs lack `for=` association with their inputs. |

### Architecture Decisions of Note

- **`ModelSettings` has 42 `Option<T>` fields** — one per launch flag. All default to `None` (= "auto" — flag omitted from launch command). Adding a flag requires editing: schema (CREATE TABLE + migration), struct definition, load SQL (SELECT column list), save SQL (INSERT/params vec + ON CONFLICT). The `#[derive(Default)]` means `compute_default_settings()` and test fixtures use `..ModelSettings::default()` and only override the few fields that differ.

- **`model_settings` save uses `rusqlite::params_from_iter`** — rusqlite caps tuple params at ~16, so 43 bind parameters need the iterator API rather than a tuple.

- **`flag_dictionary` is decoupled from `model_settings`** — the dictionary table (in System.db) feeds ONLY tooltip text for info icons in the UI. It is NOT used for configuration rendering or stored values. The `model_settings` table (in ModelRegistry.db) drives all configurable flags. This separation is intentional and mandated.

- **Proxy binds to `127.0.0.1` only** — no LAN exposure of the unauthenticated LLM endpoint.

- **`--port 0` + stderr parse** — the SidecarController lets the OS assign a free port, then parses the actual port from llama-server's `listening on 127.0.0.1:<port>` stderr line. This eliminates the TOCTOU race of pre-selecting a port.

- **WebKit2GTK compositing fix** — `main.rs` sets `WEBKIT_DISABLE_COMPOSITING_MODE=1` on Linux to prevent blank/black screens with NVIDIA proprietary drivers in the WebKit2GTK GPU sandbox.

- **`LD_LIBRARY_PATH` injection** — the bundled `llama-server` binary has RPATH=`$ORIGIN`, but the dynamic loader does not propagate RPATH through transitive `.so` dependencies. The SidecarController's `build_command()` (Linux-only) sets `LD_LIBRARY_PATH` to the binary's directory.

- **Models directory symlink** — in dev mode, `paths::ensure_models_layout()` symlinks `~/.local/share/com.omnilauncher.app/models` -> `<project_root>/models/` so developers can drop `.gguf` files in the visible project folder.

- **Hardware scan runs every startup** (not cached from first launch as AGENTS.md suggests). The scan overwrites the `hardware_profile` row each boot. The DB row IS the cache.

- **Auto-role detection on model insert** — models are auto-tagged based on GGUF metadata:
  - `{arch}.pooling_type` present (as UINT32 enum 0-4 or STRING) -> `'embedding'`
  - No `pooling_type` but `tokenizer.chat_template` present -> `'chat'`
  - Neither -> `NULL` (appears in both dropdowns)
  - Role is preserved across rescans (never overwritten by the upsert's ON CONFLICT clause)

- **`pooling_type` can be UINT32 or STRING in real GGUF files** — the parser's `get_pooling_type()` handles both: UINT32 enum values (0=none, 1=mean, 2=cls, 3=last, 4=rank) are converted to string names; STRING values are used as-is.

---

## 5. Project Rules & Logic (from AGENTS.md)

### Core Architecture Principles

1. **Target OS**: Ubuntu (Linux x86_64 only for MVP)
2. **App Base**: Rust (Backend) + Svelte (Frontend), Tauri v2 framework
3. **Core Binary**: A pinned ai-dock CUDA build of `llama-server` (Linux x86_64), bundled directly in the project structure. On Windows, the official ggml-org/llama.cpp CUDA build (tag b9821, CUDA 12.4) is used instead — same CLI surface, downloaded to `%APPDATA%/com.omnilauncher.app/binaries/` at setup along with the matching CUDA runtime DLLs.
4. **CUDA binary used universally** — the bundled `llama-server` is always CUDA-enabled
5. **GPU Present**: Memory management uses native llama.cpp hybrid mode parameter fitting (`--fit`, `--fit-target`) to dynamically allocate layers based on user-defined VRAM budgets for both chat and embedding models
6. **CPU-Only (Zero VRAM)**: If the hardware scan detects no usable VRAM, Rust acts as a safety valve. It forces `--n-gpu-layers 0` and explicitly strips all parameter-fitting flags to prevent CUDA initialization panics, allowing a safe default to system RAM
7. **No external dependencies** not strictly required for the MVP
8. **All SQLite operations** must use `rusqlite` with connection pooling or dedicated async handles to support WAL mode
9. **Utilize `tauri::command`** for all backend-to-frontend communication

### VRAM Translation Engine

Rust subtracts the user's UI VRAM allocation from the system's total VRAM to calculate a "free margin," passing this to `llama-server` via the `--fit-target` flag to natively maximize hybrid offload without OOM crashes.

**Implementation**: `margin = (hw.total_vram_mb - settings.vram_allocation_mb).max(256)`. The 256 floor prevents passing 0 (which would mean "use all VRAM" and risk OOM from CUDA context overhead).

### Database Architecture

- **Engine**: SQLite with WAL (Write-Ahead Logging). Functions as the direct, lock-free state interface layer between the Rust backend and Svelte frontend.
- **Two databases**:
  - `System.db` — application settings, hardware profile, flag dictionary
  - `ModelRegistry.db` — immutable GGUF model metadata + mutable per-model launch flags
- **WAL mode** on both — per-connection pragmas applied in deadpool's `post_create` hook: `foreign_keys=ON`, `journal_mode=WAL`, `synchronous=NORMAL`, `busy_timeout=5000`
- **`flag_dictionary`** is explicitly decoupled from dynamic UI component rendering — it strictly populates tooltip text for info icons
- **`model_settings`** is directly bound to static UI components on the Loader page, updated via the Save button

### Process Lifecycle

- Processes start stopped (never auto-launch on app boot)
- Child processes die strictly with the parent application
- `kill_on_drop(true)` set on every spawn
- `process-terminated` Tauri event emitted to frontend on crash/exit
- `RunEvent::Exit` handler calls `SidecarController::shutdown_all_blocking()` (synchronous, `try_lock`, SIGTERM+SIGKILL)

### API Traffic Routing

- **Single master port** (default 52715) — all external tools connect here
- **Path-based routing**:
  - `/v1/chat/completions` -> dynamically assigned Chat Model port
  - `/v1/embeddings` -> dynamically assigned Embedding Model port
- **503 with OpenAI error shape** when no backend is routed to a path
- **404** on unknown paths
- **SSE streaming pass-through** — responses forwarded chunk-by-chunk via `reqwest::bytes_stream()` -> `axum::body::Body::from_stream()`

### Port Management

- "Auto" checkbox (enabled by default in `app_settings.auto_port_increment`)
- Auto Strategy: Defaults to high-numbered base port 52715. Auto-increments by +1 if busy (up to +50 attempts). Excludes zombie-process collisions.
- Manual override possible when "Auto" is unchecked.

### Functional Requirements

- **Startup**: Registry syncs with `OmniLauncher/models`. Rust enforces creation of `/multimodal` sub-directory.
- **Metadata**: GGUF headers parsed for layer count, context length, architecture, model name, quantization, chat template, author, and pooling type.
- **API**: Exposes OpenAI-compatible API for external tools via the internal Rust reverse proxy.

### UI Layout

- **Shell**: Persistent AppShell with Header, Footer, and collapsible left NavRail (default open)
- **Loader Page**: Per-model actions, dropdown selection, launch controls, and model configuration flags. Two side-by-side cards: Chat Model and Embedding Model.
- **VRAM Allocation Slider**: User specifies exact VRAM amount to allocate
- **CPU Mode Checkbox**: Forces CPU-only safety valve, grays out VRAM slider
- **Save Settings Button**: Per-card, writes to `model_settings` table
- **Settings Page**: Global configurations (folder paths, hardware profile, manual rescan triggers, ports)

### llama-server Flag Dictionary & Tooltip Seeding

The flag dictionary data is explicitly decoupled from dynamic UI component rendering. It strictly populates the `flag_dictionary` table in `System.db` to feed informational UI hover tooltips. The mutable launch flags live in `model_settings` and are bound to hardcoded UI components.

**Original AGENTS.md documents 13 flags in 4 categories.** The implementation expanded this to ~40 flags across 8 categories (see Deviation 6.2).

---

## 6. Deviations from AGENTS.md

### 6.1 Schema Expansions (Intentional — Extended Beyond MVP Spec)

**`model_settings` table — 7 columns in AGENTS.md, 42 in implementation:**

AGENTS.md specifies: `vram_allocation_mb`, `ctx_size`, `batch_size`, `ubatch_size`, `flash_attn`, `cache_type_k`, `cache_type_v`.

Implementation adds 35 additional nullable columns:
- `cpu_mode`, `mlock`, `no_mmap`, `threads`, `threads_batch` (performance flags)
- `cache_prompt` (KV cache control)
- 19 sampling parameters (`temp`, `top_k`, `top_p`, `min_p`, `repeat_penalty`, `repeat_last_n`, `seed`, `presence_penalty`, `frequency_penalty`, `typical_p`, `xtc_probability`, `xtc_threshold`, `mirostat`, `mirostat_lr`, `mirostat_ent`, `dry_multiplier`, `dry_base`, `dry_allowed_length`)
- `predict`, `context_shift`, `parallel`, `cont_batching`, `timeout` (context/server config)
- `rope_scaling`, `rope_freq_base` (RoPE context extension)
- `reasoning_format` (reasoning model support)
- `pooling_type_override`, `embd_normalize`, `rerank` (embedding-specific)

All new columns are nullable and default to `None` (= "auto" — flag omitted from launch command). The AGENTS.md-specified columns remain as-is.

**`models_metadata` table — 11 columns in AGENTS.md, 13 in implementation:**

Implementation adds:
- `role TEXT` — auto-detected chat/embedding tag (NULL/chat/embedding)
- `pooling_type TEXT` — raw `{arch}.pooling_type` from the GGUF header (present on embedding models, absent on chat models)

### 6.2 flag_dictionary Seed Expansion (Intentional)

AGENTS.md documents 13 flags across 4 categories (Core Execution & Hardware, Context & Memory Limits, Performance & Optimization, Advanced KV Cache Quantization). The implementation seeds ~40 flags across 8 categories, adding: Common (sampling), Sampling - Extended, Context & Batch, Server Config, RoPE / Context Extension, Reasoning, Embedding-Specific. The dictionary still strictly serves tooltip text only — the expansion does not violate the separation rule.

### 6.3 `model_settings` "Updated Automatically on Startup" (Deviation)

AGENTS.md line 123 states: *"Directly bound to static UI components. Updated automatically on startup by checking OmniLauncher/models directory."*

**Implementation deviates**: `model_settings` rows are NOT auto-created during the startup registry scan. Instead, settings are computed lazily at launch time via `compute_default_settings()` when no saved row exists. A row is only created when the user explicitly clicks "Save Settings" in the UI. This was a deliberate design choice to avoid polluting the table with default rows the user never customized.

### 6.4 `master_port` Default Value (Deviation — Unspecified in AGENTS.md)

AGENTS.md says "high-numbered base ports (tuned for Ubuntu)" but gives no specific number. The implementation defaults to **52715** (seeded in the `app_settings` singleton at schema creation). The original schema seed was `0` (meaning "OS-assigned random"), which was changed to `52715` during Layer 4 (Reverse Proxy) development to provide a stable, known port for external tools.

### 6.5 Logging: `tracing` vs `log` (Deviation)

AGENTS.md line 159 states: *"Utilizes Rust crates (`tracing`/`log`)"*. The implementation uses **only** the `log` crate facade via `tauri-plugin-log` v2. The `tracing` crate is not a dependency. This was a deliberate choice — `tauri-plugin-log` is built on `fern` and the `log` facade, and `tracing`'s structured spans were not needed for the MVP.

### 6.6 `theme` Column Exists but No Theme Switching UI (Incomplete)

AGENTS.md specifies a `theme` TEXT column in `app_settings`. The column exists in the schema (defaults to `'dark'`) and is included in the `AppSettings`/`FullAppSettings` structs, but there is no UI control to change it. The frontend uses hardcoded dark theme via CSS custom properties. Light theme support is deferred.

### 6.7 Process Spawning Method (Deviation — Justified)

AGENTS.md does not specify the process spawning mechanism. The original Layer 1 plan proposed Tauri v2's `tauri-plugin-shell` sidecar API (`app.shell().sidecar()`). The implementation uses `tokio::process::Command` encapsulated within `SidecarController` instead.

**Justification**: the shell plugin's sidecar API doesn't expose the raw `Child` handle or allow reading stderr line-by-line, which is required for the dynamic port parsing (`listening on 127.0.0.1:<port>`) and the exit-watcher pattern. The `tokio::process::Command` approach is fully encapsulated — no command handler touches tokio types directly. The `SidecarController` exposes a clean `start`/`stop`/`status`/`shutdown_all` interface.

### 6.8 Frontend Language (Deviation — Corrected)

AGENTS.md says "Rust (Backend) + Svelte (Frontend)" with no mention of TypeScript. The implementation initially used TypeScript across the entire frontend, which was a unilateral deviation from the project spec. This was **fully purged** in a complete refactor — the frontend is now pure JavaScript with JSDoc `@typedef` annotations. Zero TypeScript artifacts remain (no `.ts` files, no `tsconfig.json`, no TS dependencies, no `lang="ts"`).

### 6.9 `--flash-attn` Default Value Discrepancy (Spec vs Binary vs Implementation)

| Source | Default |
|--------|---------|
| AGENTS.md flag dictionary | `off` |
| llama-server `--help` output | `auto` |
| Implementation (`compute_default_settings`) | `Some(true)` -> emits `--flash-attn on` |

The implementation deliberately enables flash attention by default (overriding both the spec and the binary default), as it improves performance and reduces memory footprint for long contexts. The flag_dictionary tooltip still says `off` (matching AGENTS.md), creating a minor tooltip/behavior mismatch.

### 6.10 Database Naming (Deviation — Cosmetic)

AGENTS.md refers to "System Database (System.db)" and "Model Registry Database" without naming the second file. The implementation names it `ModelRegistry.db`. This is a cosmetic addition, not a functional deviation.

### 6.11 Hardware Scan Caching (Deviation — Lazy vs Cached)

AGENTS.md line 189 states: *"Results cached on first launch; manual rescan on Settings page triggers both hardware update and registry refresh."*

**Implementation deviates**: The hardware scan runs **every startup** (not cached from first launch). The scan overwrites the `hardware_profile` row each boot. The "manual rescan" button on the Settings page re-runs the same scan. The DB row IS the cache — it's always fresh because the scan runs at boot. This is arguably better behavior than the spec (always detects hardware changes like new GPUs), but it is a deviation from the documented "cache on first launch" approach.

### 6.12 Pooling Type Field Type (Implementation Detail)

The GGUF spec does not mandate the type of `{arch}.pooling_type`. In practice, real models store it as either a STRING (e.g. `"mean"`) or a UINT32 enum (0=none, 1=mean, 2=cls, 3=last, 4=rank). The implementation's `get_pooling_type()` parser handles both formats and normalizes to a string name. This is not a deviation from AGENTS.md (which doesn't mention pooling_type at all) but is a technical detail a collaborator would need to know.

---

*End of Dossier*
