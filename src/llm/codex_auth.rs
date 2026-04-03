//! Refresh OpenAI Codex ChatGPT OAuth access tokens.

use std::path::Path;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

/// OAuth token refresh endpoint (same as Codex CLI).
const REFRESH_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// OAuth client ID used for token refresh (same as Codex CLI).
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// Request body for OAuth token refresh.
#[derive(Serialize)]
struct RefreshRequest<'a> {
    client_id: &'a str,
    grant_type: &'a str,
    refresh_token: &'a str,
}

/// Response from the OAuth token refresh endpoint.
#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: SecretString,
    refresh_token: Option<SecretString>,
}

/// Attempt to refresh an expired access token using the refresh token.
///
/// On success, returns the new `access_token` and persists the refreshed
/// tokens back to `auth.json`. This follows the same OAuth protocol as
/// Codex CLI (`POST https://auth.openai.com/oauth/token`).
///
/// Returns `None` if the refresh token is missing, the request fails,
/// or the response is malformed.
pub async fn refresh_access_token(
    client: &reqwest::Client,
    refresh_token: &SecretString,
    auth_path: Option<&Path>,
) -> Option<SecretString> {
    let req = RefreshRequest {
        client_id: CLIENT_ID,
        grant_type: "refresh_token",
        refresh_token: refresh_token.expose_secret(),
    };

    tracing::info!("Attempting to refresh Codex OAuth access token");

    let resp = match client
        .post(REFRESH_TOKEN_URL)
        .header("Content-Type", "application/json")
        .json(&req)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Token refresh request failed: {e}");
            return None;
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!("Token refresh failed: HTTP {status}: {body}");
        if status.as_u16() == 401 {
            tracing::warn!(
                "Refresh token may be expired or revoked. \
                 Please re-authenticate with: codex --login"
            );
        }
        return None;
    }

    let refresh_resp: RefreshResponse = match resp.json().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Failed to parse token refresh response: {e}");
            return None;
        }
    };

    let new_access_token = refresh_resp.access_token.clone();

    // Persist refreshed tokens back to auth.json
    if let Some(path) = auth_path {
        if let Err(e) = persist_refreshed_tokens(
            path,
            refresh_resp.access_token.expose_secret(),
            refresh_resp
                .refresh_token
                .as_ref()
                .map(ExposeSecret::expose_secret),
        ) {
            tracing::warn!(
                "Failed to persist refreshed tokens to {}: {e}",
                path.display()
            );
        } else {
            tracing::info!("Refreshed tokens persisted to {}", path.display());
        }
    }

    Some(new_access_token)
}

/// Update `auth.json` with refreshed tokens, preserving other fields.
fn persist_refreshed_tokens(
    path: &Path,
    new_access_token: &str,
    new_refresh_token: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(path)?;
    let mut json: serde_json::Value = serde_json::from_str(&content)?;

    if let Some(tokens) = json.get_mut("tokens") {
        tokens["access_token"] = serde_json::Value::String(new_access_token.to_string());
        if let Some(rt) = new_refresh_token {
            tokens["refresh_token"] = serde_json::Value::String(rt.to_string());
        }
    }

    let updated = serde_json::to_string_pretty(&json)?;
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, updated)?;
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(Box::new(e));
    }
    set_auth_file_permissions(path)?;
    Ok(())
}

#[cfg(unix)]
fn set_auth_file_permissions(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_auth_file_permissions(_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}
