//! Closed request vocabulary and DNS-pinned TLS egress; no ambient proxy support.
use super::config::ModelProfileConfig;
use anyhow::{bail, ensure, Context, Result};
use hyper::{HeaderMap, Method, Uri};
use reqwest::{Client, Url};
use serde_json::Value;
use shared::JobModelApi;
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

pub(super) const REQUEST_BYTES: usize = 2 * 1024 * 1024;
pub(super) const EVENT_BYTES: usize = 512 * 1024;
pub(super) const RESPONSE_BYTES: u64 = 32 * 1024 * 1024;
pub(super) const TOKEN_HEADER: &str = "x-apas-run-token";

pub(super) fn dependency_hosts(hosts: &[String]) -> Result<BTreeSet<String>> {
    ensure!(hosts.len() <= 128, "too many dependency hosts");
    hosts
        .iter()
        .map(|host| {
            ensure!(
                valid_hostname(host),
                "dependency host must be an exact public DNS name"
            );
            Ok(host.clone())
        })
        .collect()
}

fn valid_hostname(host: &str) -> bool {
    host.len() <= 253
        && host.contains('.')
        && host == host.to_ascii_lowercase()
        && !host.ends_with('.')
        && host.parse::<IpAddr>().is_err()
        && ![
            ".localhost",
            ".local",
            ".internal",
            ".test",
            ".invalid",
            ".onion",
        ]
        .iter()
        .any(|s| host.ends_with(s))
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        })
}

pub(super) fn model_url(profile: &ModelProfileConfig) -> Result<Url> {
    ensure!(
        !profile.key.is_empty() && !profile.model.is_empty() && profile.model.len() <= 256,
        "invalid model profile"
    );
    ensure!(
        profile.context_window > 0,
        "model context window must be explicitly provisioned"
    );
    ensure!(
        profile.input_micro_usd_per_million > 0 && profile.output_micro_usd_per_million > 0,
        "model pricing must be explicitly provisioned"
    );
    let mut url = Url::parse(&profile.upstream_base_url).context("invalid model endpoint")?;
    validate_url(&url)?;
    ensure!(
        url.query().is_none(),
        "model endpoint cannot contain a query"
    );
    let base = url.path().trim_end_matches('/');
    ensure!(
        base.is_empty() || base == "/v1",
        "unsupported model endpoint path"
    );
    url.set_path(match profile.api {
        JobModelApi::OpenaiResponses => "/v1/responses",
        JobModelApi::AnthropicMessages => "/v1/messages",
    });
    Ok(url)
}

pub(super) fn validate_url(url: &Url) -> Result<()> {
    ensure!(
        url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "egress requires an unauthenticated TLS port 443 URL"
    );
    ensure!(
        url.host_str().is_some_and(valid_hostname),
        "egress requires a public DNS name"
    );
    Ok(())
}

pub(super) fn dependency_url(uri: &Uri, hosts: &BTreeSet<String>) -> Result<Url> {
    ensure!(
        uri.scheme().is_none() && uri.authority().is_none(),
        "absolute request targets are forbidden"
    );
    let rest = uri
        .path()
        .strip_prefix("/dependencies/")
        .context("unsupported gateway route")?;
    let (host, path) = rest
        .split_once('/')
        .context("dependency route requires a path")?;
    ensure!(
        hosts.contains(host),
        "dependency destination is not approved"
    );
    ensure!(
        !path.contains('\\') && !path.contains('#'),
        "invalid dependency path"
    );
    let mut url = Url::parse(&format!("https://{host}/{path}"))?;
    url.set_query(uri.query());
    validate_url(&url)?;
    ensure!(
        url.host_str() == Some(host),
        "dependency destination changed"
    );
    Ok(url)
}

pub(super) fn public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => {
            // Only global unicast, excluding transition/translation and documentation ranges.
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] < 0x1000)
                && ip != Ipv6Addr::UNSPECIFIED
        }
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && (b == 168 || b == 0 || (b == 88 && c == 99)))
        || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
        || (a == 203 && b == 0 && c == 113))
}

pub(super) async fn pinned_client(url: &Url) -> Result<Client> {
    validate_url(url)?;
    let host = url.host_str().context("missing egress host")?;
    let resolved: Vec<SocketAddr> = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::net::lookup_host((host, 443)),
    )
    .await
    .context("egress DNS timed out")??
    .collect();
    ensure!(
        !resolved.is_empty() && resolved.len() <= 64 && resolved.iter().all(|a| public_ip(a.ip())),
        "egress DNS returned a forbidden address"
    );
    // Resolve and validate once, then pin those exact addresses while retaining hostname TLS verification.
    // Fresh client per request prevents HTTP/2 connection coalescing and stale DNS/pool surprises.
    Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &resolved)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .pool_max_idle_per_host(0)
        .build()
        .context("cannot construct TLS egress client")
}

pub(super) fn check_headers(headers: &HeaderMap, token: &str, model: bool) -> Result<()> {
    ensure!(headers.len() <= 32, "too many request headers");
    let mut bytes = 0usize;
    for (name, value) in headers {
        bytes += name.as_str().len() + value.as_bytes().len();
        ensure!(bytes <= 16 * 1024, "request headers exceed limit");
        match name.as_str() {
            "host" | "content-type" | "content-length" | "accept" | "accept-encoding"
            | "user-agent" | "connection" => {}
            TOKEN_HEADER => {}
            "authorization" if model => {
                let value = value.to_str().context("invalid scoped authorization")?;
                ensure!(
                    value
                        .strip_prefix("Bearer ")
                        .is_some_and(|v| same_token(v, token)),
                    "wrong scoped authorization"
                );
            }
            "x-api-key" if model => ensure!(
                same_token(value.to_str().unwrap_or(""), token),
                "wrong scoped authorization"
            ),
            "anthropic-version" if model => {
                ensure!(value == "2023-06-01", "unsupported Anthropic version")
            }
            "anthropic-beta" if model => {
                approved_betas(value.to_str().context("invalid beta header")?)?;
            }
            "anthropic-dangerous-direct-browser-access" if model => {
                ensure!(value == "true", "invalid SDK transport header")
            }
            "x-app" if model => ensure!(value == "cli", "invalid SDK application header"),
            // The SDK's local diagnostics cannot widen upstream behavior and are never forwarded.
            name if model && name.starts_with("x-stainless-") => {}
            _ => bail!("request header is not permitted"),
        }
    }
    ensure!(
        headers.get_all(TOKEN_HEADER).iter().count() == 1
            && headers
                .get(TOKEN_HEADER)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| same_token(v, token)),
        "gateway authentication failed"
    );
    ensure!(
        !headers.contains_key("connection")
            || headers["connection"]
                .as_bytes()
                .eq_ignore_ascii_case(b"close")
            || headers["connection"]
                .as_bytes()
                .eq_ignore_ascii_case(b"keep-alive"),
        "connection upgrade is forbidden"
    );
    Ok(())
}
fn approved_betas(value: &str) -> Result<()> {
    ensure!(
        value.split(',').all(|beta| matches!(
            beta.trim(),
            "interleaved-thinking-2025-05-14"
                | "fine-grained-tool-streaming-2025-05-14"
                | "effort-2025-11-24"
                | "structured-outputs-2025-12-15"
        )),
        "unqualified provider beta"
    );
    Ok(())
}

pub(super) fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

pub(super) fn model_request(
    method: &Method,
    uri: &Uri,
    body: &[u8],
    profile: &ModelProfileConfig,
    output_limit: u32,
) -> Result<(Vec<u8>, bool, u32)> {
    ensure!(
        *method == Method::POST
            && uri.scheme().is_none()
            && uri.authority().is_none()
            && uri.query().is_none(),
        "unsupported model request target"
    );
    let expected = match profile.api {
        JobModelApi::OpenaiResponses => "/v1/responses",
        JobModelApi::AnthropicMessages => "/v1/messages",
    };
    ensure!(
        uri.path() == expected && body.len() <= REQUEST_BYTES,
        "unsupported model route or body"
    );
    let mut value: Value = serde_json::from_slice(body).context("model request is not JSON")?;
    let object = value
        .as_object_mut()
        .context("model request must be an object")?;
    ensure!(
        object.get("model").and_then(Value::as_str) == Some(profile.model.as_str()),
        "model is not approved"
    );
    let allowed: &[&str] = match profile.api {
        JobModelApi::OpenaiResponses => &[
            "model",
            "input",
            "instructions",
            "max_output_tokens",
            "stream",
            "tools",
            "tool_choice",
            "parallel_tool_calls",
            "reasoning",
            "text",
            "temperature",
            "top_p",
            "store",
            "include",
            "truncation",
            "service_tier",
            "prompt_cache_key",
            "stream_options",
        ],
        JobModelApi::AnthropicMessages => &[
            "model",
            "messages",
            "system",
            "max_tokens",
            "stream",
            "tools",
            "tool_choice",
            "thinking",
            "temperature",
            "top_p",
            "top_k",
            "stop_sequences",
            "metadata",
            "output_config",
        ],
    };
    ensure!(
        object.keys().all(|k| allowed.contains(&k.as_str())),
        "unsupported model request field"
    );
    match profile.api {
        JobModelApi::OpenaiResponses => {
            ensure!(
                object
                    .get("input")
                    .is_some_and(|v| v.is_string() || v.as_array().is_some_and(|v| !v.is_empty())),
                "Responses input is required"
            );
            ensure!(
                object
                    .get("prompt_cache_key")
                    .is_none_or(|v| v.as_str().is_some_and(|s| s.len() <= 256)),
                "invalid prompt cache key"
            );
            ensure!(
                object
                    .get("stream_options")
                    .is_none_or(|v| v.as_object().is_some_and(|m| m.len() == 1
                        && m.get("include_obfuscation") == Some(&Value::Bool(false)))),
                "unsupported stream option"
            );
        }
        JobModelApi::AnthropicMessages => {
            ensure!(
                object
                    .get("messages")
                    .and_then(Value::as_array)
                    .is_some_and(|v| !v.is_empty()),
                "Messages input is required"
            );
            ensure!(
                object
                    .get("metadata")
                    .is_none_or(|v| v.as_object().is_some_and(|m| m.len() == 1
                        && m.get("user_id")
                            .and_then(Value::as_str)
                            .is_some_and(|s| s.len() <= 512))),
                "unsupported Messages metadata"
            );
            ensure!(
                object
                    .get("output_config")
                    .is_none_or(|v| v.as_object().is_some_and(|m| m.len() == 1
                        && m.get("effort")
                            .and_then(Value::as_str)
                            .is_some_and(|s| matches!(s, "low" | "medium" | "high" | "max")))),
                "unsupported Messages output configuration"
            );
        }
    }
    let cap_key = match profile.api {
        JobModelApi::OpenaiResponses => "max_output_tokens",
        JobModelApi::AnthropicMessages => "max_tokens",
    };
    let output = object
        .get(cap_key)
        .and_then(Value::as_u64)
        .context("explicit model output limit required")?;
    ensure!(
        output > 0 && output <= u64::from(output_limit),
        "model output limit exceeds allowance"
    );
    let streaming = match object.get("stream") {
        None => false,
        Some(Value::Bool(v)) => *v,
        _ => bail!("stream must be boolean"),
    };
    if let Some(tools) = object.get("tools") {
        let tools = tools.as_array().context("tools must be an array")?;
        ensure!(tools.len() <= 128, "too many tools");
        for tool in tools {
            let tool = tool.as_object().context("invalid tool")?;
            match profile.api {
                JobModelApi::OpenaiResponses => ensure!(
                    tool.get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| matches!(kind, "function" | "custom")),
                    "server-side tools are forbidden"
                ),
                JobModelApi::AnthropicMessages => ensure!(
                    tool.get("type").is_none()
                        || tool.get("type").and_then(Value::as_str) == Some("custom"),
                    "server-side tools are forbidden"
                ),
            }
        }
    }
    if profile.api == JobModelApi::OpenaiResponses {
        ensure!(
            object.get("store").is_none_or(|v| v == &Value::Bool(false)),
            "provider persistence is forbidden"
        );
        ensure!(
            object.get("service_tier").is_none_or(|v| v == "default"),
            "unpriced service tier"
        );
        object.insert("store".into(), Value::Bool(false));
        object.insert("service_tier".into(), Value::String("default".into()));
    }
    // Cache creation has a distinct price not represented by these qualified profiles.
    // Remote/provider-side tools and remote content can incur independently unmetered work.
    reject_unpriced_content(&value, 0)?;
    Ok((serde_json::to_vec(&value)?, streaming, output as u32))
}

fn reject_unpriced_content(value: &Value, depth: usize) -> Result<()> {
    ensure!(depth <= 64, "model request nesting exceeds limit");
    match value {
        Value::Object(map) => {
            ensure!(
                !map.contains_key("cache_control")
                    && !map.contains_key("file_id")
                    && !map.contains_key("file_url"),
                "unpriced external content is forbidden"
            );
            if let Some(url) = map.get("image_url").and_then(Value::as_str) {
                ensure!(
                    url.starts_with("data:image/"),
                    "remote image fetch is forbidden"
                );
            }
            if map.get("type").and_then(Value::as_str) == Some("url") {
                bail!("remote content is forbidden");
            }
            ensure!(
                !map.get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| matches!(kind, "input_audio" | "audio" | "video")),
                "unpriced media is forbidden"
            );
            for child in map.values() {
                reject_unpriced_content(child, depth + 1)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                reject_unpriced_content(child, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}
