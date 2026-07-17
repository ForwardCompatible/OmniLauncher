//! Seed the `flag_dictionary` table with the llama-server flag tooltips
//! from AGENTS.md.
//!
//! Per AGENTS.md this data is "explicitly decoupled from dynamic UI component
//! rendering" — it only powers informational hover tooltips. The mutable launch
//! flags live in `model_settings` (registry schema) and are bound to hardcoded
//! UI components, not to this table.

use anyhow::Result;
use deadpool_sqlite::Pool;

/// One flag-dictionary entry.
struct Flag {
    category: &'static str,
    flag_name: &'static str,
    cli_argument: &'static str,
    default_value: &'static str,
    description: &'static str,
}

/// All 13 flags from AGENTS.md, in the same order/category grouping.
const FLAGS: &[Flag] = &[
    // --- Core Execution & Hardware ---
    Flag {
        category: "Core Execution & Hardware",
        flag_name: "GPU Offload",
        cli_argument: "--n-gpu-layers",
        default_value: "999",
        description: "The maximum number of layers to offload to the GPU. Setting this artificially high (like 999) enables hybrid mode, letting the engine dynamically fill available VRAM.",
    },
    Flag {
        category: "Core Execution & Hardware",
        flag_name: "Parameter Fitting",
        cli_argument: "--fit",
        default_value: "on",
        description: "Instructs the engine to dynamically adjust layer offloading to fit within device memory safely, preventing Out-Of-Memory (OOM) crashes.",
    },
    Flag {
        category: "Core Execution & Hardware",
        flag_name: "VRAM Margin",
        cli_argument: "--fit-target",
        default_value: "Auto (Calculated)",
        description: "The target margin of VRAM (in MiB) to leave free on the GPU. Rust calculates this by subtracting your allocated amount from the system total.",
    },
    Flag {
        category: "Core Execution & Hardware",
        flag_name: "Generation Threads",
        cli_argument: "--threads",
        default_value: "Auto",
        description: "Number of CPU threads to use during text generation. Leaving this blank usually defaults to a safe physical core count.",
    },
    Flag {
        category: "Core Execution & Hardware",
        flag_name: "Prompt Processing Threads",
        cli_argument: "--threads-batch",
        default_value: "Same as -t",
        description: "Number of CPU threads used during the initial prompt processing (prefill). Can be set higher than generation threads.",
    },
    // --- Context & Memory Limits ---
    Flag {
        category: "Context & Memory Limits",
        flag_name: "Context Window",
        cli_argument: "--ctx-size",
        default_value: "0",
        description: "Maximum context size in tokens. Setting to 0 automatically loads the maximum context allowed by the model, which may heavily impact RAM.",
    },
    Flag {
        category: "Context & Memory Limits",
        flag_name: "Logical Batch Size",
        cli_argument: "--batch-size",
        default_value: "2048",
        description: "The maximum number of tokens processed simultaneously. Higher values require more RAM but speed up prompt ingestion.",
    },
    Flag {
        category: "Context & Memory Limits",
        flag_name: "Physical Batch Size",
        cli_argument: "--ubatch-size",
        default_value: "512",
        description: "The physical number of tokens processed per underlying computation step. Adjusting this helps fit prompt processing within hardware cache limits.",
    },
    // --- Performance & Optimization ---
    Flag {
        category: "Performance & Optimization",
        flag_name: "Flash Attention",
        cli_argument: "--flash-attn",
        default_value: "off",
        description: "Optimizes the attention mechanism to speed up prompt ingestion and reduce the overall memory footprint. Highly recommended for long contexts.",
    },
    Flag {
        category: "Performance & Optimization",
        flag_name: "Memory Lock",
        cli_argument: "--mlock",
        default_value: "off",
        description: "Forces the operating system to keep the entire model in physical RAM, preventing it from being swapped to the hard drive, which would severely degrade performance.",
    },
    Flag {
        category: "Performance & Optimization",
        flag_name: "Disable Memory Mapping",
        cli_argument: "--no-mmap",
        default_value: "off",
        description: "Forces the model to be loaded entirely into RAM at launch rather than memory-mapped. Slower initial load time, but can prevent stuttering on some systems.",
    },
    // --- Advanced KV Cache Quantization ---
    Flag {
        category: "Advanced KV Cache Quantization",
        flag_name: "K-Cache Quantization",
        cli_argument: "--cache-type-k",
        default_value: "f16",
        description: "Lowers the precision of the Key cache (e.g., to q8_0 or q4_0) to save significant RAM at large context sizes, with minimal quality loss.",
    },
    Flag {
        category: "Advanced KV Cache Quantization",
        flag_name: "V-Cache Quantization",
        cli_argument: "--cache-type-v",
        default_value: "f16",
        description: "Lowers the precision of the Value cache to save RAM. Warning: Quantizing the V-cache aggressively degrades output quality faster than the K-cache.",
    },
    // --- Embedding-Specific ---
    Flag {
        category: "Embedding-Specific",
        flag_name: "Pooling Type",
        cli_argument: "--pooling",
        default_value: "Model default",
        description: "Pooling type for embeddings. Overrides the model's default. Options: none, mean, cls, last, rank.",
    },
    Flag {
        category: "Embedding-Specific",
        flag_name: "Embedding Normalization",
        cli_argument: "--embd-normalize",
        default_value: "2",
        description: "Normalization method for embedding vectors. -1=none, 0=max absolute int16, 1=taxicab, 2=euclidean (default), >2=p-norm.",
    },
    Flag {
        category: "Embedding-Specific",
        flag_name: "Reranking",
        cli_argument: "--rerank",
        default_value: "off",
        description: "Enables the reranking endpoint. Use only with dedicated reranker models (e.g. bge-reranker).",
    },
    // --- Common (Sampling) ---
    Flag {
        category: "Common",
        flag_name: "Temperature",
        cli_argument: "--temp",
        default_value: "0.80",
        description: "Controls randomness in token selection. Higher values produce more creative/diverse output; lower values are more focused and deterministic.",
    },
    Flag {
        category: "Common",
        flag_name: "Top-K",
        cli_argument: "--top-k",
        default_value: "40",
        description: "Limits token selection to the K most likely next tokens. 0 disables this filter.",
    },
    Flag {
        category: "Common",
        flag_name: "Top-P",
        cli_argument: "--top-p",
        default_value: "0.95",
        description: "Nucleus sampling — considers the smallest set of tokens whose cumulative probability exceeds P. 1.0 disables.",
    },
    Flag {
        category: "Common",
        flag_name: "Min-P",
        cli_argument: "--min-p",
        default_value: "0.05",
        description: "Minimum probability for a token to be considered, relative to the most likely token. 0.0 disables.",
    },
    Flag {
        category: "Common",
        flag_name: "Repeat Penalty",
        cli_argument: "--repeat-penalty",
        default_value: "1.00",
        description: "Penalizes repeated token sequences. Values >1.0 reduce repetition. 1.0 disables.",
    },
    Flag {
        category: "Common",
        flag_name: "Repeat Last N",
        cli_argument: "--repeat-last-n",
        default_value: "64",
        description: "Number of recent tokens to consider for the repeat penalty. 0 disables, -1 uses full context.",
    },
    Flag {
        category: "Common",
        flag_name: "Seed",
        cli_argument: "--seed",
        default_value: "-1",
        description: "Random number seed for reproducible outputs. -1 = random each run.",
    },
    // --- Sampling - Extended ---
    Flag {
        category: "Sampling - Extended",
        flag_name: "Presence Penalty",
        cli_argument: "--presence-penalty",
        default_value: "0.00",
        description: "Repeat alpha presence penalty. Positive values increase likelihood of new topics.",
    },
    Flag {
        category: "Sampling - Extended",
        flag_name: "Frequency Penalty",
        cli_argument: "--frequency-penalty",
        default_value: "0.00",
        description: "Repeat alpha frequency penalty. Positive values penalize frequent tokens.",
    },
    Flag {
        category: "Sampling - Extended",
        flag_name: "Typical P",
        cli_argument: "--typical-p",
        default_value: "1.00",
        description: "Locally typical sampling. Penalizes tokens with low information content. 1.0 disables.",
    },
    Flag {
        category: "Sampling - Extended",
        flag_name: "XTC Probability",
        cli_argument: "--xtc-probability",
        default_value: "0.00",
        description: "Probability of sampling the least-likely token (XTC sampling). 0.0 disables.",
    },
    Flag {
        category: "Sampling - Extended",
        flag_name: "XTC Threshold",
        cli_argument: "--xtc-threshold",
        default_value: "0.10",
        description: "Threshold for XTC sampling. 1.0 disables.",
    },
    Flag {
        category: "Sampling - Extended",
        flag_name: "Mirostat",
        cli_argument: "--mirostat",
        default_value: "0",
        description: "Mirostat sampling mode. 0=disabled, 1=Mirostat, 2=Mirostat 2.0. Ignores top-k/top-p/typical when active.",
    },
    Flag {
        category: "Sampling - Extended",
        flag_name: "Mirostat LR",
        cli_argument: "--mirostat-lr",
        default_value: "0.10",
        description: "Mirostat learning rate (eta). Controls how fast the algorithm adapts.",
    },
    Flag {
        category: "Sampling - Extended",
        flag_name: "Mirostat Entropy",
        cli_argument: "--mirostat-ent",
        default_value: "5.00",
        description: "Mirostat target entropy (tau). Higher values produce more varied output.",
    },
    Flag {
        category: "Sampling - Extended",
        flag_name: "DRY Multiplier",
        cli_argument: "--dry-multiplier",
        default_value: "0.00",
        description: "DRY sampling multiplier. Penalizes repeating multi-token sequences. 0.0 disables.",
    },
    Flag {
        category: "Sampling - Extended",
        flag_name: "DRY Base",
        cli_argument: "--dry-base",
        default_value: "1.75",
        description: "DRY sampling base value. Controls penalty growth rate.",
    },
    Flag {
        category: "Sampling - Extended",
        flag_name: "DRY Allowed Length",
        cli_argument: "--dry-allowed-length",
        default_value: "2",
        description: "Allowed repetition length before DRY penalty applies.",
    },
    // --- Context & Batch ---
    Flag {
        category: "Context & Batch",
        flag_name: "Max Tokens",
        cli_argument: "--predict",
        default_value: "-1",
        description: "Maximum number of tokens to generate. -1 = unlimited (until context full or EOS).",
    },
    Flag {
        category: "Context & Batch",
        flag_name: "Context Shift",
        cli_argument: "--context-shift",
        default_value: "off",
        description: "Use context shift for infinite text generation. When enabled, the context window slides forward as it fills.",
    },
    // --- Server Config ---
    Flag {
        category: "Server Config",
        flag_name: "Parallel Slots",
        cli_argument: "--parallel",
        default_value: "-1",
        description: "Number of concurrent request slots. -1 = auto-detect based on context. Higher values allow more simultaneous users.",
    },
    Flag {
        category: "Server Config",
        flag_name: "Continuous Batching",
        cli_argument: "--cont-batching",
        default_value: "on",
        description: "Dynamic batching — processes new requests while others are generating. Improves throughput for concurrent use.",
    },
    Flag {
        category: "Server Config",
        flag_name: "Prompt Caching",
        cli_argument: "--cache-prompt",
        default_value: "on",
        description: "Caches prompt processing results. Speeds up repeated prompts but uses more RAM.",
    },
    Flag {
        category: "Server Config",
        flag_name: "Server Timeout",
        cli_argument: "--timeout",
        default_value: "3600",
        description: "Server read/write timeout in seconds. Connections that idle longer are dropped.",
    },
    // --- RoPE / Context Extension ---
    Flag {
        category: "RoPE / Context Extension",
        flag_name: "RoPE Scaling",
        cli_argument: "--rope-scaling",
        default_value: "linear",
        description: "RoPE context scaling method: none, linear, or yarn (Yet another RoPE extensioN). Allows extending context beyond the model's trained window.",
    },
    Flag {
        category: "RoPE / Context Extension",
        flag_name: "RoPE Base Frequency",
        cli_argument: "--rope-freq-base",
        default_value: "from model",
        description: "Base frequency for RoPE positional embeddings. Changing this affects how the model handles long contexts.",
    },
    // --- Reasoning ---
    Flag {
        category: "Reasoning",
        flag_name: "Reasoning Format",
        cli_argument: "--reasoning-format",
        default_value: "auto",
        description: "How thinking/reasoning traces are returned for reasoning models (e.g. DeepSeek-R1). none, deepseek, or deepseek-legacy.",
    },
];

/// Idempotently seed the flag dictionary. Existing entries are kept (the UI may
/// have localized/customized tooltips); missing entries are inserted. Re-runs
/// only add new flags, never duplicate or overwrite.
pub async fn run(system_pool: &Pool) -> Result<()> {
    let conn = system_pool.get().await?;
    conn.interact(|c| seed_inner(c))
        .await
        .map_err(|e| anyhow::anyhow!("Panic during flag seeding: {e}"))??;
    Ok(())
}

fn seed_inner(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
    // `cli_argument` is the natural uniqueness key for a flag entry.
    let mut stmt = conn.prepare(
        "INSERT OR IGNORE INTO flag_dictionary
            (category, flag_name, cli_argument, default_value, description)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for f in FLAGS {
        stmt.execute((
            f.category,
            f.flag_name,
            f.cli_argument,
            f.default_value,
            f.description,
        ))?;
    }
    Ok(())
}
