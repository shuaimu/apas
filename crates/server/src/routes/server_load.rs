use axum::{extract::State, http::HeaderMap, Json};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::time::Duration;
use tokio::fs;

use super::authz::require_system_admin;
use crate::{error::AppError, session::load::Activity, state::AppState};

#[derive(Serialize)]
pub(super) struct Snapshot {
    sampled_at: DateTime<Utc>,
    activity: Activity,
    host_cpu_percent: Option<f64>,
    host_memory: Option<Memory>,
}

#[derive(Serialize)]
struct Memory {
    used_bytes: u64,
    total_bytes: u64,
}

fn memory(input: &str) -> Option<Memory> {
    let bytes = |key: &str| {
        let mut fields = input
            .lines()
            .find_map(|line| line.strip_prefix(key))?
            .split_whitespace();
        let kib = fields.next()?.parse::<u64>().ok()?;
        (fields.next()? == "kB").then_some(kib)?.checked_mul(1024)
    };
    let total_bytes = bytes("MemTotal:")?;
    Some(Memory {
        used_bytes: total_bytes.checked_sub(bytes("MemAvailable:")?)?,
        total_bytes,
    })
}

struct CpuTicks {
    idle: u64,
    total: u64,
}

impl CpuTicks {
    fn parse(input: &str) -> Option<Self> {
        let mut fields = input.lines().next()?.split_whitespace();
        if fields.next()? != "cpu" {
            return None;
        }
        // The guest fields already contribute to user and nice time.
        let ticks: Vec<u64> = fields
            .take(8)
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        Some(Self {
            idle: ticks
                .get(3)?
                .checked_add(ticks.get(4).copied().unwrap_or(0))?,
            total: ticks
                .iter()
                .try_fold(0u64, |sum, tick| sum.checked_add(*tick))?,
        })
    }

    fn usage_since(&self, previous: Self) -> Option<f64> {
        let elapsed = self.total.checked_sub(previous.total)?;
        let idle = self.idle.checked_sub(previous.idle)?;
        let busy = elapsed.checked_sub(idle)?;
        (elapsed > 0).then(|| 100.0 * busy as f64 / elapsed as f64)
    }

    async fn read() -> Option<Self> {
        Self::parse(&fs::read_to_string("/proc/stat").await.ok()?)
    }
}

pub(super) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Snapshot>, AppError> {
    require_system_admin(&headers, &state).await?;
    let host_cpu_percent = if let Some(previous) = CpuTicks::read().await {
        tokio::time::sleep(Duration::from_millis(250)).await;
        CpuTicks::read()
            .await
            .and_then(|ticks| ticks.usage_since(previous))
    } else {
        None
    };
    let host_memory = fs::read_to_string("/proc/meminfo")
        .await
        .ok()
        .and_then(|data| memory(&data));
    Ok(Json(Snapshot {
        sampled_at: Utc::now(),
        activity: state.sessions.activity(),
        host_cpu_percent,
        host_memory,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_uses_interval_and_excludes_guest_and_io_wait() {
        let before = || CpuTicks::parse("cpu 10 0 10 60 20 0 0 0 10 0").unwrap();
        let after = CpuTicks::parse("cpu 20 0 25 110 45 0 0 0 20 0").unwrap();
        assert_eq!(after.usage_since(before()), Some(25.0));
        assert_eq!(before().usage_since(before()), None);
    }

    #[test]
    fn used_memory_excludes_available_pages() {
        let data = memory("MemTotal: 1024 kB\nMemFree: 100 kB\nMemAvailable: 256 kB").unwrap();
        assert_eq!(data.used_bytes, 768 * 1024);
        assert!(memory("MemFree: 100 kB").is_none());
    }
}
