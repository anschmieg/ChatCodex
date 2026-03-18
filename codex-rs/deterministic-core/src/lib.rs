//! Deterministic core: all business logic for the deterministic
//! coding-harness control plane.
//!
//! This crate **must not** depend on any model provider SDK, and
//! **must not** contain autonomous agent logic.

pub mod approval;
pub mod approval_policy;
pub mod file_read;
pub mod git_diff;
pub mod git_status;
pub mod code_search;
pub mod patch_apply;
pub mod run_annotate;
pub mod run_archive;
pub mod run_finalize;
pub mod run_pin;
pub mod run_prepare;
pub mod run_refresh;
pub mod run_reopen;
pub mod run_replan;
pub mod run_supersede;
pub mod run_unarchive;
pub mod run_unpin;
pub mod run_snooze;
pub mod run_unsnooze;
pub mod run_set_priority;
pub mod run_assign_owner;
pub mod run_set_due_date;
pub mod run_set_dependencies;
pub mod run_set_effort;
pub mod run_readiness;
pub mod run_staleness;
pub mod run_triage;
pub mod tests_run;
pub mod workspace_summary;
