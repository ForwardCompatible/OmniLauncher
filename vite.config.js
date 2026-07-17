import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [svelte()],

  // Vite options tailored for Tauri development; see
  // https://v2.tauri.app/reference/config
  clearScreen: false,
  server: {
    // Tauri expects a fixed port; fail if it's taken.
    port: 5173,
    strictPort: true,
    watch: {
      // Don't watch the Rust side — Tauri handles its own rebuilds.
      ignored: ["**/src-tauri/**"],
    },
  },
});
