# OmniLauncher — Roadmap

> Living document tracking completed features, known limitations, and planned next steps. For architecture and code patterns, see [AGENTS.md](AGENTS.md).

## Completed Features

### Core Engine
- **CUDA llama-server management** — Bundled ai-dock CUDA binary (Linux) / official ggml-org binary (Windows) with full lifecycle via `SidecarController`
- **VRAM Translation Engine** — Hybrid `--fit on` + `-ngl auto` + `--fit-target` with CPU-only safety valve
- **42-flag launch system** — Every llama-server flag configurable per-model, all defaulting to "auto" (omitted)
- **Hardware auto-detection** — NVML + sysinfo scan, cached on first launch, manual rescan available
- **Live hardware monitor** — Footer widget tracking CPU%, RAM, and VRAM usage (2s polling)

### API Gateway
- **axum reverse proxy** — Path-based routing on `127.0.0.1:{master_port}` with SSE streaming pass-through
- **Dual-model support** — Chat (`/v1/chat/completions`) + Embedding (`/v1/embeddings`) behind a single port
- **Port auto-increment** — Graceful fallback if the master port is busy

### Model Management
- **GGUF parser** — Hand-rolled v1/v2/v3 parser with auto-role detection (pooling_type → embedding, chat_template → chat)
- **Model registry** — SQLite-backed metadata store with 42-column per-model settings
- **HuggingFace browser** — OAuth device-code auth, search with cursor pagination, lazy file expansion, README/model-card viewer
- **Native downloader** — Streaming download with progress, cancellation, GGUF validation, and auto-registration
- **Local library** — Client-side search/sort/filter over downloaded models

### Cross-Platform
- **Linux (primary)** — Ubuntu with NVIDIA CUDA, GNOME Keyring via Secret Service
- **Windows Native** — Credential Manager, CREATE_NO_WINDOW process spawning, ggml-org CUDA binary
- **OS keychain auth** — OAuth token stored in OS-native credential store (never in DB or plaintext)

## Known Limitations

- **GGUF parser reads entire files** — `std::fs::read` loads multi-GB models into RAM during parsing (header-only stream refactor planned)
- **No download resume** — Cancelled/failed downloads delete the `.part` file; restart from scratch
- **No download queue** — Multiple concurrent downloads allowed but hit rate limits faster
- **No parameter-count filter** — HF API doesn't support it; param count is only visible after expanding a repo
- **`tags=gguf` is leaky** — HF's server filter returns ~40% false positives; validated client-side via siblings check

## Planned Next Steps

### Performance
1. **Streaming GGUF parser** — Replace `std::fs::read` with a `BufReader` that reads only the header + KV section. Biggest startup/memory win available.
2. **Batch file-size HEAD requests** — Collapse the N+1 HEAD-on-expand pattern into a single backend command.

### Architecture
3. **Data-driven page registry** — Replace the hardcoded NavRail + AppShell if/else chain with a `PAGES` array for easier page additions.
4. **Split HF store** — Move HF-specific state (`hfAuth`, `hfSearch`, `hfDownloads`) into a separate `stores/hf.svelte.js` module.

### Features
5. **Download resume** — Persist partial-download state to support resuming interrupted transfers.
6. **In-app chat page** — A local chat interface using the reverse proxy (no external tool needed for basic usage).
7. **Model quantization comparison** — Side-by-side view of available quants for a base model.
