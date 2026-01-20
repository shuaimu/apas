//! CLI authentication module - device code flow for login

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct DeviceCodeResponse {
    pub code: String,
    pub url: String,
    pub expires_in: u64,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status")]
pub enum DevicePollResponse {
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "success")]
    Success { token: String, user_id: String },
    #[serde(rename = "expired")]
    Expired,
}

#[derive(Debug, Serialize)]
struct DevicePollRequest {
    code: String,
}

/// Perform device code login flow
/// Returns the JWT token on success
pub async fn login(server_url: &str) -> Result<String> {
    let client = reqwest::Client::new();

    // Convert ws:// to http:// for REST endpoints
    let http_url = server_url
        .replace("ws://", "http://")
        .replace("wss://", "https://");

    // 1. Request device code
    let resp = client
        .post(format!("{}/auth/device-code", http_url))
        .send()
        .await?;

    if !resp.status().is_success() {
        bail!("Failed to get device code: {}", resp.status());
    }

    let device_code: DeviceCodeResponse = resp.json().await?;

    // 2. Show URL to user
    println!();
    println!("\x1b[1;36m🔐 To login, open this URL in your browser:\x1b[0m");
    println!();
    println!("   \x1b[4m{}\x1b[0m", device_code.url);
    println!();
    println!("\x1b[90mWaiting for login... (expires in {} seconds)\x1b[0m", device_code.expires_in);

    // 3. Poll for completion
    let poll_interval = Duration::from_secs(2);
    let max_attempts = device_code.expires_in / 2;
    let poll_url = format!("{}/auth/device-poll", http_url);

    for attempt in 0..max_attempts {
        tokio::time::sleep(poll_interval).await;

        let poll_result = client
            .post(&poll_url)
            .json(&DevicePollRequest {
                code: device_code.code.clone(),
            })
            .send()
            .await;

        let poll_resp = match poll_result {
            Ok(resp) => resp,
            Err(e) => {
                // Log network errors but continue polling
                if attempt % 5 == 0 {
                    eprintln!("\x1b[33mNetwork error (retrying...): {}\x1b[0m", e);
                }
                continue;
            }
        };

        if !poll_resp.status().is_success() {
            if attempt % 5 == 0 {
                eprintln!("\x1b[33mPoll returned {}, retrying...\x1b[0m", poll_resp.status());
            }
            continue;
        }

        let poll_result: DevicePollResponse = match poll_resp.json().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("\x1b[33mFailed to parse poll response: {}\x1b[0m", e);
                continue;
            }
        };

        match poll_result {
            DevicePollResponse::Success { token, user_id } => {
                println!();
                println!("\x1b[1;32m✅ Login successful!\x1b[0m");
                println!("\x1b[90mUser ID: {}\x1b[0m", user_id);
                return Ok(token);
            }
            DevicePollResponse::Pending => {
                // Still waiting, continue polling
                continue;
            }
            DevicePollResponse::Expired => {
                bail!("Login expired. Please try again.");
            }
        }
    }

    bail!("Login timed out. Please try again.");
}

/// Logout by clearing the stored token
pub fn logout(config: &mut crate::config::Config) -> Result<()> {
    config.remote.token = None;
    config.save()?;
    println!("\x1b[32m✅ Logged out successfully\x1b[0m");
    Ok(())
}

/// Show current login status
pub async fn whoami(config: &crate::config::Config, server_url: &str) -> Result<()> {
    match &config.remote.token {
        Some(token) => {
            // Try to validate the token by making a simple request
            // For now, just show that we have a token
            println!("\x1b[32m✓ Logged in\x1b[0m");
            println!("Server: {}", server_url);
            // Token is present, but we don't decode it client-side
            // The server will validate it on connection
            let _ = token; // Silence unused warning
        }
        None => {
            println!("\x1b[33m✗ Not logged in\x1b[0m");
            println!("Run '\x1b[1mapas login\x1b[0m' to authenticate");
        }
    }
    Ok(())
}
