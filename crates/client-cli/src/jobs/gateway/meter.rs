//! Fsynced aggregate accounting. A pending dispatch on restart is uncertainty, never free usage.
use super::{config::ModelProfileConfig, policy::EVENT_BYTES};
use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shared::{JobLimits, JobModelApi, JobUsage};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};
use uuid::Uuid;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cached: u64,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Pricing {
    key: String,
    model: String,
    api: JobModelApi,
    context_window: u32,
    reasoning: bool,
    endpoint: String,
    input: u64,
    output: u64,
    cached: Option<u64>,
}

impl From<&ModelProfileConfig> for Pricing {
    fn from(p: &ModelProfileConfig) -> Self {
        Self {
            key: p.key.clone(),
            model: p.model.clone(),
            api: p.api,
            endpoint: p.upstream_base_url.clone(),
            context_window: p.context_window,
            reasoning: p.reasoning,
            input: p.input_micro_usd_per_million,
            output: p.output_micro_usd_per_million,
            cached: p.cached_input_micro_usd_per_million,
        }
    }
}

impl Pricing {
    fn cost(&self, tokens: &Tokens) -> Result<u64> {
        ensure!(
            tokens.cached <= tokens.input,
            "provider cache usage exceeds input usage"
        );
        let cached_price = if tokens.cached == 0 {
            0
        } else {
            self.cached.context("cached input price is unknown")?
        };
        let numerator = (u128::from(tokens.input - tokens.cached) * u128::from(self.input))
            .checked_add(u128::from(tokens.output) * u128::from(self.output))
            .and_then(|cost| cost.checked_add(u128::from(tokens.cached) * u128::from(cached_price)))
            .context("reported spend arithmetic overflow")?;
        u64::try_from(numerator.div_ceil(1_000_000)).context("reported spend overflow")
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Journal {
    version: u32,
    pub run_id: Uuid,
    pricing: Option<Pricing>,
    pub dependency_hosts: Option<BTreeSet<String>>,
    pub limits: JobLimits,
    pub deadline: DateTime<Utc>,
    pub lease_expires_at: DateTime<Utc>,
    pub usage: JobUsage,
    pub response_bytes: u64,
    pub pending: BTreeMap<Uuid, Tokens>,
    pub unknown: bool,
    pub stopped: bool,
    pub model_enabled: bool,
    pub stop_reason: Option<String>,
}

impl Journal {
    pub fn load_or_create(
        path: &Path,
        run_id: Uuid,
        pricing: Option<Pricing>,
        limits: JobLimits,
        prior_usage: JobUsage,
        deadline: DateTime<Utc>,
        lease_expires_at: DateTime<Utc>,
    ) -> Result<Self> {
        if path.try_exists()? {
            let data = read_private(path, 1024 * 1024)?;
            let mut journal: Self =
                serde_json::from_slice(&data).context("invalid gateway journal")?;
            ensure!(
                journal.version == 1 && journal.run_id == run_id && journal.pricing == pricing,
                "gateway journal identity or pricing changed"
            );
            journal.limits = journal.limits.narrowed_by(&limits);
            journal.deadline = journal.deadline.min(deadline);
            journal.lease_expires_at = journal.lease_expires_at.min(lease_expires_at);
            if !journal.pending.is_empty() {
                journal.mark_unknown("interrupted_model_dispatch");
            }
            journal.persist(path)?;
            return Ok(journal);
        }
        ensure!(
            prior_usage.spend_micro_usd.is_some(),
            "prior reported spend is unknown"
        );
        let model_enabled = pricing.is_some();
        let journal = Self {
            version: 1,
            run_id,
            pricing,
            limits,
            deadline,
            lease_expires_at,
            usage: prior_usage,
            response_bytes: 0,
            pending: BTreeMap::new(),
            unknown: false,
            stopped: false,
            model_enabled,
            stop_reason: None,
            dependency_hosts: None,
        };
        journal.persist(path)?;
        Ok(journal)
    }

    pub fn persist(&self, path: &Path) -> Result<()> {
        atomic_private(path, &serde_json::to_vec(self)?)
    }

    pub fn gate(&self, model: bool) -> Result<()> {
        ensure!(!self.stopped && !self.unknown, "gateway is fenced");
        ensure!(
            Utc::now() < self.deadline && Utc::now() < self.lease_expires_at,
            "execution authority expired"
        );
        ensure!(
            self.usage
                .spend_micro_usd
                .is_some_and(|v| v < self.limits.spend_micro_usd),
            "reported spend threshold reached"
        );
        ensure!(
            self.response_bytes < self.limits.output_bytes,
            "gateway output allowance exhausted"
        );
        if model {
            ensure!(
                self.model_enabled && self.pricing.is_some(),
                "model access is disabled"
            );
            ensure!(
                self.pending.len() < self.limits.model_concurrency as usize,
                "model concurrency exhausted"
            );
        }
        Ok(())
    }

    pub fn begin(&mut self, requested_output_tokens: u32) -> Result<Uuid> {
        self.gate(true)?;
        ensure!(
            requested_output_tokens > 0
                && requested_output_tokens <= self.limits.model_output_tokens,
            "model output limit exceeds current allowance"
        );
        let id = Uuid::new_v4();
        self.pending.insert(id, Tokens::default());
        Ok(id)
    }

    pub fn observe(&mut self, id: Uuid, tokens: Tokens, terminal: bool) -> Result<()> {
        let previous = self.pending.get(&id).context("unknown model request")?;
        ensure!(
            tokens.input >= previous.input
                && tokens.output >= previous.output
                && tokens.cached >= previous.cached,
            "provider usage moved backwards"
        );
        let pricing = self.pricing.as_ref().context("missing model pricing")?;
        let old_cost = pricing.cost(previous)?;
        let new_cost = pricing.cost(&tokens)?;
        let input = self
            .usage
            .input_tokens
            .checked_add(tokens.input - previous.input)
            .context("input usage overflow")?;
        let output = self
            .usage
            .output_tokens
            .checked_add(tokens.output - previous.output)
            .context("output usage overflow")?;
        let spend = self
            .usage
            .spend_micro_usd
            .context("reported spend is unknown")?
            .checked_add(
                new_cost
                    .checked_sub(old_cost)
                    .context("provider price moved backwards")?,
            )
            .context("spend overflow")?;
        self.usage.input_tokens = input;
        self.usage.output_tokens = output;
        self.usage.spend_micro_usd = Some(spend);
        self.usage.estimated = true; // Provider token reports times pinned pricing, not a final invoice.
        if terminal {
            self.pending.remove(&id);
        } else {
            self.pending.insert(id, tokens);
        }
        Ok(())
    }

    pub fn mark_unknown(&mut self, reason: &str) {
        self.unknown = true;
        self.stop_reason = Some(reason.to_owned());
        // Preserve the last known aggregate in the journal; public usage marks the incomplete total unknown.
    }

    pub fn public_usage(&self) -> JobUsage {
        let mut usage = self.usage.clone();
        if self.unknown {
            usage.spend_micro_usd = None;
        }
        usage
    }
}

pub(super) fn read_private(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .context("cannot open private gateway file")?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.nlink() == 1
            && metadata.len() <= limit,
        "gateway file must be owner-only, regular and bounded"
    );
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "gateway file exceeds limit");
    Ok(bytes)
}

pub(super) fn atomic_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("gateway file has no parent")?;
    let temp = parent.join(format!(".gateway-{}", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub(super) fn private_directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => fs::set_permissions(path, fs::Permissions::from_mode(0o700))?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "gateway directory must be owner-only"
    );
    Ok(())
}

pub(super) struct StreamMeter {
    api: JobModelApi,
    buffer: Vec<u8>,
    pub tokens: Option<Tokens>,
    pub terminal: bool,
    saw_output_usage: bool,
}

impl StreamMeter {
    pub fn new(api: JobModelApi) -> Self {
        Self {
            api,
            buffer: Vec::new(),
            tokens: None,
            terminal: false,
            saw_output_usage: false,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<(Tokens, bool)>> {
        let mut observations = Vec::new();
        // Work bytewise only within bounded SSE frames; a network chunk may contain many frames.
        for byte in bytes {
            self.buffer.push(*byte);
            ensure!(
                self.buffer.len() <= EVENT_BYTES,
                "provider SSE frame exceeds limit"
            );
            if self.buffer.ends_with(b"\n\n") || self.buffer.ends_with(b"\r\n\r\n") {
                let frame = std::mem::take(&mut self.buffer);
                let text = std::str::from_utf8(&frame).context("provider SSE is not UTF-8")?;
                let data = text
                    .lines()
                    .filter_map(|line| {
                        line.strip_prefix("data:")
                            .map(|v| v.strip_prefix(' ').unwrap_or(v))
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if data.is_empty() || data == "[DONE]" {
                    continue;
                }
                let event: Value =
                    serde_json::from_str(&data).context("invalid provider SSE data")?;
                if let Some(observation) = self.event(&event)? {
                    observations.push(observation);
                }
            }
        }
        Ok(observations)
    }

    fn event(&mut self, value: &Value) -> Result<Option<(Tokens, bool)>> {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .context("missing provider event type")?;
        ensure!(!self.terminal, "provider data followed terminal usage");
        match self.api {
            JobModelApi::OpenaiResponses => match kind {
                "response.completed" | "response.incomplete" | "response.failed" => {
                    let tokens = openai_usage(value.get("response").context("missing response")?)?;
                    self.tokens = Some(tokens.clone());
                    self.terminal = true;
                    Ok(Some((tokens, true)))
                }
                "error" => bail!("provider stream failed without complete metering"),
                _ if kind.starts_with("response.") => Ok(None),
                _ => bail!("unsupported Responses stream event"),
            },
            JobModelApi::AnthropicMessages => match kind {
                "message_start" => {
                    ensure!(self.tokens.is_none(), "duplicate message start");
                    let tokens = anthropic_usage(value.get("message").context("missing message")?)?;
                    self.tokens = Some(tokens.clone());
                    Ok(Some((tokens, false)))
                }
                "message_delta" => {
                    let usage = value.get("usage").context("missing message delta usage")?;
                    let tokens = self.tokens.as_mut().context("message delta before start")?;
                    tokens.output = number(usage, "output_tokens")?;
                    self.saw_output_usage = true;
                    Ok(Some((tokens.clone(), false)))
                }
                "message_stop" => {
                    ensure!(self.saw_output_usage, "message stopped without final usage");
                    let tokens = self.tokens.clone().context("missing message usage")?;
                    self.terminal = true;
                    Ok(Some((tokens, true)))
                }
                "ping" | "content_block_start" | "content_block_delta" | "content_block_stop" => {
                    Ok(None)
                }
                "error" => bail!("provider stream failed without complete metering"),
                _ => bail!("unsupported Messages stream event"),
            },
        }
    }

    pub fn finish(&self) -> Result<()> {
        ensure!(
            self.terminal && self.buffer.iter().all(u8::is_ascii_whitespace),
            "provider stream ended without complete metering"
        );
        Ok(())
    }
}

fn number(value: &Value, key: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .with_context(|| format!("missing provider {key}"))
}

pub(super) fn response_usage(api: JobModelApi, body: &[u8]) -> Result<Tokens> {
    let value: Value = serde_json::from_slice(body).context("provider response is not JSON")?;
    match api {
        JobModelApi::OpenaiResponses => openai_usage(&value),
        JobModelApi::AnthropicMessages => anthropic_usage(&value),
    }
}

fn openai_usage(value: &Value) -> Result<Tokens> {
    ensure!(
        value
            .get("service_tier")
            .is_none_or(|v| v.as_str() == Some("default")),
        "provider reported an unpriced service tier"
    );
    let usage = value.get("usage").context("missing provider usage")?;
    let cached = usage
        .get("input_tokens_details")
        .and_then(|v| v.get("cached_tokens"))
        .map(|v| v.as_u64().context("invalid cached token usage"))
        .transpose()?
        .unwrap_or(0);
    let tokens = Tokens {
        input: number(usage, "input_tokens")?,
        output: number(usage, "output_tokens")?,
        cached,
    };
    ensure!(
        tokens.cached <= tokens.input,
        "invalid provider cached usage"
    );
    Ok(tokens)
}

fn anthropic_usage(value: &Value) -> Result<Tokens> {
    let usage = value.get("usage").context("missing provider usage")?;
    ensure!(
        usage
            .get("service_tier")
            .is_none_or(|v| v.as_str() == Some("standard")),
        "provider reported an unpriced service tier"
    );
    ensure!(
        usage.get("server_tool_use").is_none_or(|v| v.is_null()
            || v.as_object()
                .is_some_and(|counts| counts.values().all(|count| count.as_u64() == Some(0)))),
        "provider reported unpriced server tool usage"
    );
    ensure!(
        usage
            .get("cache_creation_input_tokens")
            .is_none_or(|v| v.as_u64() == Some(0)),
        "unpriced cache creation usage"
    );
    let cached = usage
        .get("cache_read_input_tokens")
        .map(|v| v.as_u64().context("invalid cached usage"))
        .transpose()?
        .unwrap_or(0);
    Ok(Tokens {
        input: number(usage, "input_tokens")?
            .checked_add(cached)
            .context("input usage overflow")?,
        output: number(usage, "output_tokens")?,
        cached,
    })
}
