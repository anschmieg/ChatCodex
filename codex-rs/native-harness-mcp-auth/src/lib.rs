//! OAuth 2.1 / MCP 2025-11-25 authorization layer for
//! `codex-native-harness-mcp`.
//!
//! This crate sits in front of the existing deterministic harness. It
//! issues short-lived RS256 access tokens (with rotating refresh tokens) to
//! MCP clients, using Cloudflare Access as the upstream identity provider.
//!
//! The crate is strictly deterministic: it does not call any model, does
//! not start a Codex turn, and does not invoke `codex-reply` or
//! `turn/start`. The MCP server's tool dispatch is unchanged.

#![deny(clippy::print_stdout, clippy::print_stderr)]

pub mod authorize;
pub mod cf_access;
pub mod clients;
pub mod config;
pub mod discovery;
pub mod keyring;
pub mod middleware;
pub mod ratelimit;
pub mod state;
pub mod storage;
pub mod token;
pub mod well_known;

pub use config::AuthConfig;
pub use state::AuthState;
