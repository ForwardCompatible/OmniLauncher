#
# OmniLauncher Windows Setup Script
#
# Downloads the official ggml-org/llama.cpp CUDA build of llama-server.exe for
# Windows, plus the matching CUDA 12.4 runtime DLLs, and installs frontend
# dependencies in preparation for `cargo tauri dev`.
#
# Usage:  .\setup.ps1
#

$ErrorActionPreference = "Stop"

# ── Configuration ──

# Official ggml-org/llama.cpp release. The Windows CUDA build ships as TWO zips:
#   1. llama-<tag>-bin-win-cuda-<ver>-x64.zip   -> llama-server.exe + ggml DLLs (no CUDA runtime)
#   2. cudart-llama-bin-win-cuda-<ver>-x64.zip  -> CUDA runtime DLLs (cudart64_12.dll, cublas64_12.dll, ...)
# Both are extracted into the same binaries\ directory so Windows resolves the
# CUDA DLLs side-by-side via the PATH prepend in sidecar.rs.
#
# To upgrade: pick a new tag at https://github.com/ggml-org/llama.cpp/releases,
# then update $LlamaTag, $CudaVer, and BOTH SHA-256 values below. Fetch the SHAs
# from the release's asset list (each asset exposes a sha256 digest).
$LlamaTag  = "b9821"
$CudaVer   = "12.4"
$LlamaBase = "https://github.com/ggml-org/llama.cpp/releases/download/$LlamaTag"

$BinaryZip = "llama-$LlamaTag-bin-win-cuda-$CudaVer-x64.zip"
$BinarySha = "b430eea479130961f207418f8b841cbffc9b8d83e1ee179f860a5e751818c8d2"
$CudartZip = "cudart-llama-bin-win-cuda-$CudaVer-x64.zip"
$CudartSha = "8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6"

# Marker DLL used by the pre-flight check: if present in the binaries directory
# (e.g. left by a prior run, or copied from a system CUDA install), the ~373 MB
# cudart download is skipped.
$CudartMarkerDll = "cudart64_12.dll"

$BinariesDir = Join-Path $env:APPDATA "com.omnilauncher.app\binaries"

# ── Helpers ──

function Write-Info($msg)  { Write-Host "[INFO]  $msg" -ForegroundColor Cyan }
function Write-Ok($msg)    { Write-Host "[OK]    $msg" -ForegroundColor Green }
function Write-Warn($msg)  { Write-Host "[WARN]  $msg" -ForegroundColor Yellow }
function Write-Fail($msg)  { Write-Host "[FAIL]  $msg" -ForegroundColor Red; exit 1 }

function Confirm-Archive {
    param([string]$Path, [string]$ExpectedSha)
    Write-Info "Verifying SHA-256..."
    $actual = (Get-FileHash $Path -Algorithm SHA256).Hash.ToLower()
    if ($actual -ne $ExpectedSha.ToLower()) {
        Write-Fail "SHA-256 verification failed for $Path.`nExpected: $ExpectedSha`nActual:   $actual"
    }
    Write-Ok "SHA-256 verified."
}

# Download -> verify -> extract a zip into a target directory.
# Returns the directory the caller should copy files from (top-level folder if
# the archive contains exactly one, otherwise the extraction root itself).
function Invoke-DownloadZip {
    param([string]$Url, [string]$Asset, [string]$ExpectedSha, [string]$DestDir)
    $archive = Join-Path $DestDir $Asset
    Write-Info "Downloading $Asset..."
    try {
        Invoke-WebRequest -Uri $Url -OutFile $archive -UseBasicParsing
    } catch {
        Write-Fail "Download failed: $_. Check the URL and tag at https://github.com/ggml-org/llama.cpp/releases"
    }
    Confirm-Archive -Path $archive -ExpectedSha $ExpectedSha

    Write-Info "Extracting..."
    Expand-Archive -Path $archive -DestinationPath $DestDir -Force

    # ggml-org zips contain a single top-level folder (e.g. llama-b9821-bin-win-cuda-12.4-x64/).
    # Some builds flatten to the root — handle both.
    $subDirs = Get-ChildItem -Path $DestDir -Directory | Where-Object { $_.Name -ne $Asset.TrimEnd('.zip') }
    if ($subDirs.Count -ge 1) {
        return ($subDirs | Select-Object -First 1).FullName
    }
    return $DestDir
}

# ── Preflight checks ──

Write-Info "Checking prerequisites..."

if (-not (Get-Command rustc -ErrorAction SilentlyContinue)) {
    Write-Fail "Rust is not installed. Install via: https://rustup.rs/"
}
if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    Write-Fail "Node.js is not installed. Install via: https://nodejs.org/"
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Fail "Cargo is not installed (comes with Rust)."
}

$RustVersion = (rustc --version) -replace '.*(\d+\.\d+\.\d+).*', '$1'
Write-Info "  Rust $RustVersion"
Write-Info "  Node $(node --version)"
Write-Info "  npm $(npm --version)"

# Check for NVIDIA driver
$nvidiaSmi = Get-Command nvidia-smi -ErrorAction SilentlyContinue
if ($nvidiaSmi) {
    $driverVersion = (nvidia-smi --query-gpu=driver_version --format=csv,noheader | Select-Object -First 1)
    Write-Info "  NVIDIA driver detected: $driverVersion"
} else {
    Write-Warn "nvidia-smi not found — GPU features will be disabled (CPU-only mode)."
}

Write-Ok "Prerequisites satisfied."

# ── Frontend dependencies ──

Write-Info "Installing frontend dependencies (npm install)..."
npm install --silent
Write-Ok "Frontend dependencies installed."

# ── Prepare binaries directory ──

New-Item -ItemType Directory -Force -Path $BinariesDir | Out-Null
$tempDir = New-Item -ItemType Directory -Force -Path (Join-Path $env:TEMP "omnilauncher-setup") | Select-Object -ExpandProperty FullName
$exePath = Join-Path $BinariesDir "llama-server.exe"

# ── Stage A: llama-server binary ──

if (Test-Path $exePath) {
    Write-Info "llama-server.exe already present — skipping binary download."
    Write-Info "  To re-download, delete $exePath and re-run this script."
    $binarySourceDir = $null   # already installed; no extraction dir to reference
} else {
    $binarySourceDir = Invoke-DownloadZip `
        -Url "$LlamaBase/$BinaryZip" `
        -Asset $BinaryZip `
        -ExpectedSha $BinarySha `
        -DestDir $tempDir

    # Copy llama-server.exe and all ggml *.dll side-by-side.
    Get-ChildItem -Path $binarySourceDir -Filter "llama-server.exe" | Copy-Item -Destination $BinariesDir -Force
    Get-ChildItem -Path $binarySourceDir -Filter "*.dll"           | Copy-Item -Destination $BinariesDir -Force
    Write-Ok "llama-server.exe + ggml DLLs installed."
}

# ── Stage B: CUDA 12.4 runtime DLLs (pre-flight gated) ──

$cudartMarkerPath = Join-Path $BinariesDir $CudartMarkerDll
if (Test-Path $cudartMarkerPath) {
    Write-Info "$CudartMarkerDll already present — skipping CUDA runtime download."
    Write-Info "  (System CUDA install detected, or a prior run of this script.)"
} else {
    $cudartSourceDir = Invoke-DownloadZip `
        -Url "$LlamaBase/$CudartZip" `
        -Asset $CudartZip `
        -ExpectedSha $CudartSha `
        -DestDir $tempDir

    Get-ChildItem -Path $cudartSourceDir -Filter "*.dll" | Copy-Item -Destination $BinariesDir -Force
    Write-Ok "CUDA $CudaVer runtime DLLs installed."
}

# ── Provenance ──

# Written once after both stages regardless of which ran.
$cudartStatus = if (Test-Path $cudartMarkerPath) { "present (skipped download)" } else { "downloaded ($CudartSha)" }
$provenance = @"
Source:     https://github.com/ggml-org/llama.cpp (official)
Release:    $LlamaTag
Binary:     $BinaryZip ($BinarySha)
CUDA rt:    $CudartZip ($cudartStatus)
Downloaded: $(Get-Date -Format 'yyyy-MM-ddTHH:mm:ssZ')
"@
$provenance | Out-File (Join-Path $BinariesDir "PROVENANCE.txt") -Encoding UTF8

# ── Verify binary runs ──

Write-Info "Verifying llama-server binary..."
$versionResult = & $exePath --version 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Ok "llama-server: $versionResult"
} else {
    Write-Warn "llama-server --version returned non-zero. Check DLL dependencies."
    Write-Warn "Run: $exePath --version"
}

# ── Done ──

Write-Host ""
Write-Ok "Setup complete!"
Write-Host ""
Write-Info "Next steps:"
Write-Info "  1. Add .gguf model files to the models\ directory"
Write-Info "  2. Run: cargo tauri dev"
Write-Info "  3. Point your external tool at: http://127.0.0.1:52715"
Write-Host ""
