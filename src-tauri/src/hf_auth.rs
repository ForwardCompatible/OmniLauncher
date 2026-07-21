//! HuggingFace OAuth device-code authentication.
//!
//! Implements RFC 8628 (Device Authorization Grant) against HuggingFace's
//! `/oauth/device` and `/oauth/token` endpoints, plus keychain storage of the
//! resulting access token and a cached username.
//!
//! ## Verified flow (live-probed 2026-07-17)
//!
//! ```text
//! POST https://huggingface.co/oauth/device   body: client_id=<CLIENT_ID>
//!   → 200 { device_code, user_code, verification_uri, expires_in: 300 }
//!    (HF omits `verification_uri_complete` and `interval` — poll every 5s)
//!
//! POST https://huggingface.co/oauth/token
//!   body: grant_type=urn:ietf:params:oauth:grant-type:device_code
//!         &device_code=<...>&client_id=<CLIENT_ID>
//!   → 200 { access_token, token_type:"Bearer", expires_in, scope }   // Granted
//!   → 400 { error: "authorization_pending" }                          // keep polling
//!   → 400 { error: "slow_down" }                                      // +5s interval
//!   → 400 { error: "expired_token" }                                  // terminal
//!   → 400 { error: "access_denied" }                                  // terminal
//! ```
//!
//! After a token is granted, `GET /api/whoami-v2` with the bearer returns
//! `{ name, fullname, email?, orgs, ... }` — we cache `name` so the UI can show
//! "Connected as <name>" without re-fetching on every render.
//!
//! ## Security
//!
//! - The token value is **never logged** — only presence/expiry is logged at INFO.
//! - Tokens live in the OS keychain (GNOME Keyring / Credential Manager), never
//!   in the SQLite DB or on disk in plaintext.
//! - HF issues **no refresh tokens** for this flow — when the token expires, the
//!   user re-runs the device-code flow. We surface expiry proactively.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};

/// The OAuth client_id for the OmniLauncher app (registered as a public app —
/// no client secret). Shared by every end-user; individuals are differentiated
/// by their own access tokens, not by per-user client IDs.
pub const HF_OAUTH_CLIENT_ID: &str = "92992f79-e30e-4a1e-b20c-b904b3231622";

/// Scopes requested. This list MUST match HF's accepted enum — the device-code
/// endpoint rejects unknown scope names with HTTP 400. Verified against the
/// server's own error message (which enumerates the full valid set):
///   openid | profile | email | read-repos | gated-repos | contribute-repos |
///   write-repos | manage-repos | read-mcp | read-collections | write-collections |
///   write-discussions | read-billing | inference-api | jobs | webhooks |
///   read-network-security | write-network-security
///
/// For OmniLauncher's use (search public models + download, incl. gated repos
/// the user has been granted access to) we need:
///   - `read-repos`: read public repos (search + non-gated downloads)
///   - `gated-repos`: read gated repos the user has access to
///   - `openid profile`: so whoami-v2 returns the username for the badge
const HF_OAUTH_SCOPES: &str = "openid profile read-repos gated-repos";

/// How often the frontend should poll `/oauth/token` (RFC 8628 default when the
/// server omits `interval` — HF does omit it).
pub const POLL_INTERVAL: Duration = Duration::from_secs(5);

// ───────────────────── Keychain storage ─────────────────────

/// Keychain-backed credential storage for the OAuth token and cached username.
///
/// Service `"OmniLauncher"`, accounts `"hf_oauth_token"` / `"hf_username"`.
/// On headless Linux without a keyring daemon, all ops fail gracefully —
/// `load_*` return `None`, `save_*` return an error the UI surfaces.
const KEYRING_SERVICE: &str = "OmniLauncher";
const KEYRING_ACCOUNT_TOKEN: &str = "hf_oauth_token";
const KEYRING_ACCOUNT_USER: &str = "hf_username";

pub struct HfCredentials;

impl HfCredentials {
    /// Load the stored access token. `None` if absent or keyring unavailable.
    /// Never panics. A real backend failure (vs. "never stored") is logged at
    /// DEBUG so it's diagnosable without spamming normal logs.
    pub fn load_token() -> Option<String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT_TOKEN).ok()?;
        match entry.get_password() {
            Ok(v) => Some(v),
            Err(e) => {
                // NoEntry is expected (user never signed in); other errors
                // indicate a backend problem worth logging.
                log::debug!("load_token: keyring read returned {e}");
                None
            }
        }
    }

    /// Store the access token. Fails (human-readable) if keyring unavailable.
    pub fn save_token(token: &str) -> Result<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT_TOKEN)
            .map_err(|e| anyhow!("OS keychain unavailable: {e}"))?;
        entry
            .set_password(token)
            .map_err(|e| anyhow!("Could not save token to keychain: {e}"))
    }

    /// Clear the stored access token. Ok if there was none.
    pub fn clear_token() -> Result<()> {
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT_TOKEN) {
            let _ = entry.delete_credential();
        }
        Ok(())
    }

    /// Load the cached username (from whoami). `None` if absent/never-set.
    pub fn load_username() -> Option<String> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT_USER).ok()?;
        match entry.get_password() {
            Ok(v) => Some(v),
            Err(e) => {
                log::debug!("load_username: keyring read returned {e}");
                None
            }
        }
    }

    /// Cache the username so the UI shows "Connected as <name>" without a
    /// whoami round-trip on every render.
    pub fn save_username(name: &str) -> Result<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT_USER)
            .map_err(|e| anyhow!("OS keychain unavailable: {e}"))?;
        entry
            .set_password(name)
            .map_err(|e| anyhow!("Could not save username to keychain: {e}"))
    }

    /// Clear the cached username.
    pub fn clear_username() -> Result<()> {
        if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT_USER) {
            let _ = entry.delete_credential();
        }
        Ok(())
    }

    /// `true` if the keyring backend is constructible (best-effort probe).
    /// NOTE: this only confirms `Entry::new()` succeeds — it does NOT verify
    /// the backend can actually store/retrieve. With the platform-native
    /// backends enabled (sync-secret-service on Linux, windows-native on
    /// Windows), this is accurate on desktop; on headless Linux without a
    /// keyring daemon it may return true while actual reads/writes fail.
    pub fn is_available() -> bool {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT_TOKEN).is_ok()
    }
}

// ───────────────────── Device-code request ─────────────────────

/// The initial device-authorization response. Returned to the frontend so it
/// can display the code + URL and open the verification page in the browser.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceAuthInfo {
    pub device_code: String,
    /// Short human-readable code the user enters at `verification_uri`.
    pub user_code: String,
    /// URL the user visits (HF returns the short `https://hf.co/oauth/device`).
    pub verification_uri: String,
    /// Lifetime of the device_code/user_code, in seconds (HF: 300).
    pub expires_in: u64,
}

/// Request a device code. The user must then visit `verification_uri` and enter
/// `user_code` within `expires_in` seconds.
pub async fn request_device_code(client: &reqwest::Client) -> Result<DeviceAuthInfo> {
    let resp = client
        .post("https://huggingface.co/oauth/device")
        .form(&[
            ("client_id", HF_OAUTH_CLIENT_ID),
            ("scope", HF_OAUTH_SCOPES),
        ])
        .send()
        .await
        .context("Device-code request failed")?;
    if !resp.status().is_success() {
        return Err(anyhow!(
            "Device-code endpoint returned HTTP {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }
    resp.json::<DeviceAuthInfo>()
        .await
        .context("Could not parse device-code response")
    // Note: HF does NOT return `verification_uri_complete` or `interval`, so we
    // rely on the RFC 8628 defaults (poll every POLL_INTERVAL).
}

// ───────────────────── Token poll ─────────────────────

/// One access token + its expiry. The token value is never serialized to logs.
#[derive(Debug, Clone)]
pub struct AccessToken {
    /// The bearer token. Sensitive — do not log.
    pub value: String,
    /// Unix epoch seconds at which the token expires.
    pub expires_at: u64,
}

impl AccessToken {
    /// Seconds until expiry. Saturates to 0 if already past.
    pub fn seconds_until_expiry(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.expires_at.saturating_sub(now)
    }

    /// `true` if within `soon` seconds of expiry (or already expired).
    pub fn expires_within(&self, soon: u64) -> bool {
        self.seconds_until_expiry() <= soon
    }
}

/// The outcome of one poll iteration. The command layer loops on `Pending` /
/// `SlowDown`, surfaces the terminal states to the user.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PollOutcome {
    /// User authorized; token captured + stored, username fetched.
    Granted {
        username: String,
        expires_at: u64,
    },
    /// User hasn't completed authorization yet — keep polling.
    Pending,
    /// Server asked us to slow down — increase interval by 5s.
    SlowDown,
    /// The 5-min device-code window elapsed — re-start the flow.
    Expired,
    /// User rejected, or account not permitted to authorize this app.
    Denied {
        message: String,
    },
}

/// Poll the token endpoint once. Maps the response to a [`PollOutcome`].
///
/// On `Granted`, also stores the token + fetches+caches the username via
/// `whoami`, so the frontend gets the username in one round-trip.
pub async fn poll_for_token(client: &reqwest::Client, device_code: &str) -> Result<PollOutcome> {
    let resp = client
        .post("https://huggingface.co/oauth/token")
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", device_code),
            ("client_id", HF_OAUTH_CLIENT_ID),
        ])
        .send()
        .await
        .context("Token poll request failed")?;

    let status = resp.status();
    if status.is_success() {
        let tok = resp
            .json::<TokenSuccess>()
            .await
            .context("Could not parse token response")?;
        let access = AccessToken::from_response(&tok)?;
        HfCredentials::save_token(&access.value)
            .context("Token granted but could not be saved to keychain")?;
        // Fetch + cache the username. If whoami fails, the token is still valid —
        // surface as Granted with a placeholder rather than failing the whole flow.
        let username = match whoami(client, &access.value).await {
            Ok(name) => {
                // Username save is non-critical (the token is what matters),
                // but a failure should be visible — log instead of silently
                // discarding with `let _ =`.
                if let Err(e) = HfCredentials::save_username(&name) {
                    log::warn!("Token saved but could not cache username: {e}");
                }
                name
            }
            Err(e) => {
                log::warn!("Token granted but whoami failed: {e}");
                "(unknown)".to_string()
            }
        };
        log::info!(
            "HuggingFace OAuth granted for {}; expires in {}s",
            username,
            access.seconds_until_expiry()
        );
        return Ok(PollOutcome::Granted {
            username,
            expires_at: access.expires_at,
        });
    }

    // Error response — HF returns 400 with {"error": "..."} for all poll states.
    let err = resp
        .json::<TokenError>()
        .await
        .unwrap_or_else(|_| TokenError {
            error: "unknown".into(),
        });
    Ok(match err.error.as_str() {
        "authorization_pending" => PollOutcome::Pending,
        "slow_down" => PollOutcome::SlowDown,
        "expired_token" => PollOutcome::Expired,
        "access_denied" => PollOutcome::Denied {
            message: "You declined the authorization request.".into(),
        },
        "declined_access" => PollOutcome::Denied {
            message: "Your account is not permitted to authorize this app.".into(),
        },
        other => PollOutcome::Denied {
            message: format!("Authorization failed: {other}"),
        },
    })
}

#[derive(Deserialize)]
struct TokenSuccess {
    access_token: String,
    expires_in: u64,
}

impl AccessToken {
    fn from_response(resp: &TokenSuccess) -> Result<Self> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Ok(AccessToken {
            value: resp.access_token.clone(),
            expires_at: now + resp.expires_in,
        })
    }
}

#[derive(Deserialize)]
struct TokenError {
    error: String,
}

// ───────────────────── whoami ─────────────────────

/// Fetch the authenticated user's identity. Returns the short `name` (username).
/// Verified shape: `GET /api/whoami-v2` → `{ name, fullname, email?, orgs, ... }`.
async fn whoami(client: &reqwest::Client, token: &str) -> Result<String> {
    let mut headers = HeaderMap::new();
    if let Ok(mut v) = HeaderValue::from_str(&format!("Bearer {token}")) {
        v.set_sensitive(true);
        headers.insert(AUTHORIZATION, v);
    }
    let resp = client
        .get("https://huggingface.co/api/whoami-v2")
        .headers(headers)
        .send()
        .await
        .context("whoami request failed")?;
    if !resp.status().is_success() {
        return Err(anyhow!("whoami returned HTTP {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.context("Could not parse whoami")?;
    let name = v
        .get("name")
        .and_then(|n| n.as_str())
        .map(String::from)
        .ok_or_else(|| anyhow!("whoami response missing 'name' field"))?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_success_parses_to_expiry() {
        let resp = TokenSuccess {
            access_token: "hf_test".into(),
            expires_in: 3600,
        };
        let tok = AccessToken::from_response(&resp).unwrap();
        assert_eq!(tok.value, "hf_test");
        // expires_at ≈ now + 3600, within a 5s tolerance.
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!((tok.expires_at as i64 - now as i64 - 3600).abs() < 5);
        assert!(tok.seconds_until_expiry() > 3590);
        assert!(!tok.expires_within(300));
    }

    #[test]
    fn expires_within_detects_soon_expiry() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let tok = AccessToken {
            value: "x".into(),
            expires_at: now + 120, // 2 minutes out
        };
        assert!(tok.expires_within(300)); // 120 < 300
        assert!(!tok.expires_within(60)); // 120 > 60
    }

    #[test]
    fn expired_token_saturates_to_zero() {
        let tok = AccessToken {
            value: "x".into(),
            expires_at: 1, // long past
        };
        assert_eq!(tok.seconds_until_expiry(), 0);
        assert!(tok.expires_within(300));
    }

    #[test]
    fn client_id_is_the_registered_one() {
        // Guard against accidentally swapping the client_id during refactors.
        assert_eq!(HF_OAUTH_CLIENT_ID, "92992f79-e30e-4a1e-b20c-b904b3231622");
    }
}
