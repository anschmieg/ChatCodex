//! Deterministic core: all business logic for the deterministic
//! coding-harness control plane.
//!
//! This crate **must not** depend on any model provider SDK, and
//! **must not** contain autonomous agent logic.

pub mod approval;
pub mod file_read;
pub mod git_diff;
pub mod git_status;
pub mod code_search;
pub mod patch_apply;
pub mod run_prepare;
pub mod run_refresh;
pub mod run_replan;
pub mod tests_run;
pub mod workspace_summary;
