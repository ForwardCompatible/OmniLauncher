// Prevents an extra console window on Windows; no-op on Linux.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // --- WebKit2GTK + NVIDIA workaround (Linux only) ---
    //
    // On Linux + NVIDIA proprietary drivers, WebKit2GTK's GPU-accelerated
    // compositing path fails inside its sandboxed GPU process:
    //   `KMS: DRM_IOCTL_MODE_CREATE_DUMB failed: Permission denied`
    //   `Failed to create GBM buffer of size WxH: Permission denied`
    // which produces a blank white window even though the page's JS/DOM has
    // loaded fine. Forcing software compositing sidesteps the GPU process
    // entirely. This has no effect on llama-server itself (which uses CUDA
    // directly) — only on how Tauri's webview paints to the screen.
    //
    // Set unconditionally on Linux; harmless elsewhere. We don't override an
    // explicit user setting so the env var can still be toggled externally.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").is_none() {
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    }

    omnilauncher_lib::run()
}
