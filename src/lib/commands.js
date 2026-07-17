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
