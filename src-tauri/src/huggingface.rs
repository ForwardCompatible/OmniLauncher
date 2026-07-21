//! HuggingFace Hub client — search, file listing, streaming download.
//!
//! Built against verified API shapes (live-probed 2026-07-17):
//! - Search: `/api/models?full=true` returns a bare JSON array. Each hit has
//!   `id, author, downloads, likes, pipeline_tag, lastModified, gated, tags,
//!   trendingScore`. GGUF filtering uses **`tags=gguf`** (not `library=gguf`,
//!   which is unreliable). Pagination is cursor-based via `link: rel="next"`;
//!   `offset` is silently ignored and `X-Total-Count` is never emitted.
//! - Files: `/api/models/{repo}` `siblings[].rfilename` lists files (no size).
//!   Per-file size requires a HEAD to `/resolve/` reading `x-linked-size`.
//! - Download: `/{repo}/resolve/main/<file>` 302-redirects to a signed CDN URL
//!   (expires in minutes), supports `accept-ranges: bytes`.
//! - Gated repos: return `401` with `x-error-code: GatedRepo` and a `text/plain`
//!   body (NOT JSON) — detect via the header, never body parsing.
//! - Rate limits: IETF `ratelimit` header (`r=` remaining, `t=` reset seconds).
//!   `retry-after` only appears on actual 429s.

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use percent_encoding::percent_decode_str;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;

use crate::hf_auth::HfCredentials;

const HF_BASE: &str = "https://huggingface.co";

/// Default number of results per search page (HF max is ~1000; 30 balances
/// payload size with browseable page size).
const SEARCH_PAGE_SIZE: &str = "30";

/// HTTP connect timeout for HF requests. Generous enough for slow DNS/TLS
/// handshakes without blocking indefinitely.
const HF_CONNECT_TIMEOUT_SECS: u64 = 15;

/// Default backoff seconds when rate-limited and the `ratelimit` header's `t=`
/// reset value can't be parsed.
const DEFAULT_RATELIMIT_RESET_SECS: u64 = 30;

// ───────────────────── DTOs (verified shapes) ─────────────────────

/// One search hit. Fields verified against the live `/api/models?full=true`
/// response. `gated` may be a bool (`false`) or a string (`"auto"`/`"manual"`)
/// depending on the repo's gating mode — we normalize to a `Gated` enum.
///
/// `has_gguf_files` is parsed from the `siblings` array (available via
/// `full=true`) and used to validate the leaky `tags=gguf` server filter —
/// HF's tag filter returns ~40% false positives (transformers repos without
/// any .gguf files), so we drop them here before returning to the frontend.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HfModelResult {
    pub id: String,
    pub author: Option<String>,
    pub downloads: Option<u64>,
    pub likes: Option<u64>,
    pub pipeline_tag: Option<String>,
    pub last_modified: Option<String>,
    pub trending_score: Option<f64>,
    pub gated: Gated,
    /// `true` if the repo's `siblings` contain at least one `.gguf` file.
    pub has_gguf_files: bool,
}

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Gated {
    /// `gated: false` — open access.
    No,
    /// `gated: "auto"` — gated but auto-approved on request.
    Auto,
    /// `gated: "manual"` — gated, requires manual approval.
    Manual,
    /// `gated` field absent or unrecognized.
    Unknown,
}

impl Gated {
    /// `true` for any gating mode (UI shows a "gated" badge + warns on download).
    pub fn is_gated(self) -> bool {
        !matches!(self, Gated::No)
    }
}

/// One page of search results. `next_cursor` is the opaque cursor for the next
/// page (from the `link: rel="next"` header); `None` when there are no more.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HfSearchPage {
    pub results: Vec<HfModelResult>,
    pub next_cursor: Option<String>,
}

/// One `.gguf` file in a repo. `size_bytes` is fetched lazily via a HEAD (the
/// siblings array doesn't carry per-file sizes).
#[derive(Debug, Clone, serde::Serialize)]
pub struct HfFile {
    pub filename: String,
    pub size_bytes: Option<u64>,
}

/// Repo-level GGUF metadata (architecture, context, total size). Read from the
/// `gguf` dict on the single-repo endpoint.
#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct HfRepoGgufMeta {
    pub architecture: Option<String>,
    pub context_length: Option<u64>,
    pub total_file_size: Option<u64>,
}

// ───────────────────── Client ─────────────────────

/// HTTP client for the HuggingFace Hub. The `reqwest::Client` is held behind a
/// `RwLock` so the OAuth token can be swapped after auth without `&mut` through
/// the `Arc<HfClient>` in managed state.
pub struct HfClient {
    client: RwLock<reqwest::Client>,
}

impl HfClient {
    /// Build with the current keychain token (if any) attached as a default
    /// `Authorization: Bearer` header.
    pub fn new() -> Self {
        Self {
            client: RwLock::new(build_client()),
        }
    }

    /// Rebuild the inner client after the token changes (login/logout).
    pub async fn refresh_token(&self) {
        let new_client = build_client();
        *self.client.write().await = new_client;
    }

    /// Search the Hub. All filter params are optional; the command layer maps
    /// the UI controls to these.
    pub async fn search(
        &self,
        query: &str,
        sort: &str,
        pipeline_tag: Option<&str>,
        gguf_only: bool,
        cursor: Option<&str>,
    ) -> Result<HfSearchPage> {
        let client = self.client.read().await.clone();
        let mut req = client
            .get(format!("{HF_BASE}/api/models"))
            .query(&[("full", "true"), ("sort", sort), ("direction", "-1")])
            .query(&[("limit", SEARCH_PAGE_SIZE)]);
        if !query.is_empty() {
            req = req.query(&[("search", query)]);
        }
        if let Some(tag) = pipeline_tag.filter(|s| !s.is_empty()) {
            req = req.query(&[("pipeline_tag", tag)]);
        }
        if gguf_only {
            // tags=gguf narrows the candidate set but is LEAKY — live probing
            // showed ~40% false positives (transformers repos without any .gguf
            // files slip through, e.g. "thinkingmachines/Inkling"). We still send
            // it to reduce the server-side result set, then validate each result
            // by checking its siblings for actual .gguf files (below).
            // library=gguf is NOT used — real GGUF repos have library_name=
            // "llama.cpp", not "gguf", so that filter is even worse.
            req = req.query(&[("tags", "gguf")]);
        }
        if let Some(c) = cursor.filter(|s| !s.is_empty()) {
            req = req.query(&[("cursor", c)]);
        }

        let resp = req.send().await.context("HuggingFace search request failed")?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format_hf_error(status, resp).await);
        }

        // The `link: rel="next"` header carries the cursor for the next page.
        // Format: `<https://...&cursor=eyJ...>; rel="next"` (or absent on last page).
        let next_cursor = extract_next_cursor(resp.headers());
        let raw: Vec<serde_json::Value> =
            resp.json().await.context("Could not parse search response")?;
        let raw_count = raw.len();
        let mut results: Vec<HfModelResult> = raw.iter().map(parse_model_result).collect();

        // Secondary GGUF validation: drop false positives from the leaky
        // tags=gguf server filter. We already have siblings from full=true, so
        // this costs zero extra requests.
        if gguf_only {
            let before = results.len();
            results.retain(|r| r.has_gguf_files);
            log::debug!(
                "HF search: query={:?} sort={} gguf={} cursor={} → raw={} gguf_filtered={} (dropped {})",
                query, sort, gguf_only, cursor.is_some(), raw_count, results.len(), before - results.len()
            );
        } else {
            log::debug!(
                "HF search: query={:?} sort={} gguf=false cursor={} → results={}",
                query, sort, cursor.is_some(), results.len()
            );
        }

        Ok(HfSearchPage { results, next_cursor })
    }

    /// List the `.gguf` files in a repo, plus the repo-level GGUF metadata.
    pub async fn list_files(&self, repo_id: &str) -> Result<(Vec<HfFile>, HfRepoGgufMeta)> {
        let client = self.client.read().await.clone();
        let resp = client
            .get(format!("{HF_BASE}/api/models/{repo_id}"))
            .send()
            .await
            .with_context(|| format!("File-list request failed for {repo_id}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format_hf_error(status, resp).await);
        }
        let v: serde_json::Value =
            resp.json().await.context("Could not parse repo response")?;

        // siblings[].rfilename — NO size field (verified).
        let mut files = Vec::new();
        if let Some(siblings) = v.get("siblings").and_then(|s| s.as_array()) {
            for s in siblings {
                let name = s.get("rfilename").and_then(|n| n.as_str()).unwrap_or("");
                if name.ends_with(".gguf") {
                    files.push(HfFile {
                        filename: name.to_string(),
                        size_bytes: None, // fetched lazily via head_file_size
                    });
                }
            }
        }

        // Repo-level gguf metadata dict (camelCase keys in HF's response).
        let meta = parse_gguf_meta(v.get("gguf"));
        Ok((files, meta))
    }

    /// HEAD the `/resolve/` URL to read a file's size (`x-linked-size` header
    /// on the 302 hop). Cheap; called once per file when the user expands a repo.
    pub async fn head_file_size(&self, repo_id: &str, filename: &str) -> Result<u64> {
        let client = self.client.read().await.clone();
        let url = format!("{HF_BASE}/{repo_id}/resolve/main/{filename}");
        // Don't follow redirects — the size is on the 302 hop's `x-linked-size`.
        let resp = client
            .request(reqwest::Method::HEAD, &url)
            .send()
            .await
            .with_context(|| format!("HEAD failed for {filename}"))?;
        let status = resp.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(gated_error(repo_id));
        }
        if !status.is_success() && status.as_u16() != 302 {
            return Err(format_hf_error(status, resp).await);
        }
        // Prefer x-linked-size (present on the HF 302); fall back to content-length.
        resp.headers()
            .get("x-linked-size")
            .or_else(|| resp.headers().get("content-length"))
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| anyhow!("No size header for {filename}"))
    }

    /// Fetch the repo's README (model card) as raw markdown text.
    ///
    /// Endpoint: `GET https://huggingface.co/{repo}/raw/main/README.md` (verified:
    /// returns `text/plain`, ~10KB, no auth for public repos). For gated repos
    /// the bearer token is attached automatically by the client.
    ///
    /// Returns `Ok("")` on 404 (repo has no README). On 401/403 (gated without
    /// access), returns a clear error so the UI can explain.
    pub async fn fetch_readme(&self, repo_id: &str) -> Result<String> {
        let client = self.client.read().await.clone();
        let url = format!("{HF_BASE}/{repo_id}/raw/main/README.md");
        let resp = client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("README request failed for {repo_id}"))?;
        let status = resp.status();
        if status.as_u16() == 404 {
            return Ok(String::new());
        }
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(gated_error(repo_id));
        }
        if !status.is_success() {
            return Err(format_hf_error(status, resp).await);
        }
        resp.text().await.context("Could not read README response")
    }

    /// Stream-download `filename` from `repo_id` into `dest_part` (a `.part`
    /// path). Emits progress via the callback; checks cancellation via
    /// `should_cancel`. Handles 401-gated, 429-rate-limited, and generic errors.
    ///
    /// Returns total bytes downloaded on success.
    pub async fn download(
        &self,
        repo_id: &str,
        filename: &str,
        dest_part: &Path,
        on_progress: impl Fn(u64, Option<u64>),
        mut should_cancel: impl FnMut() -> bool,
    ) -> Result<u64> {
        let url = format!("{HF_BASE}/{repo_id}/resolve/main/{filename}");
        let client = self.client.read().await.clone();

            let mut attempt = 0u32;
            const MAX_ATTEMPTS: u32 = 3;
            loop {
                attempt += 1;
            let mut resp = client
                .get(&url)
                .send()
                .await
                .with_context(|| format!("Download request failed for {filename}"))?;
            let status = resp.status();

            // Gated: 401 with x-error-code: GatedRepo (body is text/plain, not JSON).
            if status.as_u16() == 401 || status.as_u16() == 403 {
                if resp.headers().get("x-error-code").map(|v| v.to_str().unwrap_or(""))
                    == Some("GatedRepo")
                {
                    return Err(gated_error(repo_id));
                }
                return Err(anyhow!(
                    "Access denied (HTTP {status}). Check your HuggingFace connection in Settings."
                ));
            }

            // Rate-limited: back off using the IETF `ratelimit` header's `t=` reset
            // seconds, or a default. `retry-after` may or may not be present.
            if status.as_u16() == 429 {
                if attempt >= MAX_ATTEMPTS {
                    return Err(anyhow!(
                        "Rate limited by HuggingFace after {MAX_ATTEMPTS} retries"
                    ));
                }
                let wait = parse_ratelimit_reset(resp.headers()).unwrap_or(DEFAULT_RATELIMIT_RESET_SECS);
                log::warn!("HuggingFace rate-limited a download; retrying in {wait}s");
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }

            if !status.is_success() {
                return Err(format_hf_error(status, resp).await);
            }

            let total = resp.content_length();
            let mut file = tokio::fs::File::create(dest_part)
                .await
                .with_context(|| format!("Could not create {}", dest_part.display()))?;
            let mut downloaded: u64 = 0;

            loop {
                if should_cancel() {
                    drop(file);
                    let _ = tokio::fs::remove_file(dest_part).await;
                    return Err(anyhow!("Cancelled"));
                }
                let chunk = match resp.chunk().await {
                    Ok(Some(c)) => c,
                    Ok(None) => break,
                    Err(e) => {
                        drop(file);
                        let _ = tokio::fs::remove_file(dest_part).await;
                        return Err(e).context("Download stream error");
                    }
                };
                file.write_all(&chunk)
                    .await
                    .context("Failed writing download chunk")?;
                downloaded += chunk.len() as u64;
                on_progress(downloaded, total);
            }
            file.flush().await.context("Failed flushing download")?;
            return Ok(downloaded);
        }
    }

    /// Clone the inner reqwest client for use in standalone calls (device-code,
    /// whoami) that don't go through the hub search/download methods.
    pub async fn client_clone(&self) -> reqwest::Client {
        self.client.read().await.clone()
    }
}

impl Default for HfClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a client with the current keychain token baked into default headers.
fn build_client() -> reqwest::Client {
    let mut headers = HeaderMap::new();
    let token = HfCredentials::load_token();
    if let Some(ref t) = token {
        if let Ok(mut v) = HeaderValue::from_str(&format!("Bearer {t}")) {
            v.set_sensitive(true);
            headers.insert(AUTHORIZATION, v);
        }
    }
    // Auth status logged at DEBUG (not INFO) to avoid startup delay from
    // keychain reads on every build_client() call. Auth was confirmed working
    // via live testing; this stays at debug for future diagnostics.
    if token.is_some() {
        log::debug!("HuggingFace client: authenticated (OAuth token attached)");
    } else {
        log::debug!("HuggingFace client: anonymous (no token found in keychain)");
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .user_agent(concat!(
            "OmniLauncher/",
            env!("CARGO_PKG_VERSION"),
            " (https://github.com/ForwardCompatible/OmniLauncher)"
        ))
        .connect_timeout(Duration::from_secs(HF_CONNECT_TIMEOUT_SECS))
        // No overall timeout: downloads can be many GB.
        .build()
        .expect("reqwest client builder must not fail with valid config")
}

// ───────────────────── Helpers ─────────────────────

/// Parse a search-hit JSON object into [`HfModelResult`]. Tolerates missing
/// fields (verified: not all hits populate every field).
fn parse_model_result(v: &serde_json::Value) -> HfModelResult {
    let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let author = v
        .get("author")
        .and_then(|x| x.as_str())
        .map(String::from)
        .or_else(|| id.split_once('/').map(|(a, _)| a.to_string()));
    // Check siblings for at least one .gguf file. This is the secondary
    // validation that catches the ~40% false-positive rate of tags=gguf.
    let has_gguf_files = v
        .get("siblings")
        .and_then(|s| s.as_array())
        .map(|siblings| {
            siblings.iter().any(|s| {
                s.get("rfilename")
                    .and_then(|n| n.as_str())
                    .map(|n| n.ends_with(".gguf"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    HfModelResult {
        id,
        author,
        downloads: v.get("downloads").and_then(|x| x.as_u64()),
        likes: v.get("likes").and_then(|x| x.as_u64()),
        pipeline_tag: v
            .get("pipeline_tag")
            .and_then(|x| x.as_str())
            .map(String::from),
        last_modified: v
            .get("lastModified")
            .and_then(|x| x.as_str())
            .map(String::from),
        trending_score: v.get("trendingScore").and_then(|x| x.as_f64()),
        gated: parse_gated(v.get("gated")),
        has_gguf_files,
    }
}

/// Normalize the `gated` field — bool or string — into [`Gated`].
fn parse_gated(v: Option<&serde_json::Value>) -> Gated {
    match v {
        Some(serde_json::Value::Bool(false)) => Gated::No,
        Some(serde_json::Value::Bool(true)) => Gated::Manual, // true is rare; treat as manual
        Some(serde_json::Value::String(s)) => match s.as_str() {
            "auto" => Gated::Auto,
            "manual" => Gated::Manual,
            "false" => Gated::No,
            _ => Gated::Unknown,
        },
        None => Gated::No, // absent = not gated
        _ => Gated::Unknown,
    }
}

/// Parse the repo-level `gguf` metadata dict.
fn parse_gguf_meta(v: Option<&serde_json::Value>) -> HfRepoGgufMeta {
    let Some(v) = v else {
        return HfRepoGgufMeta::default();
    };
    HfRepoGgufMeta {
        architecture: v.get("architecture").and_then(|x| x.as_str()).map(String::from),
        context_length: v.get("context_length").and_then(|x| x.as_u64()),
        total_file_size: v
            .get("totalFileSize")
            .and_then(|x| x.as_u64())
            .or_else(|| v.get("total").and_then(|x| x.as_u64())),
    }
}

/// Extract the `cursor=` value from a `link: rel="next"` header.
/// Returns `None` if there's no next page.
///
/// The cursor is URL-decoded before returning so that when reqwest's `.query()`
/// re-encodes it, the result is single-encoded (matching what HF expects).
/// Without this decode, cursors containing base64 special chars (`+`, `/`, `=`)
/// get double-encoded (`%2B` → `%252B`) and HF rejects them with HTTP 400
/// "Error parsing pagination cursor".
fn extract_next_cursor(headers: &HeaderMap) -> Option<String> {
    let link = headers.get("link")?.to_str().ok()?;
    // Format: `<https://huggingface.co/api/models?...&cursor=eyJ...>; rel="next"`
    if !link.contains("rel=\"next\"") {
        return None;
    }
    let cursor_start = link.find("cursor=")? + "cursor=".len();
    let rest = &link[cursor_start..];
    // Cursor ends at `>` or `&` or `;`.
    let end = rest
        .find(|c: char| c == '>' || c == '&' || c == ';')
        .unwrap_or(rest.len());
    let raw = &rest[..end];
    // Decode percent-encoded sequences so reqwest's .query() re-encodes exactly
    // once. For pure-alphanumeric cursors this is a no-op.
    Some(percent_decode_str(raw).decode_utf8_lossy().into_owned())
}

/// Parse the IETF `ratelimit` header's `t=` (seconds to reset).
/// Format: `ratelimit: "resolvers";r=2998;t=225` → 225.
fn parse_ratelimit_reset(headers: &HeaderMap) -> Option<u64> {
    let h = headers.get("ratelimit")?.to_str().ok()?;
    let t_idx = h.find("t=")?;
    let rest = &h[t_idx + 2..];
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest[..end].parse::<u64>().ok()
}

/// Human-readable error for a gated repo, with a link to request access.
fn gated_error(repo_id: &str) -> anyhow::Error {
    anyhow!(
        "This model is gated. Visit https://huggingface.co/{repo_id} to request access, \
         then reconnect your HuggingFace account."
    )
}

/// Build a human-readable error from a failed HF response, including the body
/// snippet when useful.
async fn format_hf_error(status: reqwest::StatusCode, resp: reqwest::Response) -> anyhow::Error {
    let body = resp.text().await.unwrap_or_default();
    let snippet = if body.len() > 200 {
        format!("{}…", &body[..200])
    } else {
        body
    };
    anyhow!("HuggingFace returned HTTP {status}: {snippet}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_search_hit() {
        let v = serde_json::json!({"id": "user/model-gguf"});
        let m = parse_model_result(&v);
        assert_eq!(m.id, "user/model-gguf");
        assert_eq!(m.author.as_deref(), Some("user"));
        assert_eq!(m.gated, Gated::No);
        assert!(!m.has_gguf_files, "no siblings → has_gguf_files must be false");
    }

    #[test]
    fn parses_full_search_hit_with_gated_string() {
        let v = serde_json::json!({
            "id": "meta-llama/Llama-3.2-1B",
            "author": "meta-llama",
            "downloads": 12345,
            "likes": 67,
            "pipeline_tag": "text-generation",
            "lastModified": "2026-01-15T00:00:00.000Z",
            "trendingScore": 194,
            "gated": "manual"
        });
        let m = parse_model_result(&v);
        assert_eq!(m.downloads, Some(12345));
        assert_eq!(m.gated, Gated::Manual);
        assert!(m.gated.is_gated());
        assert_eq!(m.trending_score, Some(194.0));
    }

    #[test]
    fn detects_gguf_files_in_siblings() {
        let v = serde_json::json!({
            "id": "user/model-GGUF",
            "siblings": [
                {"rfilename": "config.json"},
                {"rfilename": "model-Q4_K_M.gguf"},
                {"rfilename": "model-Q8_0.gguf"}
            ]
        });
        let m = parse_model_result(&v);
        assert!(m.has_gguf_files, "siblings with .gguf files → must be true");
    }

    #[test]
    fn detects_no_gguf_files_in_siblings() {
        // The false-positive case: a transformers repo that leaks through tags=gguf.
        let v = serde_json::json!({
            "id": "thinkingmachines/Inkling",
            "siblings": [
                {"rfilename": "config.json"},
                {"rfilename": "model.safetensors"},
                {"rfilename": "tokenizer.json"}
            ]
        });
        let m = parse_model_result(&v);
        assert!(!m.has_gguf_files, "no .gguf files → must be false (would be filtered out)");
    }

    #[test]
    fn gated_false_bool_parses_as_no() {
        let v = serde_json::json!({"id": "x", "gated": false});
        assert_eq!(parse_gated(v.get("gated")), Gated::No);
        assert!(!parse_gated(v.get("gated")).is_gated());
    }

    #[test]
    fn gated_auto_parses_as_auto() {
        let v = serde_json::json!({"gated": "auto"});
        assert_eq!(parse_gated(v.get("gated")), Gated::Auto);
        assert!(parse_gated(v.get("gated")).is_gated());
    }

    #[test]
    fn extracts_cursor_from_link_header() {
        let mut h = HeaderMap::new();
        h.insert(
            "link",
            HeaderValue::from_static(
                "<https://huggingface.co/api/models?limit=30&cursor=eyJkb2xpbSI6dHJ1ZX0>; rel=\"next\""
            ),
        );
        assert_eq!(
            extract_next_cursor(&h).as_deref(),
            Some("eyJkb2xpbSI6dHJ1ZX0")
        );
    }

    #[test]
    fn extracts_and_decodes_percent_encoded_cursor() {
        // Cursor with base64 special chars that HF URL-encodes in the link header.
        // %2B = '+', %2F = '/', %3D = '='. Without decoding, reqwest double-encodes
        // these and HF rejects with "Error parsing pagination cursor".
        let mut h = HeaderMap::new();
        h.insert(
            "link",
            HeaderValue::from_static(
                "<https://huggingface.co/api/models?limit=30&cursor=abc%2Bdef%2Fghi%3D>; rel=\"next\""
            ),
        );
        assert_eq!(
            extract_next_cursor(&h).as_deref(),
            Some("abc+def/ghi=")
        );
    }

    #[test]
    fn no_next_link_returns_none() {
        let mut h = HeaderMap::new();
        h.insert("link", HeaderValue::from_static("<https://x>; rel=\"prev\""));
        assert_eq!(extract_next_cursor(&h), None);
    }

    #[test]
    fn missing_link_header_returns_none() {
        assert_eq!(extract_next_cursor(&HeaderMap::new()), None);
    }

    #[test]
    fn parses_ratelimit_reset_seconds() {
        let mut h = HeaderMap::new();
        h.insert("ratelimit", HeaderValue::from_static("\"resolvers\";r=2998;t=225"));
        assert_eq!(parse_ratelimit_reset(&h), Some(225));
    }

    #[test]
    fn parses_gguf_meta_with_camelcase_keys() {
        let v = serde_json::json!({
            "architecture": "llama",
            "context_length": 131072,
            "totalFileSize": 43605014656_i64
        });
        let m = parse_gguf_meta(Some(&v));
        assert_eq!(m.architecture.as_deref(), Some("llama"));
        assert_eq!(m.context_length, Some(131072));
        assert_eq!(m.total_file_size, Some(43605014656));
    }

    #[test]
    fn gguf_meta_absent_returns_default() {
        let m = parse_gguf_meta(None);
        assert!(m.architecture.is_none());
        assert!(m.context_length.is_none());
    }
}
