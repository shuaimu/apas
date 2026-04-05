//! Usage limits fetching for Claude, Codex, MiniMax, and GLM
//!
//! Claude: Fetches usage data from the OAuth usage endpoint to determine
//! how close the user is to their weekly/hourly limits.
//!
//! Codex: Fetches usage data from the ChatGPT backend usage endpoint.
//!
//! MiniMax: Fetches coding-plan remaining quota from the MiniMax remains endpoint.
//!
//! GLM: Fetches coding-plan quota limits from the GLM monitor usage endpoints.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared::{UsageLimitWindow, UsageLimits};
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

/// Anthropic OAuth usage API endpoint
const USAGE_API_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Codex/ChatGPT usage API endpoint
const CODEX_USAGE_API_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
/// MiniMax coding plan remains endpoints (primary + compatibility fallback)
const MINIMAX_USAGE_API_URLS: [&str; 2] = [
    "https://www.minimax.io/v1/api/openplatform/coding_plan/remains",
    "https://www.minimaxi.com/v1/api/openplatform/coding_plan/remains",
];
/// GLM Anthropic-bridge base URL used by APAS runtime.
const GLM_DEFAULT_API_BASE_URL: &str = "https://api.z.ai/api/anthropic";
const GLM_USAGE_MODEL_PATH: &str = "/api/monitor/usage/model-usage";
const GLM_USAGE_TOOL_PATH: &str = "/api/monitor/usage/tool-usage";
const GLM_USAGE_QUOTA_LIMIT_PATH: &str = "/api/monitor/usage/quota/limit";
const USAGE_CACHE_FILE: &str = "usage_limits_cache.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct UsageCacheFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claude: Option<UsageLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    codex: Option<UsageLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    minimax: Option<UsageLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    glm: Option<UsageLimits>,
}

#[derive(Debug, Clone, Copy)]
enum UsageProvider {
    Claude,
    Codex,
    Minimax,
    Glm,
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

fn read_usage_cache() -> Result<UsageCacheFile> {
    let path = usage_cache_path();
    if !path.exists() {
        return Ok(UsageCacheFile::default());
    }
    read_usage_cache_file(&path)
}

fn write_usage_cache(cache: &UsageCacheFile) -> Result<()> {
    let path = usage_cache_path();
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

fn cache_usage_limits(provider: UsageProvider, limits: &UsageLimits) -> Result<()> {
    let mut cache = read_usage_cache().unwrap_or_default();
    match provider {
        UsageProvider::Claude => cache.claude = Some(limits.clone()),
        UsageProvider::Codex => cache.codex = Some(limits.clone()),
        UsageProvider::Minimax => cache.minimax = Some(limits.clone()),
        UsageProvider::Glm => cache.glm = Some(limits.clone()),
    }
    write_usage_cache(&cache)
}

fn get_cached_usage_limits(provider: UsageProvider) -> Option<UsageLimits> {
    let cache = read_usage_cache().ok()?;
    match provider {
        UsageProvider::Claude => cache.claude,
        UsageProvider::Codex => cache.codex,
        UsageProvider::Minimax => cache.minimax,
        UsageProvider::Glm => cache.glm,
    }
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
    let cached = get_cached_usage_limits(provider)?;
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

pub fn read_cached_minimax_usage_limits(max_age: Option<Duration>) -> Option<UsageLimits> {
    get_cached_usage_limits_with_max_age(UsageProvider::Minimax, max_age)
}

pub fn read_cached_glm_usage_limits(max_age: Option<Duration>) -> Option<UsageLimits> {
    get_cached_usage_limits_with_max_age(UsageProvider::Glm, max_age)
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

// ----------------------- GLM usage limits -----------------------

fn monitor_origin_from_url(raw: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(raw).ok()?;
    let host = parsed.host_str()?;
    let mut origin = format!("{}://{}", parsed.scheme(), host);
    if let Some(port) = parsed.port() {
        origin.push(':');
        origin.push_str(&port.to_string());
    }
    Some(origin)
}

fn read_glm_monitor_origin() -> String {
    for env_key in ["GLM_MONITOR_BASE_URL", "GLM_API_BASE_URL"] {
        if let Some(raw) = trim_non_empty(std::env::var(env_key).ok()) {
            if let Some(origin) = monitor_origin_from_url(&raw) {
                return origin;
            }
        }
    }

    let config = crate::config::Config::load().unwrap_or_default();
    if let Some(raw) = trim_non_empty(config.local.glm_api_base_url) {
        if let Some(origin) = monitor_origin_from_url(&raw) {
            return origin;
        }
    }

    monitor_origin_from_url(GLM_DEFAULT_API_BASE_URL)
        .unwrap_or_else(|| "https://api.z.ai".to_string())
}

fn read_glm_api_key() -> Result<String> {
    for env_key in [
        "GLM_API_KEY",
        "GLM_API_TOKEN",
        "ZAI_API_KEY",
        "ZHIPU_API_KEY",
    ] {
        if let Some(value) = trim_non_empty(std::env::var(env_key).ok()) {
            return Ok(value);
        }
    }

    let config = crate::config::Config::load().unwrap_or_default();
    if let Some(value) = trim_non_empty(config.local.glm_api_key) {
        return Ok(value);
    }

    Err(anyhow::anyhow!(
        "GLM API key is not configured. Set glm_api_key in apas config or GLM_API_KEY."
    ))
}

fn normalize_utilization(raw: f64) -> Option<f64> {
    if !raw.is_finite() {
        return None;
    }
    let normalized = if raw > 1.0 { raw / 100.0 } else { raw };
    Some(normalized.clamp(0.0, 1.5))
}

fn format_glm_monitor_window() -> (String, String) {
    let now = Utc::now();
    let start = now - Duration::hours(24);
    (
        start.format("%Y-%m-%d %H:00:00").to_string(),
        now.format("%Y-%m-%d %H:59:59").to_string(),
    )
}

fn glm_auth_candidates(api_key: &str) -> Vec<String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.to_ascii_lowercase().starts_with("bearer ") {
        return vec![trimmed.to_string()];
    }
    vec![trimmed.to_string(), format!("Bearer {}", trimmed)]
}

#[derive(Debug, Clone)]
struct GlmApiError {
    code: Option<i64>,
    message: String,
    auth_failure: bool,
}

fn parse_glm_api_error(payload: &Value) -> Option<GlmApiError> {
    let root = payload.as_object()?;
    let success = root.get("success").and_then(Value::as_bool);
    let code = root
        .get("code")
        .and_then(value_as_f64)
        .map(|value| value as i64);

    let message = root
        .get("msg")
        .and_then(Value::as_str)
        .or_else(|| root.get("message").and_then(Value::as_str))
        .or_else(|| root.get("error").and_then(Value::as_str))
        .or_else(|| root.get("error_message").and_then(Value::as_str))
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| "unknown error".to_string());

    let has_error = matches!(success, Some(false)) || code.map_or(false, |value| value != 0);
    if !has_error {
        return None;
    }

    let normalized_message = message.to_ascii_lowercase();
    let auth_failure = matches!(code, Some(401 | 1001))
        || normalized_message.contains("auth")
        || normalized_message.contains("token");

    Some(GlmApiError {
        code,
        message,
        auth_failure,
    })
}

async fn fetch_glm_usage_payload(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    with_window_query: bool,
) -> Result<Value> {
    let auth_candidates = glm_auth_candidates(api_key);
    if auth_candidates.is_empty() {
        return Err(anyhow::anyhow!("GLM API key is empty"));
    }

    let (start_time, end_time) = format_glm_monitor_window();
    let mut last_error: Option<anyhow::Error> = None;

    for auth in auth_candidates {
        let mut request = client
            .get(url)
            .header("Authorization", auth.clone())
            .header("Accept-Language", "en-US,en")
            .header("Content-Type", "application/json");

        if with_window_query {
            request = request.query(&[
                ("startTime", start_time.as_str()),
                ("endTime", end_time.as_str()),
            ]);
        }

        let response = match request.send().await {
            Ok(resp) => resp,
            Err(err) => {
                last_error = Some(anyhow::anyhow!(
                    "Failed to fetch GLM usage from {}: {}",
                    url,
                    err
                ));
                continue;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            last_error = Some(anyhow::anyhow!(
                "GLM usage API {} returned {}: {}",
                url,
                status,
                body
            ));
            continue;
        }

        match response.json::<Value>().await {
            Ok(payload) => {
                if let Some(api_error) = parse_glm_api_error(&payload) {
                    let code = api_error
                        .code
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    last_error = Some(anyhow::anyhow!(
                        "GLM usage API {} returned code {}: {}",
                        url,
                        code,
                        api_error.message
                    ));
                    if api_error.auth_failure {
                        continue;
                    }
                    continue;
                }
                return Ok(payload);
            }
            Err(err) => {
                last_error = Some(anyhow::anyhow!(
                    "Failed to parse GLM usage response from {}: {}",
                    url,
                    err
                ));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("GLM usage API request failed")))
}

fn parse_glm_usage_limits(payloads: &[Value]) -> Result<UsageLimits> {
    let mut five_hour: Option<UsageLimitWindow> = None;
    let mut last_api_error: Option<GlmApiError> = None;

    for payload in payloads {
        if let Some(api_error) = parse_glm_api_error(payload) {
            last_api_error = Some(api_error);
            continue;
        }

        let mut objects: Vec<&serde_json::Map<String, Value>> = Vec::new();
        if let Some(root) = payload.as_object() {
            objects.push(root);
        }
        if let Some(data) = payload.get("data").and_then(Value::as_object) {
            objects.insert(0, data);
        }

        for source in objects.clone() {
            let limits = source.get("limits").and_then(Value::as_array);
            let Some(entries) = limits else {
                continue;
            };

            let mut fallback_rate: Option<(f64, Option<String>)> = None;

            for entry in entries.iter().filter_map(Value::as_object) {
                let utilization = first_number_from_objects(
                    &[entry],
                    &[
                        "percentage",
                        "used_percent",
                        "utilization",
                        "usage_rate",
                        "percent",
                    ],
                )
                .and_then(normalize_utilization);

                let Some(utilization) = utilization else {
                    continue;
                };

                let resets_at = first_rfc3339_from_objects(
                    &[entry],
                    &[
                        "reset_at",
                        "resets_at",
                        "next_reset_at",
                        "next_reset_time",
                        "reset_time",
                        "expires_at",
                        "expire_at",
                    ],
                );

                let limit_type = entry
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_ascii_uppercase();

                if limit_type.contains("TOKEN") {
                    five_hour = Some(UsageLimitWindow {
                        utilization,
                        resets_at,
                    });
                    break;
                }

                if fallback_rate.is_none() {
                    fallback_rate = Some((utilization, resets_at));
                }
            }

            if five_hour.is_none() {
                if let Some((utilization, resets_at)) = fallback_rate {
                    five_hour = Some(UsageLimitWindow {
                        utilization,
                        resets_at,
                    });
                }
            }

            if five_hour.is_some() {
                break;
            }
        }

        if five_hour.is_none() {
            let utilization = first_number_from_objects(
                &objects,
                &[
                    "percentage",
                    "used_percent",
                    "utilization",
                    "usage_rate",
                    "percent",
                ],
            )
            .and_then(normalize_utilization);
            if let Some(utilization) = utilization {
                let resets_at = first_rfc3339_from_objects(
                    &objects,
                    &[
                        "reset_at",
                        "resets_at",
                        "next_reset_at",
                        "next_reset_time",
                        "reset_time",
                        "expires_at",
                        "expire_at",
                    ],
                );
                five_hour = Some(UsageLimitWindow {
                    utilization,
                    resets_at,
                });
            }
        }

        if five_hour.is_some() {
            break;
        }
    }

    if five_hour.is_none() {
        if let Some(api_error) = last_api_error {
            let code = api_error
                .code
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            return Err(anyhow::anyhow!(
                "GLM usage API returned code {}: {}",
                code,
                api_error.message
            ));
        }

        return Err(anyhow::anyhow!(
            "GLM usage response missing utilization data"
        ));
    }

    Ok(UsageLimits {
        five_hour,
        seven_day: None,
        fetched_at: Some(Utc::now().to_rfc3339()),
    })
}

/// Fetch usage limits from GLM monitor usage endpoints.
pub async fn refresh_glm_usage_limits() -> Result<UsageLimits> {
    let limits = fetch_glm_usage_limits_remote().await?;
    if let Err(e) = cache_usage_limits(UsageProvider::Glm, &limits) {
        tracing::debug!("Failed to cache GLM usage limits: {}", e);
    }
    Ok(limits)
}

pub async fn fetch_glm_usage_limits() -> Result<UsageLimits> {
    match refresh_glm_usage_limits().await {
        Ok(limits) => Ok(limits),
        Err(fetch_error) => {
            if let Some(cached) = get_cached_usage_limits(UsageProvider::Glm) {
                tracing::warn!(
                    "Using cached GLM usage limits after fetch failure: {}",
                    fetch_error
                );
                Ok(cached)
            } else {
                Err(fetch_error)
            }
        }
    }
}

async fn fetch_glm_usage_limits_remote() -> Result<UsageLimits> {
    let api_key = read_glm_api_key()?;
    let monitor_origin = read_glm_monitor_origin();
    let client = reqwest::Client::new();

    let mut last_error: Option<anyhow::Error> = None;

    // Fast path: quota endpoint already exposes token percentage in most cases.
    let quota_url = format!("{}{}", monitor_origin, GLM_USAGE_QUOTA_LIMIT_PATH);
    let mut payloads = match fetch_glm_usage_payload(&client, &quota_url, &api_key, false).await {
        Ok(payload) => {
            if let Ok(parsed) = parse_glm_usage_limits(std::slice::from_ref(&payload)) {
                return Ok(parsed);
            }
            vec![payload]
        }
        Err(err) => {
            last_error = Some(err);
            Vec::new()
        }
    };

    let fallback_endpoints = [
        format!("{}{}", monitor_origin, GLM_USAGE_MODEL_PATH),
        format!("{}{}", monitor_origin, GLM_USAGE_TOOL_PATH),
    ];

    for url in fallback_endpoints {
        match fetch_glm_usage_payload(&client, &url, &api_key, true).await {
            Ok(payload) => payloads.push(payload),
            Err(err) => {
                last_error = Some(err);
            }
        }
    }

    if payloads.is_empty() {
        return Err(last_error.unwrap_or_else(|| anyhow::anyhow!("GLM usage API request failed")));
    }

    parse_glm_usage_limits(&payloads).map_err(|parse_err| last_error.unwrap_or(parse_err))
}

// ----------------------- MiniMax usage limits -----------------------

fn trim_non_empty(raw: Option<String>) -> Option<String> {
    raw.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn read_minimax_api_key() -> Result<String> {
    for env_key in ["MINIMAX_API_KEY", "MINIMAX_API_TOKEN"] {
        if let Some(value) = trim_non_empty(std::env::var(env_key).ok()) {
            return Ok(value);
        }
    }

    let config = crate::config::Config::load().unwrap_or_default();
    if let Some(value) = trim_non_empty(config.local.minimax_api_key) {
        return Ok(value);
    }

    Err(anyhow::anyhow!(
        "MiniMax API key is not configured. Set minimax_api_key in apas config or MINIMAX_API_KEY."
    ))
}

fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn value_as_rfc3339(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return None;
            }
            if DateTime::parse_from_rfc3339(trimmed).is_ok() {
                return Some(trimmed.to_string());
            }
            trimmed.parse::<i64>().ok().and_then(unix_epoch_to_rfc3339)
        }
        Value::Number(number) => number.as_i64().and_then(unix_epoch_to_rfc3339),
        _ => None,
    }
}

fn unix_epoch_to_rfc3339(epoch: i64) -> Option<String> {
    // MiniMax APIs sometimes return milliseconds.
    let seconds = if epoch > 10_000_000_000 {
        epoch / 1000
    } else {
        epoch
    };
    DateTime::<Utc>::from_timestamp(seconds, 0).map(|dt| dt.to_rfc3339())
}

fn first_number_from_objects(
    objects: &[&serde_json::Map<String, Value>],
    keys: &[&str],
) -> Option<f64> {
    for object in objects {
        for key in keys {
            if let Some(value) = object.get(*key).and_then(value_as_f64) {
                return Some(value);
            }
        }
    }
    None
}

fn first_rfc3339_from_objects(
    objects: &[&serde_json::Map<String, Value>],
    keys: &[&str],
) -> Option<String> {
    for object in objects {
        for key in keys {
            if let Some(value) = object.get(*key).and_then(value_as_rfc3339) {
                return Some(value);
            }
        }
    }
    None
}

fn utilization_from_counts(
    objects: &[&serde_json::Map<String, Value>],
    total_keys: &[&str],
    used_keys: &[&str],
    remaining_keys: &[&str],
) -> Option<f64> {
    let total = first_number_from_objects(objects, total_keys)?;
    if total <= 0.0 {
        return None;
    }

    if let Some(used) = first_number_from_objects(objects, used_keys) {
        return Some((used / total).clamp(0.0, 1.5));
    }

    if let Some(remaining) = first_number_from_objects(objects, remaining_keys) {
        let used = (total - remaining).max(0.0);
        return Some((used / total).clamp(0.0, 1.5));
    }

    None
}

fn is_minimax_model_name(raw: &str) -> bool {
    let normalized = raw.trim().to_ascii_lowercase();
    !normalized.is_empty() && (normalized.contains("minimax") || normalized.starts_with("m2"))
}

fn minimax_model_remains_score(entry: &serde_json::Map<String, Value>) -> f64 {
    let weekly_total = first_number_from_objects(
        &[entry],
        &["current_weekly_total_count", "weekly_total_count"],
    )
    .unwrap_or(0.0);
    let interval_total = first_number_from_objects(
        &[entry],
        &["current_interval_total_count", "interval_total_count"],
    )
    .unwrap_or(0.0);
    // Prefer entries that expose useful quota windows (weekly weighted higher).
    (weekly_total * 10.0) + interval_total
}

fn parse_minimax_usage_limits(payload: &Value) -> Result<UsageLimits> {
    // Best-effort API-level error extraction.
    if let Some(root) = payload.as_object() {
        if let Some(code) = root.get("code").and_then(value_as_f64) {
            if code != 0.0 {
                let msg = root
                    .get("message")
                    .and_then(Value::as_str)
                    .or_else(|| root.get("msg").and_then(Value::as_str))
                    .unwrap_or("unknown error");
                return Err(anyhow::anyhow!(
                    "MiniMax usage API returned code {}: {}",
                    code,
                    msg
                ));
            }
        }
        if let Some(base_resp) = root.get("base_resp").and_then(Value::as_object) {
            if let Some(code) = base_resp.get("status_code").and_then(value_as_f64) {
                if code != 0.0 {
                    let msg = base_resp
                        .get("status_message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error");
                    return Err(anyhow::anyhow!(
                        "MiniMax usage API returned status_code {}: {}",
                        code,
                        msg
                    ));
                }
            }
        }
    }

    let mut objects: Vec<&serde_json::Map<String, Value>> = Vec::new();
    if let Some(root) = payload.as_object() {
        objects.push(root);
    }
    if let Some(data) = payload.get("data").and_then(Value::as_object) {
        objects.push(data);
    }

    let mut selected_model_entry: Option<&serde_json::Map<String, Value>> = None;

    // Prefer model-specific remains entry for MiniMax models if present.
    for source in objects.clone() {
        let model_remains = source
            .get("model_remains")
            .or_else(|| source.get("modelRemains"))
            .and_then(Value::as_array);
        let Some(entries) = model_remains else {
            continue;
        };

        let preferred = entries
            .iter()
            .filter_map(Value::as_object)
            .filter(|entry| {
                entry
                    .get("model_name")
                    .or_else(|| entry.get("model"))
                    .and_then(Value::as_str)
                    .map(is_minimax_model_name)
                    .unwrap_or(false)
            })
            .max_by(|left, right| {
                minimax_model_remains_score(left)
                    .partial_cmp(&minimax_model_remains_score(right))
                    .unwrap_or(Ordering::Equal)
            })
            .or_else(|| entries.iter().filter_map(Value::as_object).next());

        if let Some(entry) = preferred {
            selected_model_entry = Some(entry);
            objects.insert(0, entry);
            break;
        }
    }

    let explicit_utilization = first_number_from_objects(
        &objects,
        &[
            "utilization",
            "usage_rate",
            "used_rate",
            "usage_percentage",
            "used_percent",
            "percent",
        ],
    )
    .map(|rate| if rate > 1.0 { rate / 100.0 } else { rate })
    .map(|value| value.clamp(0.0, 1.5));

    let mut five_hour = None;
    let mut seven_day = None;

    if let Some(model_entry) = selected_model_entry {
        let model_objects = [model_entry];

        // MiniMax remains endpoint reports `*_usage_count` as remaining quota
        // for the current window, not consumed usage.
        if let Some(utilization) = utilization_from_counts(
            &model_objects,
            &["current_interval_total_count", "interval_total_count"],
            &[
                "current_interval_used_count",
                "used_count",
                "consumed_count",
            ],
            &[
                "current_interval_usage_count",
                "interval_usage_count",
                "usage_count",
                "current_interval_remaining_count",
                "interval_remaining_count",
                "remaining_count",
                "remain_count",
                "remaining",
                "remain",
            ],
        ) {
            let resets_at = first_rfc3339_from_objects(
                &model_objects,
                &[
                    "end_time",
                    "current_interval_end_time",
                    "next_interval_time",
                    "next_reset_time",
                    "next_reset_at",
                    "reset_at",
                    "resets_at",
                ],
            );
            five_hour = Some(UsageLimitWindow {
                utilization,
                resets_at,
            });
        }

        if let Some(utilization) = utilization_from_counts(
            &model_objects,
            &["current_weekly_total_count", "weekly_total_count"],
            &[
                "current_weekly_used_count",
                "weekly_used_count",
                "used_count",
                "consumed_count",
            ],
            &[
                "current_weekly_usage_count",
                "weekly_usage_count",
                "usage_count",
                "current_weekly_remaining_count",
                "weekly_remaining_count",
                "remaining_count",
                "remain_count",
                "remaining",
                "remain",
            ],
        ) {
            let resets_at = first_rfc3339_from_objects(
                &model_objects,
                &[
                    "weekly_end_time",
                    "current_weekly_end_time",
                    "next_weekly_time",
                    "next_reset_time",
                    "next_reset_at",
                    "reset_at",
                    "resets_at",
                ],
            );
            seven_day = Some(UsageLimitWindow {
                utilization,
                resets_at,
            });
        }
    }

    if five_hour.is_none() {
        if let Some(utilization) = utilization_from_counts(
            &objects,
            &[
                "current_interval_total_count",
                "interval_total_count",
                "total_count",
                "quota_total",
            ],
            &[
                "current_interval_used_count",
                "used_count",
                "consumed_count",
            ],
            &[
                "current_interval_usage_count",
                "interval_usage_count",
                "usage_count",
                "current_interval_remaining_count",
                "interval_remaining_count",
                "remaining_count",
                "remain_count",
                "remaining",
                "remain",
            ],
        ) {
            let resets_at = first_rfc3339_from_objects(
                &objects,
                &[
                    "end_time",
                    "current_interval_end_time",
                    "next_interval_time",
                    "next_reset_time",
                    "next_reset_at",
                    "reset_at",
                    "resets_at",
                ],
            );
            five_hour = Some(UsageLimitWindow {
                utilization,
                resets_at,
            });
        }
    }

    if seven_day.is_none() {
        let fallback_utilization = utilization_from_counts(
            &objects,
            &[
                "current_weekly_total_count",
                "weekly_total_count",
                "seven_day_total_count",
                "total_count",
                "quota_total",
            ],
            &[
                "current_weekly_used_count",
                "weekly_used_count",
                "seven_day_used_count",
                "current_interval_used_count",
                "used_count",
                "consumed_count",
            ],
            &[
                "current_weekly_usage_count",
                "weekly_usage_count",
                "seven_day_usage_count",
                "current_interval_usage_count",
                "interval_usage_count",
                "usage_count",
                "current_weekly_remaining_count",
                "weekly_remaining_count",
                "seven_day_remaining_count",
                "current_interval_remaining_count",
                "interval_remaining_count",
                "remaining_count",
                "remain_count",
                "remaining",
                "remain",
            ],
        )
        .or(explicit_utilization);

        if let Some(utilization) = fallback_utilization {
            let resets_at = first_rfc3339_from_objects(
                &objects,
                &[
                    "weekly_end_time",
                    "current_weekly_end_time",
                    "next_weekly_time",
                    "next_reset_time",
                    "next_reset_at",
                    "end_time",
                    "current_interval_end_time",
                    "next_interval_time",
                    "reset_at",
                    "resets_at",
                ],
            );
            seven_day = Some(UsageLimitWindow {
                utilization,
                resets_at,
            });
        }
    }

    if five_hour.is_none() && seven_day.is_none() {
        return Err(anyhow::anyhow!(
            "MiniMax usage response missing utilization data"
        ));
    }

    let now = Utc::now().to_rfc3339();
    Ok(UsageLimits {
        five_hour,
        seven_day,
        fetched_at: Some(now),
    })
}

/// Fetch usage limits from MiniMax remains endpoint
pub async fn refresh_minimax_usage_limits() -> Result<UsageLimits> {
    let limits = fetch_minimax_usage_limits_remote().await?;
    if let Err(e) = cache_usage_limits(UsageProvider::Minimax, &limits) {
        tracing::debug!("Failed to cache MiniMax usage limits: {}", e);
    }
    Ok(limits)
}

pub async fn fetch_minimax_usage_limits() -> Result<UsageLimits> {
    match refresh_minimax_usage_limits().await {
        Ok(limits) => Ok(limits),
        Err(fetch_error) => {
            if let Some(cached) = get_cached_usage_limits(UsageProvider::Minimax) {
                tracing::warn!(
                    "Using cached MiniMax usage limits after fetch failure: {}",
                    fetch_error
                );
                Ok(cached)
            } else {
                Err(fetch_error)
            }
        }
    }
}

async fn fetch_minimax_usage_limits_remote() -> Result<UsageLimits> {
    let api_key = read_minimax_api_key()?;
    let client = reqwest::Client::new();
    let mut last_error: Option<anyhow::Error> = None;

    for url in MINIMAX_USAGE_API_URLS {
        let response = match client
            .get(url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(err) => {
                last_error = Some(anyhow::anyhow!(
                    "Failed to fetch MiniMax usage from {}: {}",
                    url,
                    err
                ));
                continue;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            last_error = Some(anyhow::anyhow!(
                "MiniMax usage API {} returned {}: {}",
                url,
                status,
                body
            ));
            continue;
        }

        let payload = match response.json::<Value>().await {
            Ok(value) => value,
            Err(err) => {
                last_error = Some(anyhow::anyhow!(
                    "Failed to parse MiniMax usage response from {}: {}",
                    url,
                    err
                ));
                continue;
            }
        };

        match parse_minimax_usage_limits(&payload) {
            Ok(limits) => return Ok(limits),
            Err(err) => {
                last_error = Some(anyhow::anyhow!(
                    "Failed to interpret MiniMax usage response from {}: {}",
                    url,
                    err
                ));
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("MiniMax usage API request failed")))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parse_minimax_usage_limits_from_model_remains() {
        let payload = serde_json::json!({
            "code": 0,
            "model_remains": [
                {
                    "model_name": "MiniMax-M2.7",
                    "current_interval_total_count": 100,
                    "current_interval_usage_count": 30,
                    "end_time": 1743974400000i64,
                    "current_weekly_total_count": 1000,
                    "current_weekly_usage_count": 500,
                    "weekly_end_time": 1744492800000i64
                }
            ]
        });

        let parsed = parse_minimax_usage_limits(&payload).expect("parses minimax payload");
        let weekly = parsed.seven_day.expect("weekly window exists");
        assert!((weekly.utilization - 0.5).abs() < 0.0001);
        assert!(weekly.resets_at.is_some());
        let five_hour = parsed.five_hour.expect("5h window exists");
        assert!((five_hour.utilization - 0.7).abs() < 0.0001);
        assert!(five_hour.resets_at.is_some());
    }

    #[test]
    fn parse_minimax_usage_limits_from_percentage() {
        let payload = serde_json::json!({
            "code": 0,
            "data": {
                "used_percent": 45
            }
        });

        let parsed = parse_minimax_usage_limits(&payload).expect("parses percentage payload");
        let weekly = parsed.seven_day.expect("weekly window exists");
        assert!((weekly.utilization - 0.45).abs() < 0.0001);
    }

    #[test]
    fn parse_minimax_usage_limits_from_live_shape() {
        let payload = serde_json::json!({
            "model_remains": [
                {
                    "start_time": 1775260800000i64,
                    "end_time": 1775278800000i64,
                    "remains_time": 7362522,
                    "current_interval_total_count": 1500,
                    "current_interval_usage_count": 1456,
                    "model_name": "MiniMax-M*",
                    "current_weekly_total_count": 15000,
                    "current_weekly_usage_count": 14954,
                    "weekly_start_time": 1774828800000i64,
                    "weekly_end_time": 1775433600000i64,
                    "weekly_remains_time": 162162522
                }
            ],
            "base_resp": {
                "status_code": 0,
                "status_msg": "success"
            }
        });

        let parsed = parse_minimax_usage_limits(&payload).expect("parses live minimax payload");
        let five_hour = parsed.five_hour.expect("5h window exists");
        let weekly = parsed.seven_day.expect("weekly window exists");
        assert!((five_hour.utilization - ((1500.0 - 1456.0) / 1500.0)).abs() < 0.0001);
        assert!((weekly.utilization - ((15000.0 - 14954.0) / 15000.0)).abs() < 0.0001);
        assert!(five_hour.resets_at.is_some());
        assert!(weekly.resets_at.is_some());
    }

    #[test]
    fn parse_glm_usage_limits_from_quota_limit_payload() {
        let payload = serde_json::json!({
            "data": {
                "limits": [
                    {
                        "type": "TOKENS_LIMIT",
                        "percentage": 42,
                        "reset_at": 1775433600000i64
                    }
                ]
            }
        });

        let parsed = parse_glm_usage_limits(&vec![payload]).expect("parses glm payload");
        let five_hour = parsed.five_hour.expect("5h window exists");
        assert!((five_hour.utilization - 0.42).abs() < 0.0001);
        assert!(five_hour.resets_at.is_some());
        assert!(parsed.seven_day.is_none());
    }

    #[test]
    fn parse_glm_usage_limits_from_top_level_percentage() {
        let payload = serde_json::json!({
            "data": {
                "percentage": 0.55
            }
        });

        let parsed = parse_glm_usage_limits(&vec![payload]).expect("parses glm percentage payload");
        let five_hour = parsed.five_hour.expect("5h window exists");
        assert!((five_hour.utilization - 0.55).abs() < 0.0001);
    }

    #[test]
    fn parse_glm_api_error_detects_auth_failure_in_http_200_payload() {
        let payload = serde_json::json!({
            "code": 1001,
            "msg": "Authentication parameter not received in Header, unable to authenticate",
            "success": false
        });

        let api_error = parse_glm_api_error(&payload).expect("detects API error");
        assert_eq!(api_error.code, Some(1001));
        assert!(api_error.auth_failure);
    }

    #[test]
    fn parse_glm_usage_limits_reports_api_error_when_payload_has_no_usage() {
        let payload = serde_json::json!({
            "code": 401,
            "msg": "token expired or incorrect",
            "success": false
        });

        let err = parse_glm_usage_limits(&vec![payload]).expect_err("parsing should fail");
        let text = err.to_string();
        assert!(text.contains("code 401"));
        assert!(text.contains("token expired or incorrect"));
    }
}
