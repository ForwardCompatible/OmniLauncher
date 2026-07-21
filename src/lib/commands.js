// Typed wrappers around @tauri-apps/api invoke(). Each function maps to one
// #[tauri::command] in the Rust backend.

import { invoke } from "@tauri-apps/api/core";

// ── Flag dictionary (tooltips) ──

/** @returns {Promise<import('./types.js').FlagEntry[]>} */
export async function getFlagDictionary() {
  return invoke("get_flag_dictionary");
}

// ── App settings ──

/** @returns {Promise<import('./types.js').AppSettings>} */
export async function getAppSettings() {
  return invoke("get_app_settings");
}

/**
 * @param {Object} args
 * @param {string | null} [args.models_directory]
 * @param {string | null} [args.multimodal_directory]
 * @param {number} [args.master_port]
 * @param {boolean} [args.auto_port_increment]
 * @returns {Promise<void>}
 */
export async function saveAppSettings(args) {
  await invoke("save_app_settings_cmd", { args });
}

// ── Hardware ──

/** @returns {Promise<import('./types.js').HardwareProfile>} */
export async function getHardwareProfile() {
  return invoke("get_hardware_profile");
}

/** @returns {Promise<import('./types.js').HardwareProfile>} */
export async function rescanHardware() {
  return invoke("rescan_hardware");
}

/** @returns {Promise<import('./types.js').HardwareStats>} */
export async function getHardwareStats() {
  return invoke("get_hardware_stats");
}

// ── Registry ──

/** @returns {Promise<import('./types.js').ModelDto[]>} */
export async function getModels() {
  return invoke("get_models");
}

/** @returns {Promise<import('./types.js').ResyncReport>} */
export async function resyncRegistry() {
  return invoke("resync_registry");
}

// ── Model settings ──

/**
 * @param {number} modelId
 * @returns {Promise<import('./types.js').ModelSettings>}
 */
export async function getModelSettings(modelId) {
  return invoke("get_model_settings", { modelId });
}

/**
 * @param {number} modelId
 * @param {import('./types.js').ModelSettings} settings
 * @returns {Promise<void>}
 */
export async function saveModelSettings(modelId, settings) {
  await invoke("save_model_settings", { modelId, settings });
}

// ── Proxy ──

/** @returns {Promise<import('./types.js').ProxyStatus>} */
export async function getProxyStatus() {
  return invoke("get_proxy_status");
}

// ── Process management ──

/**
 * @param {number} modelId
 * @param {string} role
 * @returns {Promise<import('./types.js').LaunchReport>}
 */
export async function launchModel(modelId, role) {
  return invoke("launch_model", { args: { model_id: modelId, role } });
}

/** @param {number} modelId @returns {Promise<void>} */
export async function stopModel(modelId) {
  await invoke("stop_model", { modelId });
}

/** @returns {Promise<import('./types.js').RunningProcess[]>} */
export async function getProcessStatus() {
  return invoke("get_process_status");
}

// ── HuggingFace OAuth ──

/** @returns {Promise<import('./types.js').DeviceAuthInfo>} */
export async function hfAuthStart() {
  return invoke("hf_auth_start");
}

/**
 * @param {string} deviceCode
 * @returns {Promise<{ status: string, username?: string, expires_at?: number, message?: string }>}
 */
export async function hfAuthPoll(deviceCode) {
  return invoke("hf_auth_poll", { deviceCode });
}

/** @returns {Promise<void>} */
export async function hfAuthCancel() {
  return invoke("hf_auth_cancel");
}

/** @returns {Promise<import('./types.js').HfAuthStatus>} */
export async function hfAuthStatus() {
  return invoke("hf_auth_status");
}

/** @returns {Promise<void>} */
export async function hfAuthLogout() {
  return invoke("hf_auth_logout");
}

// ── HuggingFace search / list / download ──

/**
 * @param {Object} opts
 * @param {string} [opts.query]
 * @param {string} [opts.sort] Default "downloads"
 * @param {string} [opts.pipeline_tag]
 * @param {boolean} [opts.gguf_only] Default true
 * @param {string} [opts.cursor]
 * @returns {Promise<import('./types.js').HfSearchPage>}
 */
export async function hfSearch(opts) {
  return invoke("hf_search", {
    query: opts.query ?? "",
    sort: opts.sort ?? "downloads",
    pipeline_tag: opts.pipeline_tag ?? null,
    gguf_only: opts.gguf_only ?? true,
    cursor: opts.cursor ?? null,
  });
}

/**
 * @param {string} repoId
 * @returns {Promise<import('./types.js').HfFilesResponse>}
 */
export async function hfListFiles(repoId) {
  return invoke("hf_list_files", { repoId });
}

/**
 * @param {string} repoId
 * @param {string} filename
 * @returns {Promise<number>} size in bytes
 */
export async function hfFileSize(repoId, filename) {
  return invoke("hf_file_size", { repoId, filename });
}

/**
 * Fetch a repo's README (model card) as raw markdown.
 * @param {string} repoId
 * @returns {Promise<string>} markdown text (empty if no README)
 */
export async function hfReadme(repoId) {
  return invoke("hf_readme", { repoId });
}

/**
 * @param {string} repoId
 * @param {string} filename
 * @returns {Promise<number>} download id
 */
export async function hfDownload(repoId, filename) {
  return invoke("hf_download", { repoId, filename });
}

/** @param {number} id @returns {Promise<boolean>} */
export async function hfCancelDownload(id) {
  return invoke("hf_cancel_download", { id });
}

