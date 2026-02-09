//! Usage limits fetching for Claude and Codex
//!
//! Claude: Fetches usage data from the OAuth usage endpoint to determine
//! how close the user is to their weekly/hourly limits.
//!
//! Codex: Fetches usage data from the ChatGPT backend usage endpoint.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use shared::{UsageLimitWindow, UsageLimits};
use std::path::PathBuf;

/// Anthropic OAuth usage API endpoint
const USAGE_API_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Codex/ChatGPT usage API endpoint
const CODEX_USAGE_API_URL: &str = "https://chatgpt.com/backend-api/wham/usage";

/// OAuth credentials from Claude's credentials file
#[derive(Debug, Deserialize)]
struct ClaudeCredentials {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: Option<OAuthToken>,
}

#[derive(Debug, Deserialize)]
struct OAuthToken {
    #[serde(rename = "accessToken")]
    access_token: String,
    // Other fields like refreshToken, expiresAt, scopes exist but we don't need them
}

/// Response from the Anthropic usage API
#[derive(Debug, Deserialize)]
struct UsageApiResponse {
    five_hour: Option<UsageWindow>,
    seven_day: Option<UsageWindow>,
}

#[derive(Debug, Deserialize)]
struct UsageWindow {
    utilization: f64,
    resets_at: Option<String>,
}

/// Get the path to Claude's credentials file
fn get_credentials_path() -> Option<PathBuf> {
    // Try ~/.claude/.credentials.json first
    if let Some(home) = dirs::home_dir() {
        let path = home.join(".claude").join(".credentials.json");
        if path.exists() {
            return Some(path);
        }
    }
    None
}

/// Read the OAuth access token from Claude's credentials file
fn read_oauth_token() -> Result<String> {
    // First check for environment variable override
    if let Ok(token) = std::env::var("CLAUDE_CODE_OAUTH_TOKEN") {
        return Ok(token);
    }

    // Otherwise read from credentials file
    let path = get_credentials_path()
        .ok_or_else(|| anyhow::anyhow!("Claude credentials file not found"))?;

    let contents = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Failed to read credentials file: {}", e))?;

    let credentials: ClaudeCredentials = serde_json::from_str(&contents)
        .map_err(|e| anyhow::anyhow!("Failed to parse credentials file: {}", e))?;

    credentials
        .claude_ai_oauth
        .and_then(|oauth| Some(oauth.access_token))
        .ok_or_else(|| anyhow::anyhow!("No OAuth token found in credentials"))
}

/// Fetch usage limits from the Anthropic API
pub async fn fetch_claude_usage_limits() -> Result<UsageLimits> {
    let token = read_oauth_token()?;

    let client = reqwest::Client::new();
    let response = client
        .get(USAGE_API_URL)
        .header("Authorization", format!("Bearer {}", token))
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch usage: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "Usage API returned error {}: {}",
            status,
            body
        ));
    }

    let api_response: UsageApiResponse = response
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to parse usage response: {}", e))?;

    let now = chrono::Utc::now().to_rfc3339();

    Ok(UsageLimits {
        // API returns utilization as percentage (0-100), convert to fraction (0-1)
        five_hour: api_response.five_hour.map(|w| UsageLimitWindow {
            utilization: w.utilization / 100.0,
            resets_at: w.resets_at,
        }),
        seven_day: api_response.seven_day.map(|w| UsageLimitWindow {
            utilization: w.utilization / 100.0,
            resets_at: w.resets_at,
        }),
        fetched_at: Some(now),
    })
}

// ----------------------- Codex usage limits -----------------------

#[derive(Debug, Deserialize)]
struct CodexAuthFile {
    #[serde(default)]
    tokens: Option<CodexAuthTokens>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexAuthTokens {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexUsageApiResponse {
    #[serde(default)]
    rate_limit: Option<CodexRateLimit>,
}

#[derive(Debug, Deserialize)]
struct CodexRateLimit {
    #[serde(default)]
    primary_window: Option<CodexRateLimitWindow>,
    #[serde(default)]
    secondary_window: Option<CodexRateLimitWindow>,
}

#[derive(Debug, Deserialize)]
struct CodexRateLimitWindow {
    used_percent: f64,
    #[serde(default)]
    reset_at: Option<i64>,
    #[serde(default)]
    reset_after_seconds: Option<i64>,
}

fn codex_auth_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CODEX_AUTH_FILE") {
        if !path.trim().is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    if let Ok(home) = std::env::var("CODEX_HOME") {
        return Some(PathBuf::from(home).join("auth.json"));
    }
    dirs::home_dir().map(|home| home.join(".codex").join("auth.json"))
}

fn read_codex_auth() -> Result<(String, Option<String>)> {
    let account_override = std::env::var("CODEX_ACCOUNT_ID")
        .ok()
        .filter(|v| !v.trim().is_empty());
    if let Some(token_override) = std::env::var("CODEX_ACCESS_TOKEN")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        return Ok((token_override, account_override));
    }

    let path =
        codex_auth_path().ok_or_else(|| anyhow::anyhow!("Codex auth file path not found"))?;

    let contents = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("Failed to read Codex auth file {}: {}", path.display(), e))?;
    let auth: CodexAuthFile = serde_json::from_str(&contents).map_err(|e| {
        anyhow::anyhow!("Failed to parse Codex auth file {}: {}", path.display(), e)
    })?;

    let access_token = auth
        .tokens
        .as_ref()
        .and_then(|t| t.access_token.clone())
        .or(auth.access_token)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No access token found in Codex auth file {}",
                path.display()
            )
        })?;

    let account_id = account_override
        .or_else(|| auth.tokens.as_ref().and_then(|t| t.account_id.clone()))
        .or(auth.account_id);

    Ok((access_token, account_id))
}

fn unix_seconds_to_rfc3339(unix_seconds: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp(unix_seconds, 0).map(|dt| dt.to_rfc3339())
}

fn map_codex_window(window: CodexRateLimitWindow, now: &DateTime<Utc>) -> UsageLimitWindow {
    let resets_at = window
        .reset_at
        .and_then(unix_seconds_to_rfc3339)
        .or_else(|| {
            window
                .reset_after_seconds
                .map(|s| (now.clone() + Duration::seconds(s)).to_rfc3339())
        });

    UsageLimitWindow {
        utilization: window.used_percent / 100.0,
        resets_at,
    }
}

/// Fetch usage limits from Codex API endpoint
pub async fn fetch_codex_usage_limits() -> Result<UsageLimits> {
    let (access_token, account_id) = read_codex_auth()?;
    let client = reqwest::Client::new();

    let mut request = client
        .get(CODEX_USAGE_API_URL)
        .bearer_auth(access_token)
        .header("User-Agent", "codex-cli");

    if let Some(account_id) = account_id {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    let response = request
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to fetch Codex usage: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "Codex usage API returned error {}: {}",
            status,
            body
        ));
    }

    let payload: CodexUsageApiResponse = response
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to parse Codex usage response: {}", e))?;

    let rate_limit = payload
        .rate_limit
        .ok_or_else(|| anyhow::anyhow!("Codex usage response missing rate_limit"))?;

    let now = Utc::now();
    let five_hour = rate_limit.primary_window.map(|w| map_codex_window(w, &now));
    let seven_day = rate_limit
        .secondary_window
        .map(|w| map_codex_window(w, &now));

    if five_hour.is_none() && seven_day.is_none() {
        return Err(anyhow::anyhow!(
            "Codex usage response does not include window data"
        ));
    }

    Ok(UsageLimits {
        five_hour,
        seven_day,
        fetched_at: Some(now.to_rfc3339()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_path() {
        // Just test that the function doesn't panic
        let _ = get_credentials_path();
    }
}
