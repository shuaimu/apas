//! Usage limits fetching for Claude, Codex, and MiniMax
//!
//! Claude: Fetches usage data from the OAuth usage endpoint to determine
//! how close the user is to their weekly/hourly limits.
//!
//! Codex: Fetches usage data from the ChatGPT backend usage endpoint.
//!
//! MiniMax: Fetches coding-plan remaining quota from the MiniMax remains endpoint.

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
const USAGE_CACHE_FILE: &str = "usage_limits_cache.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct UsageCacheFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claude: Option<UsageLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    codex: Option<UsageLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    minimax: Option<UsageLimits>,
}

#[derive(Debug, Clone, Copy)]
enum UsageProvider {
    Claude,
    Codex,
    Minimax,
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
    }
    write_usage_cache(&cache)
}

fn get_cached_usage_limits(provider: UsageProvider) -> Option<UsageLimits> {
    let cache = read_usage_cache().ok()?;
    match provider {
        UsageProvider::Claude => cache.claude,
        UsageProvider::Codex => cache.codex,
        UsageProvider::Minimax => cache.minimax,
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

        if let Some(utilization) = utilization_from_counts(
            &model_objects,
            &["current_interval_total_count", "interval_total_count"],
            &[
                "current_interval_usage_count",
                "interval_usage_count",
                "current_interval_used_count",
                "used_count",
                "usage_count",
                "consumed_count",
            ],
            &[
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
                "current_weekly_usage_count",
                "weekly_usage_count",
                "used_count",
                "usage_count",
                "consumed_count",
            ],
            &[
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
                "current_interval_usage_count",
                "interval_usage_count",
                "current_interval_used_count",
                "used_count",
                "usage_count",
                "consumed_count",
            ],
            &[
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
                "current_weekly_usage_count",
                "weekly_usage_count",
                "seven_day_usage_count",
                "current_interval_usage_count",
                "interval_usage_count",
                "used_count",
                "usage_count",
                "consumed_count",
            ],
            &[
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
        assert!((five_hour.utilization - 0.3).abs() < 0.0001);
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
        assert!((five_hour.utilization - (1456.0 / 1500.0)).abs() < 0.0001);
        assert!((weekly.utilization - (14954.0 / 15000.0)).abs() < 0.0001);
        assert!(five_hour.resets_at.is_some());
        assert!(weekly.resets_at.is_some());
    }
}
