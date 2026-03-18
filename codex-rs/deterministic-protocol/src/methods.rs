//! Canonical method names for the internal JSON-RPC surface.
//!
//! Only deterministic methods may appear here.  If a method would imply
//! backend-owned reasoning or autonomous agent iteration it **must not**
//! be added.

use std::fmt;

/// Allowed internal JSON-RPC methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    RunPrepare,
    RunRefresh,
    RunReplan,
    WorkspaceSummary,
    FileRead,
    GitStatus,
    CodeSearch,
    PatchApply,
    TestsRun,
    GitDiff,
    ApprovalResolve,
    // Milestone 7: read-only history and state inspection
    RunsList,
    RunGet,
    RunHistory,
    // Milestone 9: deterministic preflight / preview (read-only)
    PatchPreflight,
    TestsPreflight,
    // Milestone 10: deterministic run finalization
    RunFinalize,
    // Milestone 11: deterministic run reopening
    RunReopen,
    // Milestone 12: deterministic run supersession
    RunSupersede,
    // Milestone 13: deterministic run archiving
    RunArchive,
    // Milestone 14: deterministic run unarchiving
    RunUnarchive,
    // Milestone 15: deterministic run labeling / annotation
    RunAnnotate,
    // Milestone 16: deterministic run pinning
    RunPin,
    RunUnpin,
    // Milestone 17: deterministic run snoozing
    RunSnooze,
    RunUnsnooze,
    // Milestone 18: deterministic run priority
    RunSetPriority,
    // Milestone 19: deterministic run ownership/assignee
    RunAssignOwner,
    // Milestone 20: deterministic run due dates
    RunSetDueDate,
    // Milestone 21: deterministic run dependency links
    RunSetDependencies,
    // Milestone 24: deterministic queue overview
    RunsQueueOverview,
    // Milestone 25: deterministic run effort estimates
    RunSetEffort,
}

impl Method {
    /// The canonical wire name used in JSON-RPC `"method"` fields.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RunPrepare => "run.prepare",
            Self::RunRefresh => "run.refresh",
            Self::RunReplan => "run.replan",
            Self::WorkspaceSummary => "workspace.summary",
            Self::FileRead => "file.read",
            Self::GitStatus => "git.status",
            Self::CodeSearch => "code.search",
            Self::PatchApply => "patch.apply",
            Self::TestsRun => "tests.run",
            Self::GitDiff => "git.diff",
            Self::ApprovalResolve => "approval.resolve",
            // Milestone 7
            Self::RunsList => "runs.list",
            Self::RunGet => "run.get",
            Self::RunHistory => "run.history",
            // Milestone 9
            Self::PatchPreflight => "patch.preflight",
            Self::TestsPreflight => "tests.preflight",
            // Milestone 10
            Self::RunFinalize => "run.finalize",
            // Milestone 11
            Self::RunReopen => "run.reopen",
            // Milestone 12
            Self::RunSupersede => "run.supersede",
            // Milestone 13
            Self::RunArchive => "run.archive",
            // Milestone 14
            Self::RunUnarchive => "run.unarchive",
            // Milestone 15
            Self::RunAnnotate => "run.annotate",
            // Milestone 16
            Self::RunPin => "run.pin",
            Self::RunUnpin => "run.unpin",
            // Milestone 17
            Self::RunSnooze => "run.snooze",
            Self::RunUnsnooze => "run.unsnooze",
            // Milestone 18
            Self::RunSetPriority => "run.set_priority",
            // Milestone 19
            Self::RunAssignOwner => "run.assign_owner",
            // Milestone 20
            Self::RunSetDueDate => "run.set_due_date",
            // Milestone 21
            Self::RunSetDependencies => "run.set_dependencies",
            // Milestone 24
            Self::RunsQueueOverview => "runs.overview",
            // Milestone 25
            Self::RunSetEffort => "run.set_effort",
        }
    }

    /// Parse a wire name into a [`Method`].
    pub fn parse_method(s: &str) -> Option<Self> {
        match s {
            "run.prepare" => Some(Self::RunPrepare),
            "run.refresh" => Some(Self::RunRefresh),
            "run.replan" => Some(Self::RunReplan),
            "workspace.summary" => Some(Self::WorkspaceSummary),
            "file.read" => Some(Self::FileRead),
            "git.status" => Some(Self::GitStatus),
            "code.search" => Some(Self::CodeSearch),
            "patch.apply" => Some(Self::PatchApply),
            "tests.run" => Some(Self::TestsRun),
            "git.diff" => Some(Self::GitDiff),
            "approval.resolve" => Some(Self::ApprovalResolve),
            // Milestone 7
            "runs.list" => Some(Self::RunsList),
            "run.get" => Some(Self::RunGet),
            "run.history" => Some(Self::RunHistory),
            // Milestone 9
            "patch.preflight" => Some(Self::PatchPreflight),
            "tests.preflight" => Some(Self::TestsPreflight),
            // Milestone 10
            "run.finalize" => Some(Self::RunFinalize),
            // Milestone 11
            "run.reopen" => Some(Self::RunReopen),
            // Milestone 12
            "run.supersede" => Some(Self::RunSupersede),
            // Milestone 13
            "run.archive" => Some(Self::RunArchive),
            // Milestone 14
            "run.unarchive" => Some(Self::RunUnarchive),
            // Milestone 15
            "run.annotate" => Some(Self::RunAnnotate),
            // Milestone 16
            "run.pin" => Some(Self::RunPin),
            "run.unpin" => Some(Self::RunUnpin),
            // Milestone 17
            "run.snooze" => Some(Self::RunSnooze),
            "run.unsnooze" => Some(Self::RunUnsnooze),
            // Milestone 18
            "run.set_priority" => Some(Self::RunSetPriority),
            // Milestone 19
            "run.assign_owner" => Some(Self::RunAssignOwner),
            // Milestone 20
            "run.set_due_date" => Some(Self::RunSetDueDate),
            // Milestone 21
            "run.set_dependencies" => Some(Self::RunSetDependencies),
            // Milestone 24
            "runs.overview" => Some(Self::RunsQueueOverview),
            _ => None,
        }
    }

    /// All registered methods.
    pub fn all() -> &'static [Method] {
        &[
            Self::RunPrepare,
            Self::RunRefresh,
            Self::RunReplan,
            Self::WorkspaceSummary,
            Self::FileRead,
            Self::GitStatus,
            Self::CodeSearch,
            Self::PatchApply,
            Self::TestsRun,
            Self::GitDiff,
            Self::ApprovalResolve,
            // Milestone 7
            Self::RunsList,
            Self::RunGet,
            Self::RunHistory,
            // Milestone 9
            Self::PatchPreflight,
            Self::TestsPreflight,
            // Milestone 10
            Self::RunFinalize,
            // Milestone 11
            Self::RunReopen,
            // Milestone 12
            Self::RunSupersede,
            // Milestone 13
            Self::RunArchive,
            // Milestone 14
            Self::RunUnarchive,
            // Milestone 15
            Self::RunAnnotate,
            // Milestone 16
            Self::RunPin,
            Self::RunUnpin,
            // Milestone 17
            Self::RunSnooze,
            Self::RunUnsnooze,
            // Milestone 18
            Self::RunSetPriority,
            // Milestone 19
            Self::RunAssignOwner,
            // Milestone 20
            Self::RunSetDueDate,
            // Milestone 21
            Self::RunSetDependencies,
            // Milestone 24
            Self::RunsQueueOverview,
        ]
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Strings that **must never** appear as daemon method names.
pub const FORBIDDEN_METHODS: &[&str] = &[
    "turn.start",
    "turn.steer",
    "review.start",
    "agent.step",
    "run.continue",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_forbidden_methods_registered() {
        for m in Method::all() {
            let name = m.as_str();
            assert!(
                !FORBIDDEN_METHODS.contains(&name),
                "forbidden method registered: {name}"
            );
        }
    }

    #[test]
    fn roundtrip() {
        for m in Method::all() {
            assert_eq!(Method::parse_method(m.as_str()), Some(*m));
        }
    }
}
