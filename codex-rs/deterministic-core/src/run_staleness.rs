//! Deterministic run age/staleness derivation (Milestone 26).
//!
//! All rules are derived from existing timestamps only.
//! No state mutation, no background processing, no model calls, no timers.

use deterministic_protocol::{RunStaleness, RunState};

/// Compact derived staleness summary for a run.
///
/// Computed deterministically from existing run state fields.
/// No mutations are performed; no autonomous transitions are triggered.
#[derive(Debug, Clone, PartialEq)]
pub struct RunStalenessSummary {
    /// Number of days since last update.
    pub age_days: usize,
    /// Whether this run is considered stale (>7 days).
    pub is_stale: bool,
    /// Concise reason for staleness classification.
    pub staleness_reason: String,
    /// Staleness bucket.
    pub staleness_bucket: RunStaleness,
}

/// Derive the staleness summary from an existing run's `updated_at` timestamp.
///
/// # Staleness rules
///
/// - `fresh`: updated within 3 days (age_days <= 3)
/// - `aging`: updated 4-7 days ago (3 < age_days <= 7)
/// - `stale`: updated more than 7 days ago (age_days > 7)
///
/// # Parameters
///
/// - `updated_at`: ISO 8601 timestamp of last update (e.g., "2024-01-15T10:30:00Z")
/// - `reference_date`: ISO date string for comparison (e.g., "2024-01-18")
///
/// If either parameter is missing or invalid, returns a default "fresh" classification.
///
/// No timers, notifications, escalations, or state mutations are performed.
#[allow(clippy::collapsible_if)]
pub fn derive_staleness(updated_at: &str, reference_date: &str) -> RunStalenessSummary {
    // Try to parse dates and compute age
    let age_days = compute_age_days(updated_at, reference_date);

    // Determine staleness bucket and reason
    let (staleness_bucket, staleness_reason, is_stale) = if age_days <= 3 {
        (
            RunStaleness::Fresh,
            format!("updated {} day(s) ago", age_days),
            false,
        )
    } else if age_days <= 7 {
        (
            RunStaleness::Aging,
            format!("updated {} days ago", age_days),
            false,
        )
    } else {
        (
            RunStaleness::Stale,
            format!("stale: {} days since update", age_days),
            true,
        )
    };

    RunStalenessSummary {
        age_days,
        is_stale,
        staleness_reason,
        staleness_bucket,
    }
}

/// Compute the number of days between updated_at and reference_date.
///
/// Both should be ISO 8601 format:
/// - `updated_at`: "2024-01-15T10:30:00Z" (full timestamp)
/// - `reference_date`: "2024-01-18" (date only)
///
/// Returns 0 if parsing fails.
fn compute_age_days(updated_at: &str, reference_date: &str) -> usize {
    // Parse the reference date (YYYY-MM-DD)
    let ref_date = match chrono::NaiveDate::parse_from_str(reference_date, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return 0,
    };

    // Parse the updated_at timestamp - extract just the date part
    let updated_date_str = if let Some(d) = updated_at.split('T').next() {
        d
    } else {
        return 0;
    };

    let updated_date = match chrono::NaiveDate::parse_from_str(updated_date_str, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return 0,
    };

    // Compute difference in days
    let diff = ref_date.signed_duration_since(updated_date);
    let days = diff.num_days();

    // Return 0 for negative (future dates) or invalid
    if days < 0 {
        0
    } else {
        days as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fresh_run() {
        let result = derive_staleness("2024-01-15T10:30:00Z", "2024-01-16");
        assert_eq!(result.age_days, 1);
        assert!(!result.is_stale);
        assert_eq!(result.staleness_bucket, RunStaleness::Fresh);
    }

    #[test]
    fn test_aging_run() {
        let result = derive_staleness("2024-01-10T10:30:00Z", "2024-01-14");
        assert_eq!(result.age_days, 4);
        assert!(!result.is_stale);
        assert_eq!(result.staleness_bucket, RunStaleness::Aging);
    }

    #[test]
    fn test_stale_run() {
        let result = derive_staleness("2024-01-01T10:30:00Z", "2024-01-15");
        assert_eq!(result.age_days, 14);
        assert!(result.is_stale);
        assert_eq!(result.staleness_bucket, RunStaleness::Stale);
    }

    #[test]
    fn test_boundary_fresh() {
        // Exactly 3 days = fresh
        let result = derive_staleness("2024-01-12T10:30:00Z", "2024-01-15");
        assert_eq!(result.age_days, 3);
        assert_eq!(result.staleness_bucket, RunStaleness::Fresh);
    }

    #[test]
    fn test_boundary_aging() {
        // Exactly 4 days = aging
        let result = derive_staleness("2024-01-11T10:30:00Z", "2024-01-15");
        assert_eq!(result.age_days, 4);
        assert_eq!(result.staleness_bucket, RunStaleness::Aging);
    }

    #[test]
    fn test_boundary_stale() {
        // Exactly 8 days = stale
        let result = derive_staleness("2024-01-07T10:30:00Z", "2024-01-15");
        assert_eq!(result.age_days, 8);
        assert_eq!(result.staleness_bucket, RunStaleness::Stale);
    }

    #[test]
    fn test_invalid_timestamp_returns_fresh() {
        let result = derive_staleness("not-a-timestamp", "2024-01-15");
        assert_eq!(result.age_days, 0);
        assert_eq!(result.staleness_bucket, RunStaleness::Fresh);
    }

    #[test]
    fn test_invalid_reference_returns_fresh() {
        let result = derive_staleness("2024-01-01T10:30:00Z", "not-a-date");
        assert_eq!(result.age_days, 0);
        assert_eq!(result.staleness_bucket, RunStaleness::Fresh);
    }
}