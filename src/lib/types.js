// Type definitions as JSDoc @typedef comments.
// These provide IDE autocomplete and type-checking without TypeScript.

/**
 * @typedef {Object} AppSettings
 * @property {string | null} models_directory
 * @property {string | null} multimodal_directory
 * @property {number} master_port
 * @property {boolean} auto_port_increment
 * @property {string} theme
 */

/**
 * @typedef {Object} HardwareProfile
 * @property {string} gpu_name
 * @property {number} total_vram_mb
 * @property {number} total_system_ram_mb
 * @property {number} cpu_physical_cores
 * @property {number} cpu_logical_threads
 * @property {string} last_scanned_at
 * @property {boolean} gpu_present
 */

/**
 * Live telemetry sample from the hardware monitor. Pushed to the frontend via
 * the `hardware-stats` event every 2 seconds; also fetchable on demand.
 * @typedef {Object} HardwareStats
 * @property {number} cpu_usage_percent Aggregate CPU utilization, 0.0–100.0
 * @property {number} ram_used_mb Used system RAM in MiB
 * @property {number} ram_total_mb Total system RAM in MiB
 * @property {?number} vram_used_mb Used VRAM in MiB; null when CPU-only
 * @property {?number} vram_total_mb Total VRAM in MiB; null when CPU-only
 */

/**
 * @typedef {Object} ModelDto
 * @property {number} id
 * @property {string} filename
 * @property {string} filepath
 * @property {number} filesize_bytes
 * @property {string} architecture
 * @property {string} model_name
 * @property {number} context_length
 * @property {number} layer_count
 * @property {string} quantization
 * @property {string | null} chat_template
 * @property {string | null} author
 * @property {string | null} role
 * @property {string | null} pooling_type
 * @property {boolean} has_settings
 */

/**
 * @typedef {Object} ModelSettings
 * @property {number | null} vram_allocation_mb
 * @property {number | null} ctx_size
 * @property {boolean | null} flash_attn
 * @property {boolean | null} cpu_mode
 * @property {number | null} batch_size
 * @property {number | null} ubatch_size
 * @property {number | null} threads
 * @property {number | null} threads_batch
 * @property {boolean | null} mlock
 * @property {boolean | null} no_mmap
 * @property {string | null} cache_type_k
 * @property {string | null} cache_type_v
 * @property {boolean | null} cache_prompt
 * @property {number | null} temp
 * @property {number | null} top_k
 * @property {number | null} top_p
 * @property {number | null} min_p
 * @property {number | null} repeat_penalty
 * @property {number | null} repeat_last_n
 * @property {number | null} seed
 * @property {number | null} presence_penalty
 * @property {number | null} frequency_penalty
 * @property {number | null} typical_p
 * @property {number | null} xtc_probability
 * @property {number | null} xtc_threshold
 * @property {number | null} mirostat
 * @property {number | null} mirostat_lr
 * @property {number | null} mirostat_ent
 * @property {number | null} dry_multiplier
 * @property {number | null} dry_base
 * @property {number | null} dry_allowed_length
 * @property {number | null} predict
 * @property {boolean | null} context_shift
 * @property {number | null} parallel
 * @property {boolean | null} cont_batching
 * @property {number | null} timeout
 * @property {string | null} rope_scaling
 * @property {number | null} rope_freq_base
 * @property {string | null} reasoning_format
 * @property {string | null} pooling_type_override
 * @property {number | null} embd_normalize
 * @property {boolean | null} rerank
 */

/**
 * @typedef {Object} ProxyStatus
 * @property {number} master_port
 * @property {boolean} auto_port_increment
 * @property {number | null} chat_port
 * @property {number | null} embedding_port
 */

/**
 * @typedef {Object} RunningProcess
 * @property {number} model_id
 * @property {string} model_name
 * @property {string} role
 * @property {number} port
 * @property {number} pid
 */

/**
 * @typedef {Object} LaunchReport
 * @property {number} model_id
 * @property {string} model_name
 * @property {string} role
 * @property {number} port
 * @property {number} pid
 * @property {string[]} args
 */

/**
 * @typedef {Object} ResyncReport
 * @property {number} added
 * @property {number} updated
 * @property {number} removed
 * @property {number} failed
 * @property {number} total
 */

/**
 * @typedef {Object} FlagEntry
 * @property {string} category
 * @property {string} flag_name
 * @property {string} cli_argument
 * @property {string | null} default_value
 * @property {string} description
 */

/**
 * @typedef {"loader" | "settings"} PageId
 * @typedef {"chat" | "embedding"} Role
 */

/**
 * @typedef {Object} ErrorItem
 * @property {string} message
 * @property {number} timestamp
 * @property {"error" | "warning"} severity
 */

/** The 8 GGUF KV cache quantization types. */
export const CACHE_TYPES = [
  "f32",
  "f16",
  "bf16",
  "q8_0",
  "q4_0",
  "q4_1",
  "iq4_nl",
  "q5_0",
  "q5_1",
];
