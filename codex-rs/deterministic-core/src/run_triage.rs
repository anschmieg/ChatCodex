//! Deterministic run triage bucket derivation (Milestone 27).
//!
//! All rules are derived from existing run state fields.
//! No state mutation, no background processing, no model calls, no timers.

use deterministic_protocol::{RunStaleness, RunTriageBucket};

/// Compact derived triage summary for a run.
///
/// Computed deterministically from existing run state fields.
/// No mutations are performed; no autonomous transitions are triggered.
#[derive(Debug, Clone, PartialEq)]
pub struct RunTriageSummary {
    /// The triage bucket this run belongs to.
    pub triage_bucket: RunTriageBucket,
    /// Concise reason for triage classification.
    pub triage_reason: String,
}

/// Derive the triage bucket from existing run state.
///
/// # Triage precedence rules (in order)
///
/// 1. `done`: archived OR finalized
/// 2. `deferred`: snoozed
/// 3. `blocked`: has blockers (is_blocked = true)
/// 4. `critical`: urgent priority AND (overdue OR staleness = stale)
/// 5. `attention`: urgent priority OR needs_attention
/// 6. `ready`: not blocked, not urgent, not stale, not snoozed
/// 7. `deferred`: fallback (shouldn't happen given above rules)
///
/// # Parameters
///
/// - `status`: run status string (e.g., "active", "finalized:completed", "archived")
/// - `is_archived`: whether the run is archived
/// - `is_snoozed`: whether the run is snoozed
/// - `is_blocked`: whether the run has blockers
/// - `priority`: priority as string ("urgent", "high", "normal", "low")
/// - `due_date`: optional due date in YYYY-MM-DD format
/// - `staleness_bucket`: derived staleness bucket from run_staleness module
/// - `reference_date`: ISO date for comparison (e.g., "2024-01-18")
///
/// No timers, notifications, escalations, or state mutations are performed.
#[allow(clippy::collapsible_if, clippy::too_many_arguments)]
pub fn derive_triage(
    status: &str,
    is_archived: bool,
    is_snoozed: bool,
    is_blocked: bool,
    priority: &str,
    due_date: Option<&str>,
    staleness_bucket: Option<RunStaleness>,
    reference_date: &str,
) -> RunTriageSummary {
    // 1. done: archived OR finalized
    if is_archived || status.starts_with("finalized:") {
        return RunTriageSummary {
            triage_bucket: RunTriageBucket::Done,
            triage_reason: "archived or finalized".to_string(),
        };
    }

    // 2. deferred: snoozed
    if is_snoozed {
        return RunTriageSummary {
            triage_bucket: RunTriageBucket::Deferred,
            triage_reason: "snoozed".to_string(),
        };
    }

    // 3. blocked: has active blockers
    if is_blocked {
        return RunTriageSummary {
            triage_bucket: RunTriageBucket::Blocked,
            triage_reason: "has blocking dependencies".to_string(),
        };
    }

    // 4. critical: urgent + (overdue OR stale)
    if priority == "urgent" {
        // Check if overdue
        let is_overdue = if let Some(due) = due_date {
            is_past_due(due, reference_date)
        } else {
            false
        };

        // Check staleness
        let is_stale = staleness_bucket == Some(RunStaleness::Stale);

        if is_overdue || is_stale {
            let reason = if is_overdue && is_stale {
                "urgent, overdue, and stale"
            } else if is_overdue {
                "urgent and overdue"
            } else {
                "urgent and stale"
            };
            return RunTriageSummary {
                triage_bucket: RunTriageBucket::Critical,
                triage_reason: reason.to_string(),
            };
        }
    }

    // 5. attention: urgent OR needs_attention (status indicates attention needed)
    if priority == "urgent" || status.contains("attention") || status.contains("waiting") {
        return RunTriageSummary {
            triage_bucket: RunTriageBucket::Attention,
            triage_reason: if priority == "urgent" {
                "urgent priority".to_string()
            } else {
                "needs attention".to_string()
            },
        };
    }

    // 6. ready: not blocked, not urgent, not stale, not snoozed
    let is_stale = staleness_bucket == Some(RunStaleness::Stale);
    if !is_blocked && priority != "urgent" && !is_stale && !is_snoozed {
        return RunTriageSummary {
            triage_bucket: RunTriageBucket::Ready,
            triage_reason: "ready to proceed".to_string(),
        };
    }

    // 7. deferred: fallback
    RunTriageSummary {
        triage_bucket: RunTriageBucket::Deferred,
        triage_reason: "no specific priority".to_string(),
    }
}

/// Check if a due date is in the past relative to reference date.
fn is_past_due(due_date: &str, reference_date: &str) -> bool {
    let ref_date = match chrono::NaiveDate::parse_from_str(reference_date, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return false,
    };

    let due = match chrono::NaiveDate::parse_from_str(due_date, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return false,
    };

    due < ref_date
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_archived_is_done() {
        let result = derive_triage("active", true, false, false, "normal", None, None, "2024-01-15");
        assert_eq!(result.triage_bucket, RunTriageBucket::Done);
    }

    #[test]
    fn test_finalized_is_done() {
        let result = derive_triage(
            "finalized:completed",
            false,
            false,
            false,
            "normal",
            None,
            None,
            "2024-01-15",
        );
        assert_eq!(result.triage_bucket, RunTriageBucket::Done);
    }

    #[test]
    fn test_snoozed_is_deferred() {
        let result = derive_triage("active", false, true, false, "normal", None, None, "2024-01-15");
        assert_eq!(result.triage_bucket, RunTriageBucket::Deferred);
    }

    #[test]
    fn test_blocked_is_blocked() {
        let result = derive_triage(
            "active",
            false,
            false,
            true,
            "normal",
            None,
            None,
            "2024-01-15",
        );
        assert_eq!(result.triage_bucket, RunTriageBucket::Blocked);
    }

    #[test]
    fn test_urgent_overdue_is_critical() {
        let result = derive_triage(
            "active",
            false,
            false,
            false,
            "urgent",
            Some("2024-01-01"),
            None,
            "2024-01-15",
        );
        assert_eq!(result.triage_bucket, RunTriageBucket::Critical);
    }

    #[test]
    fn test_urgent_stale_is_critical() {
        let result = derive_triage(
            "active",
            false,
            false,
            false,
            "urgent",
            None,
            Some(RunStaleness::Stale),
            "2024-01-15",
        );
        assert_eq!(result.triage_bucket, RunTriageBucket::Critical);
    }

    #[test]
    fn test_urgent_not_overdue_is_attention() {
        let result = derive_triage(
            "active",
            false,
            false,
            false,
            "urgent",
            Some("2024-01-20"),
            None,
            "2024-01-15",
        );
        assert_eq!(result.triage_bucket, RunTriageBucket::Attention);
    }

    #[test]
    fn test_normal_ready() {
        let result = derive_triage(
            "active",
            false,
            false,
            false,
            "normal",
            None,
            Some(RunStaleness::Fresh),
            "2024-01-15",
        );
        assert_eq!(result.triage_bucket, RunTriageBucket::Ready);
    }

    #[test]
    fn test_stale_normal_is_deferred() {
        let result = derive_triage(
            "active",
            false,
            false,
            false,
            "normal",
            None,
            Some(RunStaleness::Stale),
            "2024-01-15",
        );
        assert_eq!(result.triage_bucket, RunTriageBucket::Deferred);
    }
}