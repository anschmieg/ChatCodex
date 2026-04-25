//! Deterministic protocol: shared DTOs and method names for the
//! deterministic coding-harness control plane.
//!
//! This crate contains **only** types and constants.  It has no
//! business logic and no transport awareness.

pub mod methods;
pub mod types;

pub use methods::Method;
pub use types::*;
