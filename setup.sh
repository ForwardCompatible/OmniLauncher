#!/usr/bin/env bash
#
# OmniLauncher Linux Setup Script
#
# Downloads and installs the pinned llama-server CUDA binary, installs
# frontend dependencies, and prepares the project for `cargo tauri dev`.
#
# Usage:  ./setup.sh
#

set -euo pipefail

# ── Configuration ──

LLAMA_TAG="b9893"
LLAMA_ASSET="llama.cpp-${LLAMA_TAG}-cuda-12.8-amd64.tar.gz"
LLAMA_URL="https://github.com/ai-dock/llama.cpp-cuda/releases/download/${LLAMA_TAG}/${LLAMA_ASSET}"
LLAMA_SHA256="eb5a24647a11ad7103dbf73c10595e002619694923321c49f0e6be6a062ef9e4"

RESOURCES_DIR="src-tauri/resources/llama-server"

# ── Helpers ──

info()  { echo -e "\033[1;34m[INFO]\033[0m  $*"; }
ok()    { echo -e "\033[1;32m[OK]\033[0m    $*"; }
warn()  { echo -e "\033[1;33m[WARN]\033[0m  $*"; }
fail()  { echo -e "\033[1;31m[FAIL]\033[0m  $*"; exit 1; }

# ── Preflight checks ──

info "Checking prerequisites..."

command -v rustc >/dev/null 2>&1 || fail "Rust is not installed. Install via: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
command -v node >/dev/null 2>&1 || fail "Node.js is not installed. Install via: https://nodejs.org/"
command -v npm >/dev/null 2>&1 || fail "npm is not installed (comes with Node.js)."
command -v cargo >/dev/null 2>&1 || fail "Cargo is not installed (comes with Rust)."

RUST_VERSION=$(rustc --version | grep -oP '\d+\.\d+\.\d+' | head -1)
info "  Rust $RUST_VERSION"
info "  Node $(node --version)"
info "  npm $(npm --version)"

# Check for NVIDIA driver
if ! command -v nvidia-smi >/dev/null 2>&1; then
    warn "nvidia-smi not found — GPU features will be disabled (CPU-only mode)."
else
    info "  NVIDIA driver detected: $(nvidia-smi --query-gpu=driver_version --format=csv,noheader | head -1)"
fi

ok "Prerequisites satisfied."

# ── Frontend dependencies ──

info "Installing frontend dependencies (npm install)..."
npm install --silent
ok "Frontend dependencies installed."

# ── Download llama-server binary ──

if [ -f "${RESOURCES_DIR}/llama-server" ]; then
    info "llama-server binary already exists — skipping download."
    info "  To re-download, delete ${RESOURCES_DIR}/llama-server and re-run this script."
else
    info "Downloading llama-server (ai-dock build ${LLAMA_TAG}, CUDA 12.8)..."

    TMP_DIR=$(mktemp -d)
    trap 'rm -rf "${TMP_DIR}"' EXIT

    ARCHIVE="${TMP_DIR}/${LLAMA_ASSET}"

    curl -L --fail --show-error --progress-bar -o "${ARCHIVE}" "${LLAMA_URL}"

    # Verify SHA-256
    info "Verifying SHA-256..."
    echo "${LLAMA_SHA256}  ${ARCHIVE}" | sha256sum -c - || fail "SHA-256 verification failed. The download may be corrupted."

    # Extract
    info "Extracting..."
    tar -xzf "${ARCHIVE}" -C "${TMP_DIR}"

    # The archive contains a cuda-12.8/ directory — copy its contents
    SRC_DIR="${TMP_DIR}/cuda-12.8"
    if [ ! -d "${SRC_DIR}" ]; then
        fail "Expected cuda-12.8/ directory in archive not found."
    fi

    mkdir -p "${RESOURCES_DIR}"

    # Copy only what we need: llama-server + its .so dependencies
    cp "${SRC_DIR}/llama-server" "${RESOURCES_DIR}/"
    for so in "${SRC_DIR}"/lib*.so*; do
        [ -e "$so" ] && cp "$so" "${RESOURCES_DIR}/"
    done
    cp "${SRC_DIR}/VERSION.txt" "${RESOURCES_DIR}/" 2>/dev/null || true

    # Set executable permission
    chmod +x "${RESOURCES_DIR}/llama-server"

    # Apply patchelf for RPATH=$ORIGIN
    if command -v patchelf >/dev/null 2>&1; then
        info "Applying patchelf RPATH=\$ORIGIN..."
        patchelf --set-rpath '$ORIGIN' "${RESOURCES_DIR}/llama-server"
        patchelf --set-rpath '$ORIGIN' "${RESOURCES_DIR}/libllama-server-impl.so" 2>/dev/null || true
        ok "RPATH set."
    else
        warn "patchelf not found — RPATH not applied."
        warn "The app will set LD_LIBRARY_PATH at runtime as a fallback."
        warn "Install patchelf via: sudo apt install patchelf"
    fi

    # Write provenance
    cat > "${RESOURCES_DIR}/PROVENANCE.txt" << PVEOF
Source:     https://github.com/ai-dock/llama.cpp-cuda
Release:    ${LLAMA_TAG}
Asset:      ${LLAMA_ASSET}
SHA-256:    ${LLAMA_SHA256}
Downloaded: $(date -u +"%Y-%m-%dT%H:%M:%SZ")
PVEOF

    ok "llama-server installed to ${RESOURCES_DIR}/"
fi

# ── Verify binary runs ──

info "Verifying llama-server binary..."
if LD_LIBRARY_PATH="${RESOURCES_DIR}" "${RESOURCES_DIR}/llama-server" --version >/dev/null 2>&1; then
    VERSION=$(${RESOURCES_DIR}/llama-server --version 2>&1 | head -1)
    ok "llama-server: ${VERSION}"
else
    warn "llama-server --version returned non-zero. Check library dependencies."
    warn "Run: LD_LIBRARY_PATH=${RESOURCES_DIR} ${RESOURCES_DIR}/llama-server --version"
fi

# ── Done ──

echo ""
ok "Setup complete!"
echo ""
info "Next steps:"
info "  1. Add .gguf model files to the models/ directory"
info "  2. Run: cargo tauri dev"
info "  3. Point your external tool at: http://127.0.0.1:52715"
echo ""
