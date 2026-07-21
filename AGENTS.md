# OmniLauncher — Architectural Guide

> **Purpose:** This document is the source of truth for OmniLauncher's architecture and the teachable patterns for extending it. It serves two jobs: (1) accurately describe what exists today, and (2) guide future development — new pages, new commands, new model features.

## 1. Overview & Core Architecture

**Target OS:** Ubuntu Linux (primary) and Windows Native (secondary).

**App Base:** Rust (Backend) + Svelte 5 (Frontend, pure JavaScript).

**Framework:** Tauri v2.

**Core Binary:** CUDA-accelerated `llama-server` (x86_64). Source differs per OS — each ships the same CLI surface, only the build origin differs.
- **Linux:** A pinned ai-dock CUDA build (`b9893`, CUDA 12.8.1). Bundled directly in the project structure as a Tauri resource. Includes sibling `.so` libraries.
- **Windows:** The official ggml-org/llama.cpp CUDA build (`b9821`, CUDA 12.4). Downloaded during installation to `%APPDATA%/com.omnilauncher.app/binaries/` along with the matching CUDA 12.4 runtime DLLs (`cudart64_12.dll`, `cublas64_12.dll`, ...). Not bundled in the installer.

### Execution & Hardware Branching

CUDA binary used universally. The branch is driven by `HardwareProfileRow::has_usable_gpu()` (the single source of truth — see §4).

**GPU Present:** Hybrid mode via `--fit on` + `-ngl auto` + `--fit-target <margin>`. The fit engine dynamically allocates layers between GPU VRAM and system RAM.

**CPU-Only (Zero VRAM):** Safety valve forces `-ngl 0` and `--fit off`, stripping all parameter-fitting flags to prevent CUDA initialization panics.

---

## 2. Project Structure

```
OmniLauncher/
├── src-tauri/                          # Rust backend
│   ├── src/
│   │   ├── main.rs                     # Entry bin; sets WEBKIT_DISABLE_COMPOSITING_MODE on Linux
│   │   ├── lib.rs                      # Crate root; setup(), 25 commands, RunEvent::Exit handler
│   │   ├── sidecar.rs                  # Encapsulated process controller (start/stop/status/shutdown_all)
│   │   ├── process.rs                  # VRAM Translation Engine (build_args, compute_default_settings)
│   │   ├── proxy.rs                    # axum reverse proxy with SSE streaming
│   │   ├── gguf.rs                     # Hand-rolled GGUF v1/v2/v3 header parser
│   │   ├── hardware.rs                 # NVML + sysinfo detection
│   │   ├── registry.rs                 # Model file discovery + GGUF scan pipeline
│   │   ├── paths.rs                    # app_data_dir resolution + dev-mode symlinks
│   │   ├── logging.rs                  # tauri-plugin-log config (Stdout + Webview + LogDir)
│   │   ├── commands/                   # Tauri command bridge (8 files, one per domain)
│   │   │   ├── mod.rs                  #   module declarations
│   │   │   ├── app_settings.rs         #   get_app_settings, save_app_settings_cmd
│   │   │   ├── flags.rs                #   get_flag_dictionary
│   │   │   ├── hardware.rs             #   get_hardware_profile, rescan_hardware
│   │   │   ├── huggingface.rs          #   HF Hub commands (search, list, download, readme)
│   │   │   ├── models.rs               #   get/save_model_settings
│   │   │   ├── process.rs              #   launch_model, stop_model, get_process_status
│   │   │   ├── proxy.rs                #   get_proxy_status
│   │   │   └── registry.rs             #   get_models, resync_registry
│   │   ├── hf_auth.rs                  # HuggingFace OAuth device-code + keychain credentials
│   │   ├── huggingface.rs              # HuggingFace Hub client (search, list, download, readme)
│   │   ├── download_manager.rs         # In-flight download tracking + cancellation
│   │   └── db/                         # SQLite layer
│   │       ├── mod.rs                  #   DbPools facade + DTOs + has_usable_gpu()
│   │       ├── pool.rs                 #   deadpool-sqlite pool, WAL pragmas
│   │       ├── system_schema.rs        #   System.db DDL + seed app_settings/hardware_profile
│   │       ├── registry_schema.rs      #   ModelRegistry.db DDL + add_column_if_absent migrations
│   │       ├── registry_ops.rs         #   ModelSettings struct (42 fields) + CRUD
│   │       └── seed.rs                 #   flag_dictionary seed (44 entries, 11 categories)
│   ├── resources/llama-server/         # Bundled Linux binary (.gitkeep'd; populated by setup.sh)
│   ├── Cargo.toml
│   └── tauri.conf.json
├── src/                                # Svelte 5 frontend (pure JS)
│   ├── main.js                         # mount() entry; imports app.css
│   ├── AppShell.svelte                 # Top-level shell + page switcher + event listeners
│   ├── app.css                         # Global stylesheet; CSS custom properties on :root (dark theme)
│   ├── pages/
│   │   ├── Loader.svelte               # Loader page wrapper (renders both model cards)
│   │   ├── Models.svelte               # HuggingFace browser + local model library page
│   │   └── Settings.svelte             # Settings page
│   ├── components/
│   │   ├── NavRail.svelte              # Left navigation rail (hardcoded buttons)
│   │   └── ModelCard.svelte            # Unified chat+embedding card (parameterized by role)
│   └── lib/
│       ├── commands.js                 # Centralized invoke() wrappers (13 functions)
│       ├── format.js                   # Shared formatting helpers (fmtBytes/fmtMiB)
│       ├── stores.svelte.js            # Module-level $state singletons + refresh functions
│       └── types.js                    # JSDoc @typedef declarations (12 types) + CACHE_TYPES
├── models/                             # User-supplied .gguf files (.gitkeep'd)
├── setup.sh                            # Linux setup (ai-dock binary download + patchelf)
├── setup.ps1                           # Windows setup (ggml-org binary + cudart download)
└── AGENTS.md                           # This document
```

---

## 3. Extension Guides

These are the three load-bearing patterns for extending OmniLauncher. Each lists the exact files to touch.

### 3.1 Adding a Page

There is **no router and no SvelteKit.** Navigation is a single `$state` string (`page.current` in `stores.svelte.js`) switched by a literal `{#if}`/`{:else if}` block in `AppShell.svelte`. This keeps the bundle tiny and avoids router abstraction — but adding a page requires touching four files:

1. **`src/lib/types.js`** — extend the `PageId` typedef union:
   ```js
   /** @typedef {"loader" | "settings" | "models"} PageId */
   ```
2. **`src/AppShell.svelte`** — add an import at the top, then add a branch to the page switch (~line 84):
   ```svelte
   {:else if page.current === "browser"}
     <Browser />
   ```
3. **`src/components/NavRail.svelte`** — add a new `<button class="nav-item">` with `onclick={() => navigate("browser")}` and an SVG icon, mirroring the existing Loader/Settings buttons.
4. **Create `src/pages/Browser.svelte`** — the page component.

New pages can call any function in `src/lib/commands.js` and read reactive state from `src/lib/stores.svelte.js`. For pages that need to talk to a launched model (e.g., a local chat page using the chat model), point `fetch()` at `http://127.0.0.1:{master_port}/v1/chat/completions` — the reverse proxy is already running locally and routes to the active backend. This is what enables ideas like a HuggingFace model browser or an in-app chat experience without any new backend plumbing.

### 3.2 Adding a Tauri Command

1. **Write the command** in `src-tauri/src/commands/<area>.rs`:
   ```rust
   #[tauri::command]
   pub async fn my_command(arg: String) -> Result<MyType, String> {
       // ... implementation ...
       Ok(result).map_err(|e: anyhow::Error| e.to_string())
   }
   ```
   All commands return `Result<T, String>` (errors flattened to strings). Keep heavy logic out of the command — call into `db/`, `sidecar.rs`, or `process.rs`.
2. **Register it** in `src-tauri/src/lib.rs` inside the `generate_handler!` macro (currently 25 commands).
3. **Add the wrapper** in `src/lib/commands.js`:
   ```js
   export async function myCommand(arg) {
     return invoke("my_command", { arg });
   }
   ```
4. **(Optional) Emit an event** if the command changes state asynchronously (e.g., a long-running task completing). See §6.1 for the event system.

**Naming convention note:** command names are `snake_case` on the Rust side and `camelCase` in `commands.js`. The existing codebase has one inconsistency — `save_app_settings_cmd` carries a `_cmd` suffix that no other command uses. New commands should omit the suffix.

### 3.3 Adding a DB Column or Table

Schema changes use **idempotent migrations** via `add_column_if_absent()` in `db/registry_schema.rs` / `db/system_schema.rs`. The pattern:

1. Add the column to the `CREATE TABLE` statement (for fresh installs).
2. Add a matching `add_column_if_absent(&conn, "table", "column", "TYPE")` call (for existing installs upgrading).
3. Update the corresponding Rust struct + the `INSERT`/`SELECT` column lists.

This dual-write (CREATE + ALTER) ensures both new and existing databases converge to the same schema without a separate migration runner.

---

## 4. Database Architecture

**Engine:** SQLite with WAL mode. Per-connection pragmas applied in deadpool's `post_create` hook (`db/pool.rs`): `foreign_keys=ON`, `journal_mode=WAL`, `synchronous=NORMAL`, `busy_timeout=10000`.

Two database files, managed by `DbPools` (a pooled facade — `db/mod.rs`):

### System.db

**`app_settings`** (singleton, `id=1`) — 6 columns:

| Column | Type | Default | Description |
|--------|------|---------|-------------|
| `models_directory` | TEXT | NULL | Path to models directory |
| `multimodal_directory` | TEXT | NULL | Default: `models/multimodal` |
| `master_port` | INTEGER | 52715 | Reverse proxy port |
| `auto_port_increment` | BOOLEAN | 1 | Auto-increment port if busy (max 50 attempts) |
| `theme` | TEXT | 'dark' | **Currently unwired** — fetched but not read by frontend (see §7) |

**`hardware_profile`** (singleton, `id=1`) — 7 columns. Cached on first launch; manual rescan available on Settings page. Cache validity = `cpu_physical_cores > 0` (NOT GPU presence — a CPU-only machine is still a valid cache hit).

| Column | Type | Description |
|--------|------|-------------|
| `gpu_name` | TEXT | GPU model name (or `"CPU-only"` sentinel) |
| `total_vram_mb` | INTEGER | Total VRAM in MiB |
| `total_system_ram_mb` | INTEGER | Total system RAM in MiB |
| `cpu_physical_cores` | INTEGER | Physical CPU cores |
| `cpu_logical_threads` | INTEGER | Logical CPU threads |
| `last_scanned_at` | TIMESTAMP | Last hardware scan timestamp |

**GPU availability — single source of truth:** `HardwareProfileRow::has_usable_gpu()` in `db/mod.rs` returns `self.total_vram_mb > 0 && !self.gpu_name.starts_with("CPU-only")`. All code that branches on GPU presence must call this method — never duplicate the `starts_with("CPU-only")` heuristic.

**`flag_dictionary`** — 6 columns (`category`, `flag_name`, `cli_argument`, `default_value`, `description`). Contains **44 entries across 11 categories**. Strictly for tooltip text in the UI — decoupled from configuration. `model_settings` drives all configurable flags.

### ModelRegistry.db

**`models_metadata`** — 13 columns. Immutable GGUF header data: `filename`, `filepath`, `filesize_bytes`, `architecture`, `model_name`, `context_length`, `layer_count`, `quantization`, `chat_template`, `author`, plus two auto-detected columns:
- `role` (`'chat'` / `'embedding'` / NULL)
- `pooling_type` (from the GGUF `{arch}.pooling_type` field)

**Auto-role detection on upsert** (`db/registry_ops.rs`): when inserting a new model, `role` is set to `"embedding"` if `pooling_type` is present, `"chat"` if `chat_template` is present, else NULL. On conflict (rescan), the existing `role` is preserved (not overwritten) but `pooling_type` is refreshed. Users can manually override via `set_model_role`.

**`model_settings`** — **44 columns**: `id` (PK), `model_id` (UNIQUE FK→models_metadata, ON DELETE CASCADE), plus **42 nullable flag columns** (one per launch flag). All default to `None` (= "auto" — flag omitted from launch). The `ModelSettings` Rust struct (`db/registry_ops.rs`) uses `#[derive(Default)]` so `..ModelSettings::default()` gives an all-None baseline.

The 42 flag columns, by category: VRAM (1), Context & Batch (3), Attention (1), KV Cache (2), CPU mode (1), Binary (3), Threads (2), Sampling-Common (7), Sampling-Extended (12), Server Config (3), RoPE (2), Reasoning (1), Embedding-Specific (3), Generation (2).

---

## 5. Sidecar Controller (Process Management)

Encapsulates all `llama-server` lifecycle in `src-tauri/src/sidecar.rs`. No raw `tokio::process`/`std::process` calls exist outside this module.

**Public API (5 methods):**

| Method | Signature | Purpose |
|--------|-----------|---------|
| `start` | `async (app, proxy_state, model_id, name, role, args) -> Result<LaunchReport>` | Spawn + port detection + proxy registration |
| `stop` | `async (proxy_state, model_id) -> Result<()>` | Terminate + clear proxy routing |
| `status` | `async () -> Vec<ProcessInfo>` | List running processes |
| `shutdown_all` | `async ()` | Graceful shutdown of all processes |
| `shutdown_all_blocking` | `sync ()` | **Same as above but non-async** — used from `RunEvent::Exit` to avoid a tokio `block_on` deadlock. Uses `try_lock` (non-blocking). |

**Port detection:** `parse_listening_port` reads stdout for a line containing `"listen"` and extracts the port. Timeout is **120 seconds** — models that take longer to start listening will fail.

### Linux
- Binary: bundled as Tauri resource at `resources/llama-server/llama-server`
- Environment: `LD_LIBRARY_PATH` set to binary's directory for sibling `.so` resolution
- Termination: `libc::kill(pid, SIGTERM)` → 5s grace → `SIGKILL`

### Windows
- Binary: downloaded during install to `%APPDATA%/com.omnilauncher.app/binaries/llama-server.exe`
- Environment: binary directory prepended to `PATH` for side-by-side DLL resolution
- Spawn flags: `CREATE_NO_WINDOW` (0x08000000) to prevent console popups
- Termination: `OpenProcess` + `TerminateProcess` via `windows-sys` crate (no POSIX signals; the 5s grace period between terminate/kill is a no-op on Windows)

---

## 6. Backend Systems

### 6.1 Tauri Event System

Three backend events propagate async state changes to the frontend, where `AppShell.svelte`'s `onMount` registers listeners via `@tauri-apps/api/event`:

| Event | Emitted by | Frontend response |
|-------|-----------|-------------------|
| `process-terminated` | `sidecar.rs` (exit watcher) | `refreshProcesses()` + `refreshProxy()` |
| `hardware-updated` | `commands/hardware.rs` (post-rescan) | `refreshHardware()` |
| `registry-updated` | `commands/registry.rs` (post-resync) | `refreshModels()` |

When adding a backend feature that changes shared state asynchronously, emit a named event and add a listener — this is the established push-based update path. The pull-based path is the `refresh*()` functions in `stores.svelte.js`.

### 6.2 VRAM Translation Engine

`src-tauri/src/process.rs`. `build_args()` is a pure function (`Vec<String>` out, no I/O).

- `--fit-target = total_vram_mb - user_allocation_mb` (clamped to ≥256)
- Uses `-ngl auto` (NOT a fixed number) so the `--fit` engine can dynamically reduce layer count to fit the VRAM budget
- GPU path: `--fit on` + `-ngl auto` + `--fit-target <margin>`
- CPU path (safety valve): `-ngl 0` + `--fit off`

**`compute_default_settings()`** (`process.rs`) generates sensible defaults for un-customized models: `vram_allocation_mb = total_vram * 0.8`, `ctx_size = min(model.context_length, 4096)`, `flash_attn = Some(true)`, `cpu_mode = Some(false)`.

### 6.3 GGUF Parser

`src-tauri/src/gguf.rs` — hand-rolled, supports GGUF v1/v2/v3. No external dependency. Reads metadata KV pairs; extracts architecture, model_name, context_length, layer_count, quantization (from `general.file_type` llama_ftype enum), chat_template, author, and pooling_type. Role auto-detection (see §4) consumes these fields.

### 6.4 Dev-Mode Symlinks

`paths.rs::ensure_models_layout()` symlinks the app-data `models/` directory to `<project_root>/models/` when the project tree exists (detected via `CARGO_MANIFEST_DIR`). This lets developers drop `.gguf` files into the repo's `models/` dir and have them discovered in dev mode. Windows falls back gracefully if symlink creation requires Developer Mode.

### 6.5 Reverse Proxy

`src-tauri/src/proxy.rs` — axum-based, bound to `127.0.0.1:{master_port}`. Path-based routing to dynamically assigned backend ports:
- `POST /v1/chat/completions` → active chat backend
- `POST /v1/embeddings` → active embedding backend
- All other paths → 404 JSON

SSE streaming is pass-through via `Body::from_stream(upstream.bytes_stream())` — chunk-by-chunk, no buffering. When no backend is routed, returns HTTP 503 with an OpenAI-shaped error JSON. Port auto-increments up to 50 attempts (`MAX_PORT_INCREMENTS`) when the requested port is busy.

### 6.6 WebKit/NVIDIA Workaround

`main.rs` unconditionally sets `WEBKIT_DISABLE_COMPOSITING_MODE=1` on Linux to fix a blank-window bug with NVIDIA proprietary drivers in Tauri's WebKit2GTK webview. Affects webview painting only, not CUDA inference. If rendering issues appear on non-NVIDIA Linux hardware, this is the first place to look.

---

## 7. Frontend Architecture

### 7.1 Navigation

See §3.1 for the page-addition pattern. Summary: `page.current` (`$state` string in `stores.svelte.js`) + `navigate(p)` setter + `{#if}` switch in `AppShell.svelte` + hardcoded buttons in `NavRail.svelte`. The `PageId` typedef in `types.js` is documentation-only — nothing enforces it at runtime.

### 7.2 State (`stores.svelte.js`)

Eleven module-level `$state` singletons (shared via import):

| Export | Shape |
|--------|-------|
| `page` | `{ current: "loader" \| "settings" \| "models" }` |
| `models` | `{ list: ModelDto[] }` |
| `hardware` | `{ data: HardwareProfile \| null }` |
| `proxy` | `{ data: ProxyStatus \| null }` |
| `processes` | `{ list: RunningProcess[] }` |
| `settings` | `{ data: AppSettings \| null }` |
| `flags` | `{ map: Map<string, FlagEntry> }` (keyed by `cli_argument`) |
| `errors` | `{ items: ErrorItem[] }` |
| `hfAuth` | HF auth/credential state |
| `hfSearch` | HF Hub search query/results state |
| `hfDownloads` | In-flight + completed HF download state |

Six `refresh*()` functions fetch via `commands.js` and route failures to the error queue. `initAll()` runs all six in parallel on mount.

### 7.3 Error Queue

`errors.items` is an array of `{ message, timestamp, severity }`. All `refresh*()` failures route here — no silent swallowing. **Severity conventions:** `refreshModels` pushes `"error"` (model fetch failure is critical); all other refreshers push `"warning"`. Rendered in `AppShell.svelte` with severity-based styling and a "Clear all" button when `items.length > 1`.

### 7.4 Model Cards

`ModelCard.svelte` is the single, unified card component used for both roles. It is parameterized by a `role` prop (`"chat"` or `"embedding"`) and renders only the flags relevant to that role. The former `ChatModelCard.svelte` and `EmbeddingModelCard.svelte` were consolidated into this one component so shared flags (VRAM, ctx_size, threads, batch, cache types) now live in a single place — no more keeping two copies in sync.

### 7.5 Styling

Single global stylesheet (`app.css`), no per-component `<style>` blocks. CSS custom properties on `:root` define the palette, spacing scale, radii, and typography. `color-scheme: dark` is hardcoded.

**Known gap:** the `app_settings.theme` column exists and is fetched into `settings.data.theme`, but no frontend code reads or applies it. Theme switching is a stub awaiting implementation. A contributor wiring this up should add alternate palettes in `app.css` and toggle a class on the document root based on `settings.data.theme`.

### 7.6 Entry Point

`main.js` uses Svelte 5's `mount()` API to mount `AppShell` into `<div id="app">` in `index.html`. No provider wrappers or context setup. `AppShell.onMount` calls `initAll()` then registers the three Tauri event listeners (see §6.1).

---

## 8. Coding Standards

These rules are load-bearing governance. Reserve them for code review.

- Prioritize memory safety and strict typing (Rust).
- Use `tauri::command` for all backend-to-frontend communication.
- All SQLite operations use `rusqlite` with connection pooling (`deadpool-sqlite`) to support WAL mode.
- **Pure JavaScript frontend** — no TypeScript. Types expressed as JSDoc `@typedef` annotations in `src/lib/types.js`.
- **Centralized API layer** — all `invoke()` calls live in `src/lib/commands.js`. No component calls `invoke()` directly.
- **Infrastructure encapsulation** — all process management goes through `SidecarController`. No raw `tokio`/`std` process calls in command handlers.
- **Flag dictionary separation** — `flag_dictionary` (System.db) feeds ONLY tooltip text. `model_settings` (ModelRegistry.db) drives all configurable flags.
- **GPU detection** — call `HardwareProfileRow::has_usable_gpu()`. Never duplicate the `starts_with("CPU-only")` heuristic.
- **Cross-platform paths** — use `std::path::PathBuf`. No hardcoded `/` or `\` separators. OS-specific logic gated behind `#[cfg(target_os = "...")]`.
- `nvml-wrapper` and `sysinfo` are cross-platform and require no `cfg` gates.

---

## 9. Dependency Hygiene

A dedicated cleanup pass removed three previously-unused dependencies. They are **gone** from the manifests and should not be re-added:

| Dependency | Where | Status |
|-----------|-------|--------|
| `thiserror = "2.0"` | `src-tauri/Cargo.toml` | **Removed.** Was never imported; all error handling uses `anyhow`. |
| `dirs = "6.0"` | `src-tauri/Cargo.toml` | **Removed.** Was never used; `paths.rs` resolves dirs via Tauri's `app.path()` API. |
| `@tauri-apps/plugin-log` | `package.json` | **Removed.** Was never imported; frontend errors route to the in-UI queue. |

Note: the Rust backend's `log` crate (used by `tauri-plugin-log` on the Rust side) is still active and should not be confused with the removed frontend plugin.
