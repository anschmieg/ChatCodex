//! Deterministic run readiness and attention derivation (Milestone 22).
//!
//! All rules are derived from existing run metadata only.
//! No state mutation, no background processing, no model calls, no timers.

use deterministic_protocol::RunPriority;

/// Compact derived readiness and attention summary for a run.
///
/// Computed deterministically from existing run state fields.
/// No mutations are performed; no autonomous transitions are triggered.
#[derive(Debug, Clone, PartialEq)]
pub struct RunReadinessSummary {
    /// Whether this run is ready to work on now.
    pub is_ready: bool,
    /// Concise reason why this run is not ready, if applicable.
    /// Absent when the run is ready.
    pub readiness_reason: Option<String>,
    /// Whether this run currently warrants operator attention.
    pub needs_attention: bool,
    /// Concise reason why this run warrants attention.
    /// Absent when no attention is needed.
    pub attention_reason: Option<String>,
}

/// Derive the readiness and attention summary from existing run metadata.
///
/// # Readiness rules (first match wins)
///
/// 1. `is_archived` → not ready (`"archived"`)
/// 2. `status` starts with `"finalized:"` → not ready (`"finalized"`)
/// 3. `is_snoozed` → not ready (`"snoozed"`)
/// 4. `blocked_by_count > 0` → not ready (`"blocked by N run(s)"`)
/// 5. otherwise → ready
///
/// # Attention rules (independent, additive)
///
/// Conservative — derived only from visible-run state:
/// - Urgent visible ready run → `"urgent"`
/// - Pinned run that is blocked → `"pinned and blocked"`
/// - Overdue visible run (requires `today` date for comparison) → `"overdue"`
///
/// No timers, notifications, escalations, or state mutations are performed.
pub fn derive_readiness(
    status: &str,
    is_archived: bool,
    is_snoozed: bool,
    blocked_by_count: usize,
    priority: RunPriority,
    due_date: Option<&str>,
    is_pinned: bool,
    today: Option<&str>,
) -> RunReadinessSummary {
    // ---- readiness derivation (first match) ----
    let (is_ready, readiness_reason) = if is_archived {
        (false, Some("archived".to_string()))
    } else if status.starts_with("finalized:") {
        (false, Some("finalized".to_string()))
    } else if is_snoozed {
        (false, Some("snoozed".to_string()))
    } else if blocked_by_count > 0 {
        let reason = if blocked_by_count == 1 {
            "blocked by 1 run".to_string()
        } else {
            format!("blocked by {} runs", blocked_by_count)
        };
        (false, Some(reason))
    } else {
        (true, None)
    };

    // ---- attention derivation (conservative, independent) ----
    // Only visible (not archived, not snoozed, not finalized) runs can need attention.
    let visible = !is_archived && !is_snoozed && !status.starts_with("finalized:");

    let mut attention_reasons: Vec<&'static str> = Vec::new();

    if visible {
        // Urgent ready run.
        if priority == RunPriority::Urgent && is_ready {
            attention_reasons.push("urgent");
        }
        // Pinned but blocked.
        if is_pinned && blocked_by_count > 0 {
            attention_reasons.push("pinned and blocked");
        }
    }
    // Overdue check requires a reference date; only applied to visible runs.
    if visible {
        if let (Some(due), Some(today_val)) = (due_date, today) {
            if due < today_val {
                attention_reasons.push("overdue");
            }
        }
    }

    let needs_attention = !attention_reasons.is_empty();
    let attention_reason = if needs_attention {
        Some(attention_reasons.join(", "))
    } else {
        None
    };

    RunReadinessSummary {
        is_ready,
        readiness_reason,
        needs_attention,
        attention_reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(status: &str) -> RunReadinessSummary {
        derive_readiness(status, false, false, 0, RunPriority::Normal, None, false, None)
    }

    // ---- readiness: ready cases ----

    #[test]
    fn prepared_run_is_ready() {
        let s = ready("prepared");
        assert!(s.is_ready);
        assert!(s.readiness_reason.is_none());
    }

    #[test]
    fn active_run_is_ready() {
        let s = ready("active");
        assert!(s.is_ready);
        assert!(s.readiness_reason.is_none());
    }

    #[test]
    fn awaiting_approval_run_is_ready() {
        let s = ready("awaiting_approval");
        assert!(s.is_ready);
        assert!(s.readiness_reason.is_none());
    }

    // ---- readiness: not-ready cases ----

    #[test]
    fn archived_run_is_not_ready() {
        let s = derive_readiness("prepared", true, false, 0, RunPriority::Normal, None, false, None);
        assert!(!s.is_ready);
        assert_eq!(s.readiness_reason.as_deref(), Some("archived"));
    }

    #[test]
    fn finalized_completed_is_not_ready() {
        let s = ready("finalized:completed");
        assert!(!s.is_ready);
        assert_eq!(s.readiness_reason.as_deref(), Some("finalized"));
    }

    #[test]
    fn finalized_failed_is_not_ready() {
        let s = ready("finalized:failed");
        assert!(!s.is_ready);
        assert_eq!(s.readiness_reason.as_deref(), Some("finalized"));
    }

    #[test]
    fn finalized_abandoned_is_not_ready() {
        let s = ready("finalized:abandoned");
        assert!(!s.is_ready);
        assert_eq!(s.readiness_reason.as_deref(), Some("finalized"));
    }

    #[test]
    fn snoozed_run_is_not_ready() {
        let s = derive_readiness("active", false, true, 0, RunPriority::Normal, None, false, None);
        assert!(!s.is_ready);
        assert_eq!(s.readiness_reason.as_deref(), Some("snoozed"));
    }

    #[test]
    fn blocked_by_one_run_is_not_ready() {
        let s = derive_readiness("active", false, false, 1, RunPriority::Normal, None, false, None);
        assert!(!s.is_ready);
        assert_eq!(s.readiness_reason.as_deref(), Some("blocked by 1 run"));
    }

    #[test]
    fn blocked_by_multiple_runs_is_not_ready() {
        let s = derive_readiness("active", false, false, 3, RunPriority::Normal, None, false, None);
        assert!(!s.is_ready);
        assert_eq!(s.readiness_reason.as_deref(), Some("blocked by 3 runs"));
    }

    // ---- readiness: priority ordering (archived beats finalized beats snoozed beats blocked) ----

    #[test]
    fn archived_beats_snoozed_in_readiness_reason() {
        let s = derive_readiness("active", true, true, 1, RunPriority::Normal, None, false, None);
        assert!(!s.is_ready);
        assert_eq!(s.readiness_reason.as_deref(), Some("archived"));
    }

    #[test]
    fn finalized_beats_snoozed_in_readiness_reason() {
        let s = derive_readiness("finalized:completed", false, true, 1, RunPriority::Normal, None, false, None);
        assert!(!s.is_ready);
        assert_eq!(s.readiness_reason.as_deref(), Some("finalized"));
    }

    // ---- attention: no attention by default ----

    #[test]
    fn normal_ready_run_needs_no_attention() {
        let s = ready("active");
        assert!(!s.needs_attention);
        assert!(s.attention_reason.is_none());
    }

    #[test]
    fn high_priority_ready_run_needs_no_attention() {
        let s = derive_readiness("active", false, false, 0, RunPriority::High, None, false, None);
        assert!(!s.needs_attention);
    }

    // ---- attention: urgent ready run ----

    #[test]
    fn urgent_ready_run_needs_attention() {
        let s = derive_readiness("active", false, false, 0, RunPriority::Urgent, None, false, None);
        assert!(s.is_ready);
        assert!(s.needs_attention);
        assert_eq!(s.attention_reason.as_deref(), Some("urgent"));
    }

    #[test]
    fn urgent_but_blocked_run_does_not_trigger_urgent_attention() {
        // Blocked urgent run is not ready; urgent attention only fires for ready runs.
        // But it will still need attention for "pinned and blocked" if pinned.
        let s = derive_readiness("active", false, false, 1, RunPriority::Urgent, None, false, None);
        assert!(!s.is_ready);
        // No "urgent" attention reason since not ready; no pin attention either.
        assert!(!s.needs_attention);
    }

    // ---- attention: pinned and blocked ----

    #[test]
    fn pinned_blocked_run_needs_attention() {
        let s = derive_readiness("active", false, false, 2, RunPriority::Normal, None, true, None);
        assert!(!s.is_ready);
        assert!(s.needs_attention);
        assert_eq!(s.attention_reason.as_deref(), Some("pinned and blocked"));
    }

    #[test]
    fn pinned_ready_run_needs_no_attention() {
        let s = derive_readiness("active", false, false, 0, RunPriority::Normal, None, true, None);
        assert!(s.is_ready);
        assert!(!s.needs_attention);
    }

    // ---- attention: overdue ----

    #[test]
    fn overdue_run_needs_attention() {
        let s = derive_readiness(
            "active",
            false,
            false,
            0,
            RunPriority::Normal,
            Some("2025-01-01"),
            false,
            Some("2026-01-01"),
        );
        assert!(s.needs_attention);
        assert_eq!(s.attention_reason.as_deref(), Some("overdue"));
    }

    #[test]
    fn not_yet_due_run_needs_no_attention() {
        let s = derive_readiness(
            "active",
            false,
            false,
            0,
            RunPriority::Normal,
            Some("2030-12-31"),
            false,
            Some("2026-01-01"),
        );
        assert!(!s.needs_attention);
    }

    #[test]
    fn no_today_date_means_no_overdue_attention() {
        let s = derive_readiness(
            "active",
            false,
            false,
            0,
            RunPriority::Normal,
            Some("2020-01-01"),
            false,
            None, // no reference date
        );
        assert!(!s.needs_attention);
    }

    #[test]
    fn overdue_archived_run_needs_no_attention() {
        let s = derive_readiness(
            "active",
            true, // archived
            false,
            0,
            RunPriority::Normal,
            Some("2020-01-01"),
            false,
            Some("2026-01-01"),
        );
        assert!(!s.needs_attention);
    }

    #[test]
    fn overdue_snoozed_run_needs_no_attention() {
        let s = derive_readiness(
            "active",
            false,
            true, // snoozed
            0,
            RunPriority::Normal,
            Some("2020-01-01"),
            false,
            Some("2026-01-01"),
        );
        assert!(!s.needs_attention);
    }

    #[test]
    fn overdue_finalized_run_needs_no_attention() {
        let s = derive_readiness(
            "finalized:completed",
            false,
            false,
            0,
            RunPriority::Normal,
            Some("2020-01-01"),
            false,
            Some("2026-01-01"),
        );
        assert!(!s.needs_attention);
    }

    // ---- attention: combined reasons ----

    #[test]
    fn urgent_and_overdue_run_has_combined_attention() {
        let s = derive_readiness(
            "active",
            false,
            false,
            0,
            RunPriority::Urgent,
            Some("2020-01-01"),
            false,
            Some("2026-01-01"),
        );
        assert!(s.needs_attention);
        let reason = s.attention_reason.unwrap();
        assert!(reason.contains("urgent"), "expected 'urgent' in: {reason}");
        assert!(reason.contains("overdue"), "expected 'overdue' in: {reason}");
    }
}
