//! ChatCodex workspace root
//!
//! This library exists solely to make the workspace root a real (non-virtual) manifest,
//! which is required to declare workspace-level [[bin]] and [[test]] tables (used for
//! `cargo run --bin` and `cargo test --workspace` from the workspace root).
//!
//! All real crates are under `crates/`:
//!   - `crates/deterministic-protocol` — wire types, JSON-RPC method definitions
//!   - `crates/deterministic-core` — business logic, run state, approval policy
//!   - `crates/deterministic-daemon` — Axum HTTP server, persistence, handlers

pub mod _private {
    //Marker type to suppress "empty library" warnings
    pub struct NoPublicApi;
}