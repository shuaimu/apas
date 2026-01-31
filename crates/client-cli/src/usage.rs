//! Claude usage limits fetching from Anthropic API
//!
//! Fetches usage data from the OAuth usage endpoint to determine
//! how close the user is to their weekly/hourly limits.

use anyhow::Result;
use serde::Deserialize;
use shared::{UsageLimitWindow, UsageLimits};
use std::path::PathBuf;

/// Anthropic OAuth usage API endpoint
const USAGE_API_URL: &str = "https://api.anthropic.com/api/oauth/usage";

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
pub async fn fetch_usage_limits() -> Result<UsageLimits> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credentials_path() {
        // Just test that the function doesn't panic
        let _ = get_credentials_path();
    }
}
