# OmniLauncher

A Tauri v2 desktop application that provides a clean simple GUI that locally exposes an OpenAI-compatible API gateway with hosted, CUDA-accelerated `llama-server` models. Drop in `.gguf` files, configure VRAM allocation, and point any OpenAI-compatible client at a single reverse-proxy port.

![First open](https://github.com/ForwardCompatible/OmniLauncher/blob/main/OmniLauncher_1.png)


## Features

- **Hybrid VRAM Management** — Dynamic layer offloading via llama.cpp's `--fit` engine. Automatically splits model layers between GPU VRAM and system RAM based on your hardware.
- **Dual-Model Support** — Run either a chat model (`/v1/chat/completions`) an embedding model (`/v1/embeddings`), or both simultaneously behind a single reverse proxy. (**hardware limited feature)
- **42 Configurable Flags** — Temperature, top-k/p, repeat penalty, mirostat, DRY sampling, RoPE scaling, and more. All default to "auto" — only set what you need.
- **Helpful Tooltips** — Each setting has a hover-tip to explain what it does, in terms you can understand (NO excessive techno-babble!)
- **Hardware Auto-Detection** — Scans NVIDIA VRAM, system RAM, and CPU cores on first launch. CPU-only fallback if no GPU is detected.
- **HuggingFace Model Browser** — Search, browse, and download GGUF models directly from HuggingFace with OAuth authentication.
- **Live Hardware Monitor** — Real-time CPU, RAM, and VRAM usage in the footer.
- **Model Card Viewer** — Read the README/model card of any HuggingFace repo before downloading.
- **Pure JavaScript Frontend** — Svelte 5 with runes. No TypeScript, no bloat.

## Quick Start

### Linux

```bash
git clone https://github.com/ForwardCompatible/OmniLauncher.git
cd OmniLauncher
./setup.sh
cargo tauri dev
```

### Windows (PowerShell)

```powershell
git clone https://github.com/ForwardCompatible/OmniLauncher.git
cd OmniLauncher
.\setup.ps1
cargo tauri dev
```

## Prerequisites

| Requirement | Version | Notes |
|-------------|---------|-------|
| Rust | 1.95+ | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| Node.js | 20+ | https://nodejs.org/ |
| NVIDIA Driver | 570.15+ | For GPU support (optional — CPU-only mode works without) |
| CUDA | 12.x | Runtime libraries come bundled with the binary |

### Linux Additional

```bash
sudo apt install -y libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev patchelf libnccl2
```
![model selected and model cards expanded](https://github.com/ForwardCompatible/OmniLauncher/blob/main/OmniLauncher_2.png)

## Adding Models

Drop `.gguf` files into the `models/` directory:

```
models/
├── my-chat-model.gguf          # Chat models (auto-detected from GGUF header)
├── my-embedding-model.gguf     # Embedding models (auto-detected from pooling_type)
└── multimodal/
    └── my-vision-projector.gguf # Vision projectors (**Future feature)
```

The app discovers models automatically on startup. No manual registration needed.

## Using the API

Once a model is launched, point any OpenAI-compatible client at:

```
http://127.0.0.1:52715
```

Routes:
- `POST /v1/chat/completions` → your chat model
- `POST /v1/embeddings` → your embedding model

Example with `curl`:

```bash
curl http://127.0.0.1:52715/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"my-model","messages":[{"role":"user","content":"Hello!"}]}'
```

## Architecture

```
OmniLauncher/
├── src/                    # Svelte 5 frontend (pure JS)
│   ├── components/         # ModelCard (unified), NavRail, HfBrowser, LocalLibrary, ...
│   ├── lib/                # commands.js (API layer), format.js, types.js, stores.svelte.js
│   └── pages/              # Loader, Models, Settings
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── sidecar.rs      # Encapsulated process controller (start/stop/status)
│   │   ├── process.rs      # VRAM Translation Engine (build_args)
│   │   ├── proxy.rs        # axum reverse proxy with SSE streaming
│   │   ├── gguf.rs         # Hand-rolled GGUF header parser
│   │   ├── hardware.rs     # NVML + sysinfo detection
│   │   ├── hf_auth.rs      # HuggingFace OAuth credential storage (keyring)
│   │   ├── huggingface.rs  # HuggingFace API client (search, model cards, files)
│   │   ├── download_manager.rs # Resumable GGUF downloads with progress events
│   │   ├── db/             # SQLite layer (deadpool-sqlite, WAL mode)
│   │   └── commands/       # Tauri command bridge
│   └── resources/          # Bundled llama-server binary (Linux)
└── models/                 # User-supplied .gguf files
```

See [ROADMAP.md](ROADMAP.md) for the project roadmap.

## Development

```bash
# Run in dev mode (hot reload frontend + backend)
cargo tauri dev

# Build production installer
cargo tauri build

# Run tests
cd src-tauri && cargo test
```

## Additional Images
![remaining available model settings](https://github.com/ForwardCompatible/OmniLauncher/blob/main/OmniLauncher_3.png)
![example of hover tooltip](https://github.com/ForwardCompatible/OmniLauncher/blob/main/OmniLauncher_info.png)

## License

PolyForm Noncommercial License 1.0.0
https://github.com/polyformproject/polyform-licenses/blob/1.0.0/PolyForm-Noncommercial-1.0.0.md