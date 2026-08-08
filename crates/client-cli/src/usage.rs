//! Usage limits fetching for supported providers.
//!
//! Claude: Fetches usage data from the OAuth usage endpoint to determine
//! how close the user is to their weekly/hourly limits.
//!
//! Codex: Fetches usage data from the ChatGPT backend usage endpoint.
//!
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use shared::{UsageLimitWindow, UsageLimits};
use std::fs;
use std::path::{Path, PathBuf};

/// Anthropic OAuth usage API endpoint
const USAGE_API_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Codex/ChatGPT usage API endpoint
const CODEX_USAGE_API_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const USAGE_CACHE_FILE: &str = "usage_limits_cache.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct UsageCacheFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claude: Option<UsageLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    codex: Option<UsageLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deepseek: Option<UsageLimits>,
}

impl UsageCacheFile {
    fn get(&self, provider: UsageProvider) -> Option<UsageLimits> {
        match provider {
            UsageProvider::Claude => self.claude.clone(),
            UsageProvider::Codex => self.codex.clone(),
            UsageProvider::Deepseek => self.deepseek.clone(),
        }
    }

    fn set(&mut self, provider: UsageProvider, limits: UsageLimits) {
        match provider {
            UsageProvider::Claude => self.claude = Some(limits),
            UsageProvider::Codex => self.codex = Some(limits),
            UsageProvider::Deepseek => self.deepseek = Some(limits),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum UsageProvider {
    Claude,
    Codex,
    Deepseek,
}

fn usage_cache_dir() -> PathBuf {
    ProjectDirs::from("", "", "apas")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("/tmp/apas"))
}

fn usage_cache_path() -> PathBuf {
    usage_cache_dir().join(USAGE_CACHE_FILE)
}

fn read_usage_cache_file(path: &Path) -> Result<UsageCacheFile> {
    let content = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read usage cache {}: {}", path.display(), e))?;
    let cache: UsageCacheFile = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse usage cache {}: {}", path.display(), e))?;
    Ok(cache)
}

fn read_usage_cache_file_or_default(path: &Path) -> Result<UsageCacheFile> {
    if !path.exists() {
        return Ok(UsageCacheFile::default());
    }
    read_usage_cache_file(path)
}

fn read_usage_cache() -> Result<UsageCacheFile> {
    read_usage_cache_file_or_default(&usage_cache_path())
}

fn write_usage_cache_file(path: &Path, cache: &UsageCacheFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            anyhow::anyhow!(
                "Failed to create usage cache directory {}: {}",
                parent.display(),
                e
            )
        })?;
    }
    let content = serde_json::to_string_pretty(cache)?;
    fs::write(&path, content)
        .map_err(|e| anyhow::anyhow!("Failed to write usage cache {}: {}", path.display(), e))?;
    Ok(())
}

fn write_usage_cache(cache: &UsageCacheFile) -> Result<()> {
    write_usage_cache_file(&usage_cache_path(), cache)
}

fn cache_usage_limits_file(
    path: &Path,
    provider: UsageProvider,
    limits: &UsageLimits,
) -> Result<()> {
    let mut cache = read_usage_cache_file_or_default(path).unwrap_or_default();
    cache.set(provider, limits.clone());
    write_usage_cache_file(path, &cache)
}

fn cache_usage_limits(provider: UsageProvider, limits: &UsageLimits) -> Result<()> {
    let mut cache = read_usage_cache().unwrap_or_default();
    cache.set(provider, limits.clone());
    write_usage_cache(&cache)
}

fn get_cached_usage_limits_file(path: &Path, provider: UsageProvider) -> Option<UsageLimits> {
    let cache = read_usage_cache_file_or_default(path).ok()?;
    cache.get(provider)
}

fn get_cached_usage_limits(provider: UsageProvider) -> Option<UsageLimits> {
    get_cached_usage_limits_file(&usage_cache_path(), provider)
}

fn parse_fetched_at(limits: &UsageLimits) -> Option<DateTime<Utc>> {
    let raw = limits.fetched_at.as_ref()?;
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn is_cache_fresh(limits: &UsageLimits, max_age: Duration) -> bool {
    let fetched_at = match parse_fetched_at(limits) {
        Some(ts) => ts,
        None => return false,
    };
    let age = Utc::now().signed_duration_since(fetched_at);
    age <= max_age
}

fn get_cached_usage_limits_with_max_age(
    provider: UsageProvider,
    max_age: Option<Duration>,
) -> Option<UsageLimits> {
    get_cached_usage_limits_with_max_age_file(&usage_cache_path(), provider, max_age)
}

fn get_cached_usage_limits_with_max_age_file(
    path: &Path,
    provider: UsageProvider,
    max_age: Option<Duration>,
) -> Option<UsageLimits> {
    let cached = get_cached_usage_limits_file(path, provider)?;
    match max_age {
        Some(max_age) if !is_cache_fresh(&cached, max_age) => None,
        _ => Some(cached),
    }
}

pub fn read_cached_claude_usage_limits(max_age: Option<Duration>) -> Option<UsageLimits> {
    get_cached_usage_limits_with_max_age(UsageProvider::Claude, max_age)
}

pub fn read_cached_codex_usage_limits(max_age: Option<Duration>) -> Option<UsageLimits> {
    get_cached_usage_limits_with_max_age(UsageProvider::Codex, max_age)
}

pub fn read_cached_deepseek_usage_limits(max_age: Option<Duration>) -> Option<UsageLimits> {
    get_cached_usage_limits_with_max_age(UsageProvider::Deepseek, max_age)
}

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
pub async fn refresh_claude_usage_limits() -> Result<UsageLimits> {
    let limits = fetch_claude_usage_limits_remote().await?;
    if let Err(e) = cache_usage_limits(UsageProvider::Claude, &limits) {
        tracing::debug!("Failed to cache Claude usage limits: {}", e);
    }
    Ok(limits)
}

pub async fn fetch_claude_usage_limits() -> Result<UsageLimits> {
    match refresh_claude_usage_limits().await {
        Ok(limits) => Ok(limits),
        Err(fetch_error) => {
            if let Some(cached) = get_cached_usage_limits(UsageProvider::Claude) {
                tracing::warn!(
                    "Using cached Claude usage limits after fetch failure: {}",
                    fetch_error
                );
                Ok(cached)
            } else {
                Err(fetch_error)
            }
        }
    }
}

async fn fetch_claude_usage_limits_remote() -> Result<UsageLimits> {
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
pub async fn refresh_codex_usage_limits() -> Result<UsageLimits> {
    let limits = fetch_codex_usage_limits_remote().await?;
    if let Err(e) = cache_usage_limits(UsageProvider::Codex, &limits) {
        tracing::debug!("Failed to cache Codex usage limits: {}", e);
    }
    Ok(limits)
}

pub async fn fetch_codex_usage_limits() -> Result<UsageLimits> {
    match refresh_codex_usage_limits().await {
        Ok(limits) => Ok(limits),
        Err(fetch_error) => {
            if let Some(cached) = get_cached_usage_limits(UsageProvider::Codex) {
                tracing::warn!(
                    "Using cached Codex usage limits after fetch failure: {}",
                    fetch_error
                );
                Ok(cached)
            } else {
                Err(fetch_error)
            }
        }
    }
}

async fn fetch_codex_usage_limits_remote() -> Result<UsageLimits> {
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
    use tempfile::tempdir;

    fn test_limits(five_hour_utilization: f64, seven_day_utilization: f64) -> UsageLimits {
        UsageLimits {
            five_hour: Some(UsageLimitWindow {
                utilization: five_hour_utilization,
                resets_at: Some("2026-06-18T12:00:00Z".to_string()),
            }),
            seven_day: Some(UsageLimitWindow {
                utilization: seven_day_utilization,
                resets_at: Some("2026-06-25T12:00:00Z".to_string()),
            }),
            fetched_at: Some(Utc::now().to_rfc3339()),
        }
    }

    fn cache_file_path() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().expect("creates temp dir");
        let path = dir.path().join("usage_limits_cache.json");
        (dir, path)
    }

    #[test]
    fn test_credentials_path() {
        // Just test that the function doesn't panic
        let _ = get_credentials_path();
    }

    #[test]
    fn cache_freshness_checks_fetched_at() {
        let fresh = UsageLimits {
            five_hour: None,
            seven_day: None,
            fetched_at: Some(Utc::now().to_rfc3339()),
        };
        assert!(is_cache_fresh(&fresh, Duration::minutes(45)));
        assert!(!is_cache_fresh(&fresh, Duration::seconds(-1)));

        let stale = UsageLimits {
            five_hour: None,
            seven_day: None,
            fetched_at: Some((Utc::now() - Duration::hours(3)).to_rfc3339()),
        };
        assert!(!is_cache_fresh(&stale, Duration::minutes(45)));

        let unknown = UsageLimits {
            five_hour: None,
            seven_day: None,
            fetched_at: None,
        };
        assert!(!is_cache_fresh(&unknown, Duration::minutes(45)));
    }

    #[test]
    fn usage_cache_keeps_provider_slots_isolated() {
        let (_dir, path) = cache_file_path();
        let claude = test_limits(0.11, 0.12);
        let codex = test_limits(0.21, 0.22);
        let deepseek = test_limits(0.31, 0.32);

        cache_usage_limits_file(&path, UsageProvider::Claude, &claude).expect("caches Claude");
        cache_usage_limits_file(&path, UsageProvider::Codex, &codex).expect("caches Codex");
        cache_usage_limits_file(&path, UsageProvider::Deepseek, &deepseek)
            .expect("caches DeepSeek");

        assert_eq!(
            get_cached_usage_limits_file(&path, UsageProvider::Claude),
            Some(claude)
        );
        assert_eq!(
            get_cached_usage_limits_file(&path, UsageProvider::Codex),
            Some(codex)
        );
        assert_eq!(
            get_cached_usage_limits_file(&path, UsageProvider::Deepseek),
            Some(deepseek)
        );
    }

    #[test]
    fn usage_cache_update_preserves_other_provider_slots() {
        let (_dir, path) = cache_file_path();
        let claude = test_limits(0.11, 0.12);
        let codex = test_limits(0.21, 0.22);
        let updated_codex = test_limits(0.91, 0.92);
        let deepseek = test_limits(0.31, 0.32);

        cache_usage_limits_file(&path, UsageProvider::Claude, &claude).expect("caches Claude");
        cache_usage_limits_file(&path, UsageProvider::Codex, &codex).expect("caches Codex");
        cache_usage_limits_file(&path, UsageProvider::Deepseek, &deepseek)
            .expect("caches DeepSeek");
        cache_usage_limits_file(&path, UsageProvider::Codex, &updated_codex)
            .expect("updates Codex");

        assert_eq!(
            get_cached_usage_limits_file(&path, UsageProvider::Claude),
            Some(claude)
        );
        assert_eq!(
            get_cached_usage_limits_file(&path, UsageProvider::Codex),
            Some(updated_codex)
        );
        assert_eq!(
            get_cached_usage_limits_file(&path, UsageProvider::Deepseek),
            Some(deepseek)
        );
    }

    #[test]
    fn usage_cache_freshness_filter_is_provider_specific() {
        let (_dir, path) = cache_file_path();
        let stale_claude = UsageLimits {
            fetched_at: Some((Utc::now() - Duration::hours(3)).to_rfc3339()),
            ..test_limits(0.11, 0.12)
        };
        let fresh_codex = test_limits(0.21, 0.22);
        let fresh_deepseek = test_limits(0.31, 0.32);

        cache_usage_limits_file(&path, UsageProvider::Claude, &stale_claude)
            .expect("caches stale Claude");
        cache_usage_limits_file(&path, UsageProvider::Codex, &fresh_codex)
            .expect("caches fresh Codex");
        cache_usage_limits_file(&path, UsageProvider::Deepseek, &fresh_deepseek)
            .expect("caches fresh DeepSeek");

        assert_eq!(
            get_cached_usage_limits_with_max_age_file(
                &path,
                UsageProvider::Claude,
                Some(Duration::minutes(45))
            ),
            None
        );
        assert_eq!(
            get_cached_usage_limits_with_max_age_file(
                &path,
                UsageProvider::Codex,
                Some(Duration::minutes(45))
            ),
            Some(fresh_codex)
        );
        assert_eq!(
            get_cached_usage_limits_with_max_age_file(
                &path,
                UsageProvider::Deepseek,
                Some(Duration::minutes(45))
            ),
            Some(fresh_deepseek)
        );
    }

    #[test]
    fn legacy_retired_cache_slots_are_ignored_and_not_reemitted() {
        let (_dir, path) = cache_file_path();
        std::fs::write(
            &path,
            r#"{"minimax":{"fetched_at":"2026-01-01T00:00:00Z"},"glm":{"fetched_at":"2026-01-01T00:00:00Z"},"codex":null}"#,
        )
        .unwrap();
        let cache = read_usage_cache_file(&path).expect("legacy slots are ignored");
        write_usage_cache_file(&path, &cache).unwrap();
        let saved = std::fs::read_to_string(path).unwrap();
        assert!(!saved.contains("minimax"));
        assert!(!saved.contains("glm"));
    }
}
