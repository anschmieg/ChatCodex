//! Deterministic daemon: HTTP JSON-RPC transport, SQLite persistence,
//! and handler wiring for the deterministic coding-harness control plane.
//!
//! This crate **must not** depend on any model provider SDK.
//! It **must not** contain autonomous agent logic.

pub mod handlers;
pub mod persistence;
pub mod router;
