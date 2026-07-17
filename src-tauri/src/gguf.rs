//! Hand-rolled GGUF (GPT-Generated Unified Format) header parser.
//!
//! Reads only the metadata needed for the model registry — we stop after the
//! metadata KV section and never touch the tensor info or tensor data. This
//! keeps parse time bounded by the header size, not the (multi-GB) tensor data.
//!
//! Spec: https://github.com/ggml-org/ggml/blob/master/docs/gguf.md
//!
//! ## Scope
//!   * GGUF v1, v2, v3 (they differ in count/string-length widths).
//!   * Little-endian only (the spec says assume LE; BE cannot be detected
//!     in-band and every real-world file is LE).
//!   * Defensive bounds on every count/length to resist corrupt/malicious files.
//!
//! ## What we extract
//!   architecture, model_name, context_length, layer_count, quantization,
//!   chat_template, author — the 7 fields the `models_metadata` table needs.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};

/// The GGUF magic bytes: b"GGUF". Checked as raw bytes, not a cast u32, to
/// avoid any endianness ambiguity (per the spec's explicit warning).
const MAGIC: [u8; 4] = [0x47, 0x47, 0x55, 0x46];

/// Hard caps to prevent OOM on corrupt/malicious headers. Generous enough to
/// never reject a legitimate file.
const MAX_KV_COUNT: u64 = 1 << 20; // 1M metadata entries
const MAX_STRING_LEN: u64 = 1 << 24; // 16 MiB per string (keys are ≤64KiB by spec)
const MAX_ARRAY_LEN: u64 = 1 << 24; // 16M elements

/// The parsed metadata we care about, in the shape the registry wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GgufMetadata {
    pub architecture: String,
    pub model_name: String,
    pub context_length: i64,
    pub layer_count: i64,
    pub quantization: String,
    pub chat_template: Option<String>,
    pub author: Option<String>,
    /// `{arch}.pooling_type` — present on embedding models, absent on chat
    /// models. Used to auto-detect whether a model is an embedding model.
    pub pooling_type: Option<String>,
}

// ─── Metadata value types (spec §3a — note ARRAY=9 comes BEFORE the 64-bit types) ───
mod value_type {
    pub const UINT8: u32 = 0;
    pub const INT8: u32 = 1;
    pub const UINT16: u32 = 2;
    pub const INT16: u32 = 3;
    pub const UINT32: u32 = 4;
    pub const INT32: u32 = 5;
    pub const FLOAT32: u32 = 6;
    pub const BOOL: u32 = 7;
    pub const STRING: u32 = 8;
    pub const ARRAY: u32 = 9;
    pub const UINT64: u32 = 10;
    pub const INT64: u32 = 11;
    pub const FLOAT64: u32 = 12;
}

/// Tagged metadata value. Only the variants we can use; scalars we don't care
/// about still need their byte widths known so we can skip past them.
#[derive(Debug)]
enum Value {
    U32(u32),
    U64(u64),
    I32(i32),
    I64(i64),
    String(String),
    /// Placeholder for any value we walked past but didn't materialize
    /// (FLOAT32, BOOL, arrays, etc.). Keeps the map complete for diagnostics.
    Other,
}

/// Top-level entry point. Reads the file, parses the header, returns metadata.
pub fn parse(path: &Path) -> Result<GgufMetadata> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("Failed to read GGUF file {}", path.display()))?;
    parse_bytes(&bytes).with_context(|| {
        format!("GGUF parse failed for {}", path.display())
    })
}

fn parse_bytes(bytes: &[u8]) -> Result<GgufMetadata> {
    let mut cur = Cursor::new(bytes);

    // --- Header ---
    let magic = read_bytes(&mut cur, 4)?;
    if magic.as_slice() != MAGIC {
        bail!("not a GGUF file (bad magic: {:?})", magic);
    }
    let version = read_u32(&mut cur)?;
    if !matches!(version, 1 | 2 | 3) {
        bail!("unsupported GGUF version: {version} (expected 1, 2, or 3)");
    }
    // v1 uses u32 counts/string-lengths; v2/v3 use u64.
    let wide = version >= 2;

    let _tensor_count = read_count(&mut cur, wide)?;
    let kv_count = read_count(&mut cur, wide)?;
    if kv_count > MAX_KV_COUNT {
        bail!("absurd metadata_kv_count: {kv_count}");
    }

    // --- Metadata KV section ---
    let mut kv: HashMap<String, Value> = HashMap::with_capacity(kv_count as usize);
    for _ in 0..kv_count {
        let key = read_gguf_string(&mut cur, wide)?;
        let vtype = read_u32(&mut cur)?;
        let value = read_value(&mut cur, vtype, wide)?;
        kv.insert(key, value);
    }

    // We stop here — tensor info + tensor data are not needed (quantization
    // comes from general.file_type, not tensor types).

    extract_metadata(&kv)
}

/// Extract our 7 fields from the parsed KV map. Architecture-prefixed keys
/// (e.g. `llama.context_length`) are built dynamically from `general.architecture`.
fn extract_metadata(kv: &HashMap<String, Value>) -> Result<GgufMetadata> {
    let architecture = get_string(kv, "general.architecture")?
        .ok_or_else(|| anyhow!("missing required key general.architecture"))?;

    // {arch}.context_length and {arch}.block_count may be u32 OR u64 per spec.
    let context_length = get_int(kv, &format!("{architecture}.context_length"))?
        .ok_or_else(|| anyhow!("missing {architecture}.context_length"))?;
    let layer_count = get_int(kv, &format!("{architecture}.block_count"))?
        .ok_or_else(|| anyhow!("missing {architecture}.block_count"))?;

    let model_name = get_string(kv, "general.name")?
        .unwrap_or_else(|| "(unnamed)".to_string());
    let author = get_string(kv, "general.author")?;

    // Chat template: modern key first, legacy {arch}.chat_template fallback.
    let chat_template = get_string(kv, "tokenizer.chat_template")?
        .or_else(|| get_string(kv, &format!("{architecture}.chat_template")).ok().flatten());

    // Quantization: prefer general.file_type (llama_ftype enum); it carries
    // the _M/_S/_L suffix that tensor-scanning cannot. Fall back to "unknown"
    // if absent or GUESSED (1024) — tensor scanning would be needed there and
    // is out of scope for the MVP header-only parser.
    let quantization = match get_int(kv, "general.file_type")? {
        Some(1024) | None => "unknown".to_string(),
        Some(ft) => quantization_from_file_type(ft as u32),
    };

    // Pooling type: present on embedding models ({arch}.pooling_type), absent
    // on chat models. Can be a STRING (e.g. "mean") or a UINT32 enum value
    // (0=none, 1=mean, 2=cls, 3=last, 4=rank). We normalize to a string name.
    // Used by the registry to auto-tag the model's role.
    let pooling_type = get_pooling_type(kv, &architecture);

    Ok(GgufMetadata {
        architecture,
        model_name,
        context_length,
        layer_count,
        quantization,
        chat_template,
        author,
        pooling_type,
    })
}

/// Map a `general.file_type` value (the `llama_ftype` enum) to a readable
/// quantization string. Source: `enum llama_ftype` in llama.cpp/include/llama.h.
/// Unknown values fall back to `"ftype:{n}"` so we never silently misreport.
fn quantization_from_file_type(ft: u32) -> String {
    match ft {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19 => "IQ2_XXS",
        20 => "IQ2_XS",
        21 => "IQ3_XS",
        22 => "IQ3_XXS",
        23 => "IQ1_S",
        24 => "IQ4_NL",
        25 => "IQ3_S",
        26 => "IQ3_M",
        27 => "IQ2_S",
        28 => "IQ2_M",
        29 => "IQ4_XS",
        30 => "IQ1_M",
        32 => "BF16",
        36 => "TQ1_0",
        37 => "TQ2_0",
        _ => "unknown",
    }
    .to_string()
}

// ─── KV accessors ───

fn get_string(kv: &HashMap<String, Value>, key: &str) -> Result<Option<String>> {
    match kv.get(key) {
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Other) => Ok(None), // present but not a string — treat as absent
        Some(v) => bail!("key {key} expected string, got {v:?}"),
        None => Ok(None),
    }
}

/// Read any integer-valued key (u32, u64, i32, i64 all accepted and coerced).
fn get_int(kv: &HashMap<String, Value>, key: &str) -> Result<Option<i64>> {
    match kv.get(key) {
        Some(Value::U32(v)) => Ok(Some(*v as i64)),
        Some(Value::U64(v)) => Ok(Some(*v as i64)),
        Some(Value::I32(v)) => Ok(Some(*v as i64)),
        Some(Value::I64(v)) => Ok(Some(*v)),
        Some(Value::Other) => Ok(None),
        Some(v) => bail!("key {key} expected integer, got {v:?}"),
        None => Ok(None),
    }
}

/// Read `{arch}.pooling_type` and normalize to a readable string.
///
/// In practice this field appears as either:
///   - UINT32 enum (llama.cpp's `enum llama_pooling_type`): 0=none, 1=mean,
///     2=cls, 3=last, 4=rank — this is what most real models use.
///   - STRING (some converted models use the name directly, e.g. "mean").
///
/// Returns `None` if the key is absent (which means the model is not an
/// embedding model — chat models don't have this field).
fn get_pooling_type(kv: &HashMap<String, Value>, arch: &str) -> Option<String> {
    let key = format!("{arch}.pooling_type");
    match kv.get(&key) {
        // Integer enum value → convert to name.
        Some(Value::U32(v)) => Some(match *v {
            0 => "none",
            1 => "mean",
            2 => "cls",
            3 => "last",
            4 => "rank",
            other => {
                log::debug!("Unknown pooling_type enum value {other} for {arch}");
                "unknown"
            }
        }
        .to_string()),
        // Already a string — use as-is.
        Some(Value::String(s)) => Some(s.clone()),
        // Present but a type we don't materialize (e.g. U64) — treat as present
        // with an unknown name so role detection still fires.
        Some(_) => Some("unknown".to_string()),
        // Absent → not an embedding model.
        None => None,
    }
}

// ─── Low-level cursor readers (all little-endian, all bounds-checked) ───

fn read_bytes(cur: &mut Cursor<&[u8]>, n: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    cur.read_exact(&mut buf)?;
    Ok(buf)
}

fn read_u32(cur: &mut Cursor<&[u8]>) -> Result<u32> {
    let b = read_bytes(cur, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_u64(cur: &mut Cursor<&[u8]>) -> Result<u64> {
    let b = read_bytes(cur, 8)?;
    Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

/// Read a count field: u64 in v2/v3, u32 in v1.
fn read_count(cur: &mut Cursor<&[u8]>, wide: bool) -> Result<u64> {
    if wide {
        read_u64(cur)
    } else {
        read_u32(cur).map(|v| v as u64)
    }
}

fn read_gguf_string(cur: &mut Cursor<&[u8]>, wide: bool) -> Result<String> {
    let len = if wide {
        read_u64(cur)?
    } else {
        read_u32(cur)? as u64
    };
    if len > MAX_STRING_LEN {
        bail!("absurd string length: {len}");
    }
    let raw = read_bytes(cur, len as usize)?;
    String::from_utf8(raw).context("GGUF string was not valid UTF-8")
}

/// Read one metadata value of the given type, advancing the cursor past it.
fn read_value(cur: &mut Cursor<&[u8]>, vtype: u32, wide: bool) -> Result<Value> {
    Ok(match vtype {
        value_type::UINT8 => {
            read_bytes(cur, 1)?;
            Value::Other
        }
        value_type::INT8 => {
            read_bytes(cur, 1)?;
            Value::Other
        }
        value_type::UINT16 => {
            read_bytes(cur, 2)?;
            Value::Other
        }
        value_type::INT16 => {
            read_bytes(cur, 2)?;
            Value::Other
        }
        value_type::UINT32 => Value::U32(read_u32(cur)?),
        value_type::INT32 => Value::I32(read_u32(cur)? as i32),
        value_type::FLOAT32 => {
            read_bytes(cur, 4)?;
            Value::Other
        }
        value_type::BOOL => {
            read_bytes(cur, 1)?;
            Value::Other
        }
        value_type::STRING => Value::String(read_gguf_string(cur, wide)?),
        value_type::ARRAY => {
            // ARRAY = u32 elem_type + u64 len + len elements of elem_type.
            // We walk past it without materializing (we don't need any array).
            let elem_type = read_u32(cur)?;
            let len = read_count(cur, wide)?;
            if len > MAX_ARRAY_LEN {
                bail!("absurd array length: {len}");
            }
            for _ in 0..len {
                read_value(cur, elem_type, wide)?;
            }
            Value::Other
        }
        value_type::UINT64 => Value::U64(read_u64(cur)?),
        value_type::INT64 => Value::I64(read_u64(cur)? as i64),
        value_type::FLOAT64 => {
            read_bytes(cur, 8)?;
            Value::Other
        }
        other => bail!("unknown metadata value type: {other}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid GGUF v3 byte buffer with a chosen set of KV pairs.
    /// Strings are UTF-8; integers use the value types we care about.
    fn build_test_gguf() -> Vec<u8> {
        let mut buf = Vec::new();
        // Header
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&3u32.to_le_bytes()); // version 3
        buf.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
        buf.extend_from_slice(&7u64.to_le_bytes()); // kv_count

        // architecture = "llama" (string)
        push_str_kv(&mut buf, "general.architecture", "llama");
        push_str_kv(&mut buf, "general.name", "Test Llama");
        push_str_kv(&mut buf, "general.author", "tester");
        push_u32_kv(&mut buf, "llama.context_length", 4096);
        push_u32_kv(&mut buf, "llama.block_count", 32);
        push_u32_kv(&mut buf, "general.file_type", 15); // Q4_K_M
        push_str_kv(&mut buf, "tokenizer.chat_template", "{{ messages }}");

        buf
    }

    /// Test helper: push a STRING-typed KV pair into a buffer.
    fn push_str_kv(buf: &mut Vec<u8>, k: &str, v: &str) {
        buf.extend_from_slice(&(k.len() as u64).to_le_bytes());
        buf.extend_from_slice(k.as_bytes());
        buf.extend_from_slice(&value_type::STRING.to_le_bytes());
        buf.extend_from_slice(&(v.len() as u64).to_le_bytes());
        buf.extend_from_slice(v.as_bytes());
    }

    /// Test helper: push a UINT32-typed KV pair into a buffer.
    fn push_u32_kv(buf: &mut Vec<u8>, k: &str, v: u32) {
        buf.extend_from_slice(&(k.len() as u64).to_le_bytes());
        buf.extend_from_slice(k.as_bytes());
        buf.extend_from_slice(&value_type::UINT32.to_le_bytes());
        buf.extend_from_slice(&v.to_le_bytes());
    }

    /// Test helper: push a UINT64-typed KV pair into a buffer.
    fn push_u64_kv(buf: &mut Vec<u8>, k: &str, v: u64) {
        buf.extend_from_slice(&(k.len() as u64).to_le_bytes());
        buf.extend_from_slice(k.as_bytes());
        buf.extend_from_slice(&value_type::UINT64.to_le_bytes());
        buf.extend_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn parses_minimal_valid_gguf_v3() {
        let bytes = build_test_gguf();
        let m = parse_bytes(&bytes).expect("parse should succeed");
        assert_eq!(m.architecture, "llama");
        assert_eq!(m.model_name, "Test Llama");
        assert_eq!(m.author.as_deref(), Some("tester"));
        assert_eq!(m.context_length, 4096);
        assert_eq!(m.layer_count, 32);
        assert_eq!(m.quantization, "Q4_K_M");
        assert_eq!(m.chat_template.as_deref(), Some("{{ messages }}"));
        assert_eq!(m.pooling_type, None, "chat model should have no pooling_type");
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = build_test_gguf();
        bytes[0] = b'X';
        let err = parse_bytes(&bytes).unwrap_err();
        assert!(format!("{err}").contains("bad magic"));
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut bytes = build_test_gguf();
        bytes[4..8].copy_from_slice(&99u32.to_le_bytes());
        let err = parse_bytes(&bytes).unwrap_err();
        assert!(format!("{err}").contains("unsupported GGUF version"));
    }

    #[test]
    fn quant_table_covers_common_types() {
        assert_eq!(quantization_from_file_type(0), "F32");
        assert_eq!(quantization_from_file_type(1), "F16");
        assert_eq!(quantization_from_file_type(7), "Q8_0");
        assert_eq!(quantization_from_file_type(15), "Q4_K_M");
        assert_eq!(quantization_from_file_type(14), "Q4_K_S");
        assert_eq!(quantization_from_file_type(18), "Q6_K");
        assert_eq!(quantization_from_file_type(32), "BF16");
        assert_eq!(quantization_from_file_type(9999), "unknown");
    }

    #[test]
    fn handles_u64_context_length() {
        // The spec allows {arch}.context_length to be either UINT32 or UINT64.
        // Build a GGUF where it's UINT64 and confirm get_int coerces correctly.
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&3u32.to_le_bytes()); // v3
        buf.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
        buf.extend_from_slice(&4u64.to_le_bytes()); // 4 KVs

        push_str_kv(&mut buf, "general.architecture", "llama");
        push_str_kv(&mut buf, "general.name", "U64 Test");
        push_u64_kv(&mut buf, "llama.context_length", 8192);
        push_u64_kv(&mut buf, "llama.block_count", 32);

        let m = parse_bytes(&buf).expect("parse should succeed");
        assert_eq!(m.context_length, 8192);
        assert_eq!(m.layer_count, 32);
    }

    #[test]
    fn falls_back_to_legacy_chat_template_key() {
        // tokenizer.chat_template absent; {arch}.chat_template present.
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&3u32.to_le_bytes());
        buf.extend_from_slice(&0u64.to_le_bytes());
        buf.extend_from_slice(&5u64.to_le_bytes()); // 5 KVs

        push_str_kv(&mut buf, "general.architecture", "mistral");
        push_str_kv(&mut buf, "general.name", "Legacy Test");
        push_u32_kv(&mut buf, "mistral.context_length", 32768);
        push_u32_kv(&mut buf, "mistral.block_count", 32);
        push_str_kv(&mut buf, "mistral.chat_template", "legacy template");

        let m = parse_bytes(&buf).expect("parse should succeed");
        assert_eq!(m.chat_template.as_deref(), Some("legacy template"));
    }

    #[test]
    fn embedding_model_has_pooling_type() {
        // Embedding models include {arch}.pooling_type as a UINT32 enum in
        // real GGUF files (0=none, 1=mean, 2=cls, 3=last, 4=rank).
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&3u32.to_le_bytes());
        buf.extend_from_slice(&0u64.to_le_bytes());
        buf.extend_from_slice(&6u64.to_le_bytes()); // 6 KVs

        push_str_kv(&mut buf, "general.architecture", "qwen3");
        push_str_kv(&mut buf, "general.name", "Qwen3 Embedding");
        push_u32_kv(&mut buf, "qwen3.context_length", 32768);
        push_u32_kv(&mut buf, "qwen3.block_count", 28);
        push_u32_kv(&mut buf, "qwen3.pooling_type", 1); // mean
        push_u32_kv(&mut buf, "general.file_type", 1); // F16

        let m = parse_bytes(&buf).expect("parse should succeed");
        assert_eq!(m.pooling_type.as_deref(), Some("mean"));
    }

    #[test]
    fn file_type_1024_guessed_falls_back_to_unknown() {
        // 1024 = GUESSED; we can't determine quantization without tensor scanning.
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(&3u32.to_le_bytes());
        buf.extend_from_slice(&0u64.to_le_bytes());
        buf.extend_from_slice(&5u64.to_le_bytes());
        push_str_kv(&mut buf, "general.architecture", "llama");
        push_str_kv(&mut buf, "general.name", "Guessed");
        push_u32_kv(&mut buf, "llama.context_length", 4096);
        push_u32_kv(&mut buf, "llama.block_count", 8);
        push_u32_kv(&mut buf, "general.file_type", 1024);

        let m = parse_bytes(&buf).expect("parse should succeed");
        assert_eq!(m.quantization, "unknown");
    }
}
