## Plan: Switch `setup.ps1` from ai-dock to official ggml-org/llama.cpp b9821

### Background & verified facts
- The official ggml-org Windows CUDA build (tag `b9821`, published 2026-06-26) splits into **two zips** that must both be downloaded for a driver-only install:
  - **Binary:** `llama-b9821-bin-win-cuda-12.4-x64.zip` (249.65 MB) — contains `llama-server.exe` + ggml DLLs, **no** CUDA runtime
    - SHA-256: `b430eea479130961f207418f8b841cbffc9b8d83e1ee179f860a5e751818c8d2`
  - **CUDA runtime:** `cudart-llama-bin-win-cuda-12.4-x64.zip` (373.31 MB) — `cudart64_12.dll`, `cublas64_12.dll`, etc.
    - SHA-256: `8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6`
- User decision: download **both** zips, merge into `%APPDATA%/com.omnilauncher.app/binaries/`, but add a **pre-flight check** that skips the cudart download if the CUDA runtime DLLs are already present on the machine (system-installed CUDA, or a prior run of the script).
- Runtime code in `sidecar.rs:288` already resolves `app_data_dir().join("binaries").join("llama-server.exe")` and prepends the dir to PATH for sibling DLL resolution — **no Rust changes needed**. This is purely a packaging/script fix.
- Linux is untouched: `setup.sh`, the bundled binary in `src-tauri/resources/llama-server/`, and its `PROVENANCE.txt`/`VERSION.txt` keep using ai-dock.

### File 1: `setup.ps1` (rewrite)
**Configuration block (lines 14–20):** Replace the single ai-dock URL/tag/asset/SHA with a version-pinned pair plus CUDA-version constant. New shape:
```powershell
$LlamaTag   = "b9821"
$CudaVer    = "12.4"   # CUDA toolkit line the binary was built against
$LlamaBase  = "https://github.com/ggml-org/llama.cpp/releases/download/$LlamaTag"
$BinaryZip  = "llama-$LlamaTag-bin-win-cuda-$CudaVer-x64.zip"
$BinarySha  = "b430eea479130961f207418f8b841cbffc9b8d83e1ee179f860a5e751818c8d2"
$CudartZip  = "cudart-llama-bin-win-cuda-$CudaVer-x64.zip"
$CudartSha  = "8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6"
```
Delete the now-incorrect NOTE comments (lines 14–16) and the `TBD` placeholder. Document the pinning policy in a fresh comment (bump tag + re-fetch SHAs from `https://github.com/ggml-org/llama.cpp/releases` when upgrading).

**Helper:** Add a `Confirm-Archive($path, $expectedSha)` function to dedupe the SHA-256 verify block (currently inline once; now needed twice). Uses `Get-FileHash -Algorithm SHA256`, lowercased compare, `Write-Fail` on mismatch. Replaces the inline verify at old lines 90–101.

**Download flow (old lines 67–128) restructured into two independent idempotent stages:**

*Stage A — binary (always fetched unless `llama-server.exe` exists):*
- Existing `Test-Path $exePath` skip-logic kept.
- Download `llama-b9821-bin-win-cuda-12.4-x64.zip` → `Confirm-Archive` → `Expand-Archive` to temp.
- The official zip extracts to a top-level folder named `llama-b9821-bin-win-cuda-12.4-x64/` containing `llama-server.exe` + ggml `*.dll` side-by-side (no nested `cuda-12.8/` dir like ai-dock). Existing "take first directory, copy `llama-server.exe` + `*.dll`" logic (old lines 108–115) is correct for this layout — keep it, but make it non-fatal if no subdirectory exists (some ggml zips extract flat to root).

*Stage B — CUDA runtime (pre-flight gated):*
- Pre-flight check: if `$BinariesDir` already contains `cudart64_12.dll`, skip the 373 MB download. This honors the user's "check if CUDA binaries are already present" instruction — covers both system-CUDA machines and re-runs of the script. Print a clear `[INFO] CUDA runtime already present — skipping cudart download.` when skipping.
- If absent: download `cudart-llama-bin-win-cuda-12.4-x64.zip` → `Confirm-Archive` → extract → copy all `*.dll` to `$BinariesDir`.
- This stage also runs if the user deleted the DLLs but kept the exe.

**Provenance (old lines 118–125):** Rewrite the `PROVENANCE.txt` heredoc to record **both** sources accurately:
```
Source:     https://github.com/ggml-org/llama.cpp (official)
Release:    b9821
Binary:     llama-b9821-bin-win-cuda-12.4-x64.zip  ($BinarySha)
CUDA rt:    cudart-llama-bin-win-cuda-12.4-x64.zip ($CudartSha, skipped if pre-existing)
Downloaded: <ISO timestamp>
```
Only written once, after both stages.

**Status messages:** Update `Write-Info`/`Write-Fail` strings at old lines 75 and 87 that mention "ai-dock" / "Check the URL ... ai-dock" → "ggml-org official" / correct help URL.

### File 2: `README.md` (line 40 edit)
The `CUDA | 12.x` row is fine, but line 40's note "Runtime libraries come bundled with the binary" is now accurate **only for the post-setup state**, not the shipped repo. No change needed — the script does bundle them into AppData. Keep as-is. (Verified; no edit.)

### File 3: `AGENTS.md` (lines 11–13 edit)
The "Core Binary" section lumps both OSes under "A pinned ai-dock CUDA build", which is now wrong for Windows. Rewrite to split the provenance:
```
**Core Binary:** CUDA-accelerated `llama-server` (x86_64), source differs per OS.
- **Linux:** A pinned ai-dock CUDA build, bundled as a Tauri resource with sibling `.so` libraries.
- **Windows:** The official ggml-org/llama.cpp CUDA build (tag b9821, CUDA 12.4), downloaded during installation to `%APPDATA%/com.omnilauncher.app/binaries/` along with the matching CUDA 12.4 runtime DLLs. Not bundled in the installer.
```

### File 4: `PROJECT_DOSSIER.md` (lines 37 and 343 edits)
- Line 37 table row currently says `llama-server (ai-dock CUDA build b9893, CUDA 12.8.1) | bundled as Tauri resource`. This is the *cross-platform* summary table — expand to two rows:
  - `| Linux binary | llama-server (ai-dock b9893, CUDA 12.8.1) | bundled as Tauri resource |`
  - `| Windows binary | llama-server (ggml-org b9821, CUDA 12.4) | downloaded to AppData at setup |`
- Line 343 "A pinned ai-dock CUDA build of `llama-server` (Linux x86_64)..." — append a sentence noting Windows uses the official ggml-org release.

### Out of scope (explicitly)
- `setup.sh` — unchanged (Linux keeps ai-dock).
- Committed `src-tauri/resources/llama-server/PROVENANCE.txt` + `VERSION.txt` — unchanged (Linux artifacts).
- `sidecar.rs`, `process.rs`, any Rust code — unchanged (runtime path resolution already correct).
- `models/README.md:29` "b9893+" — leave; it's a model-compatibility floor, not a Windows-binary claim.

### Verification after implementation
1. Re-read `setup.ps1` end-to-end to confirm no `ai-dock` / `TBD` / `cuda-12.8` strings remain and both SHAs are present.
2. `grep -rn "ai-dock" AGENTS.md PROJECT_DOSSIER.md` should return only the **Linux** ai-dock references (lines for the Linux binary), not the Windows ones.
3. PowerShell syntax sanity: since this is a Linux dev box, I cannot execute `pwsh`; I will visually confirm PowerShell idiom correctness (no bash-isms leaked from `setup.sh`).
4. Confirm the user-facing help URLs in error branches point at `https://github.com/ggml-org/llama.cpp/releases`.