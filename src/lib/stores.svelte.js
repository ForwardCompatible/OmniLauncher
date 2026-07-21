// Shared application state via Svelte 5 module-level runes.

import * as cmd from "./commands.js";

// ── Reactive state ──

export const page = $state({ current: "loader" });
export const models = $state({ list: [] });
export const hardware = $state({ data: null });
export const hardwareStats = $state({ data: /** @type {import('./types.js').HardwareStats | null} */ (null) });
export const proxy = $state({ data: null });
export const processes = $state({ list: [] });
export const settings = $state({ data: null });
export const flags = $state({ map: new Map() });

// Error queue — array of { message, timestamp, severity }.
// All backend failures route here so the user is never left guessing.
export const errors = $state({ items: [] });

// HuggingFace auth + search + downloads state.
export const hfAuth = $state({ data: /** @type {import('./types.js').HfAuthStatus|null} */ (null) });
export const hfSearch = $state({
  results: /** @type {import('./types.js').HfModelResult[]} */ ([]),
  nextCursor: /** @type {string|null} */ (null),
  loading: false,
});
export const hfDownloads = $state({
  /** @type {Record<string, { id: number, downloaded: number, total: number|null, status: "active"|"completed"|"failed", message?: string }>} */
  map: {},
});

export async function refreshHfAuth() {
  try {
    hfAuth.data = await cmd.hfAuthStatus();
  } catch (e) {
    pushError(String(e), "warning");
  }
}

/** Update one entry in the downloads map (keyed by `repoId/filename`). */
export function setHfDownload(key, entry) {
  hfDownloads.map[key] = entry;
}

/** Remove a finished/failed download entry. */
export function clearHfDownload(key) {
  delete hfDownloads.map[key];
}

/**
 * Push an error into the visible UI queue.
 * @param {string} message
 * @param {"error" | "warning"} [severity="error"]
 */
export function pushError(message, severity = "error") {
  errors.items.push({ message, timestamp: Date.now(), severity });
}

/** Remove a single error by index. */
export function dismissError(index) {
  errors.items.splice(index, 1);
}

/** Clear all errors. */
export function clearErrors() {
  errors.items = [];
}

// ── Refresh functions ──
// All catches route to pushError so failures are visible.

export async function refreshModels() {
  try {
    models.list = await cmd.getModels();
  } catch (e) {
    pushError(String(e), "error");
  }
}

export async function refreshProxy() {
  try {
    proxy.data = await cmd.getProxyStatus();
  } catch (e) {
    pushError(String(e), "warning");
  }
}

export async function refreshProcesses() {
  try {
    processes.list = await cmd.getProcessStatus();
  } catch (e) {
    pushError(String(e), "warning");
  }
}

export async function refreshHardware() {
  try {
    hardware.data = await cmd.getHardwareProfile();
  } catch (e) {
    pushError(String(e), "warning");
  }
}

export async function refreshHardwareStats() {
  try {
    hardwareStats.data = await cmd.getHardwareStats();
  } catch (e) {
    pushError(String(e), "warning");
  }
}

export async function refreshSettings() {
  try {
    settings.data = await cmd.getAppSettings();
  } catch (e) {
    pushError(String(e), "warning");
  }
}

export async function refreshFlags() {
  try {
    const entries = await cmd.getFlagDictionary();
    flags.map = new Map(entries.map((f) => [f.cli_argument, f]));
  } catch (e) {
    pushError(String(e), "warning");
  }
}

/** Fetch all initial data on app boot. */
export async function initAll() {
  await Promise.all([
    refreshModels(),
    refreshHardware(),
    refreshHardwareStats(),
    refreshProxy(),
    refreshProcesses(),
    refreshSettings(),
    refreshFlags(),
  ]);
}

// ── Navigation ──

export function navigate(p) {
  page.current = p;
}
