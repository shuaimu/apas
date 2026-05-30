//! Codex → DeepSeek API bridge.
//!
//! Codex 1.0+ talks the OpenAI Responses API (`POST /v1/responses`) and
//! dropped support for `wire_api = "chat"`. DeepSeek's API only serves
//! the OpenAI Chat Completions shape (`POST /v1/chat/completions`). This
//! module is a tiny local proxy that lets Codex think it's talking to a
//! Responses endpoint while we translate to/from Chat Completions and
//! forward to DeepSeek.
//!
//! The translation crate is shaped after VibeAround's `va-ai-api-bridge`
//! but trimmed: only the two API shapes we actually need, no universal
//! IR, no streaming yet (streaming + axum HTTP wrapping land in the
//! follow-up slice).

pub mod translate;
