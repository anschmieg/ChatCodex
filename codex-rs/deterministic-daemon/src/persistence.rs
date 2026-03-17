//! SQLite-backed persistence for run state.
//!
//! The daemon stores all run state in a local SQLite database.  This
//! provides ACID transactions, schema enforcement, and safe concurrent
//! access — unlike the previous JSON-file approach.

use anyhow::{Context, Result};
use deterministic_protocol::{ArchiveMetadata, PendingApproval, PinMetadata, ReopenMetadata, RetryableAction, RunAnnotation, RunHistoryEntry, RunOutcome, RunPolicy, RunPriority, RunState, RunSummary, SnoozeMetadata, UnarchiveMetadata};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

/// Run-state store backed by SQLite.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (or create) the SQLite database at `dir/runs.db`.
    ///
    /// Runs migrations on first open to ensure the schema is up to date.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).context("cannot create store directory")?;
        let db_path = dir.join("runs.db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("cannot open SQLite database at {}", db_path.display()))?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Create an in-memory store (useful for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("cannot open in-memory SQLite")?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Apply database migrations.
    ///
    /// Handles both fresh database creation and upgrades from older schemas.
    /// Uses ALTER TABLE to add missing columns for backward compatibility.
    fn migrate(conn: &Connection) -> Result<()> {
        // Create the base tables if they don't exist.
        // Note: This creates tables with the full Milestone 5 schema for new databases.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS runs (
                run_id                   TEXT PRIMARY KEY,
                workspace_id             TEXT NOT NULL,
                user_goal                TEXT NOT NULL,
                status                   TEXT NOT NULL,
                plan                     TEXT NOT NULL,   -- JSON array of strings
                current_step             INTEGER NOT NULL DEFAULT 0,
                completed_steps          TEXT NOT NULL DEFAULT '[]',
                pending_steps            TEXT NOT NULL DEFAULT '[]',
                last_action              TEXT,
                last_observation         TEXT,
                recommended_next_action  TEXT,
                recommended_tool         TEXT,
                latest_diff_summary      TEXT,
                latest_test_result       TEXT,
                focus_paths              TEXT NOT NULL DEFAULT '[]',
                warnings                 TEXT NOT NULL DEFAULT '[]',
                created_at               TEXT NOT NULL,
                updated_at               TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS approvals (
                approval_id         TEXT PRIMARY KEY,
                run_id              TEXT NOT NULL,
                action_description  TEXT NOT NULL,
                risk_reason         TEXT NOT NULL,
                policy_rationale    TEXT NOT NULL DEFAULT '',
                status              TEXT NOT NULL DEFAULT 'pending',
                decision            TEXT,
                decision_reason     TEXT,
                created_at          TEXT NOT NULL,
                resolved_at         TEXT,
                FOREIGN KEY (run_id) REFERENCES runs(run_id)
            );
            CREATE TABLE IF NOT EXISTS audit_trail (
                entry_id    TEXT PRIMARY KEY,
                run_id      TEXT NOT NULL,
                event_kind  TEXT NOT NULL,
                summary     TEXT NOT NULL,
                metadata    TEXT,
                occurred_at TEXT NOT NULL,
                FOREIGN KEY (run_id) REFERENCES runs(run_id)
            );",
        )
        .context("failed to create tables")?;

        // Migrate older databases: add columns that may be missing from earlier milestones.
        // SQLite supports ALTER TABLE ADD COLUMN; we ignore errors if columns already exist.
        let migrations = [
            // Milestone 4 columns
            ("runs", "completed_steps", "TEXT NOT NULL DEFAULT '[]'"),
            ("runs", "pending_steps", "TEXT NOT NULL DEFAULT '[]'"),
            ("runs", "last_action", "TEXT"),
            ("runs", "last_observation", "TEXT"),
            ("runs", "recommended_next_action", "TEXT"),
            ("runs", "recommended_tool", "TEXT"),
            ("runs", "latest_diff_summary", "TEXT"),
            ("runs", "latest_test_result", "TEXT"),
            ("runs", "warnings", "TEXT NOT NULL DEFAULT '[]'"),
            // Milestone 5 columns
            ("runs", "focus_paths", "TEXT NOT NULL DEFAULT '[]'"),
            ("approvals", "policy_rationale", "TEXT NOT NULL DEFAULT ''"),
            // Milestone 6 columns
            ("runs", "retryable_action", "TEXT"),
            // Milestone 8 columns
            ("runs", "policy_profile", "TEXT NOT NULL DEFAULT '{}'"),
            // Milestone 10 columns
            ("runs", "outcome_kind", "TEXT"),
            ("runs", "finalized_outcome", "TEXT"),
            // Milestone 11 columns
            ("runs", "reopen_metadata", "TEXT"),
            // Milestone 12 columns
            ("runs", "supersedes_run_id", "TEXT"),
            ("runs", "superseded_by_run_id", "TEXT"),
            ("runs", "supersession_reason", "TEXT"),
            ("runs", "superseded_at", "TEXT"),
            // Milestone 13 columns
            ("runs", "is_archived", "INTEGER DEFAULT 0"),
            ("runs", "archive_metadata", "TEXT"),
            // Milestone 14 columns
            ("runs", "unarchive_metadata", "TEXT"),
            // Milestone 15 columns
            ("runs", "annotation", "TEXT"),
            // Milestone 16 columns
            ("runs", "pin_metadata", "TEXT"),
            // Milestone 17 columns
            ("runs", "is_snoozed", "INTEGER DEFAULT 0"),
            ("runs", "snooze_metadata", "TEXT"),
            // Milestone 18 columns
            ("runs", "priority", "TEXT NOT NULL DEFAULT 'normal'"),
            // Milestone 19 columns
            ("runs", "assignee", "TEXT"),
            ("runs", "ownership_note", "TEXT"),
            // Milestone 20 columns
            ("runs", "due_date", "TEXT"),
        ];

        for (table, column, def) in migrations {
            let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {def}");
            match conn.execute(&sql, []) {
                Ok(_) => {}
                Err(rusqlite::Error::SqliteFailure(_code, Some(msg)))
                    if msg.contains("duplicate column") =>
                {
                    // Column already exists — this is fine.
                }
                Err(e) => return Err(e).with_context(|| {
                    format!("failed to add column {column} to {table}")
                }),
            }
        }

        // Milestone 7: ensure audit_trail table exists (for older DBs without it).
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit_trail (
                entry_id    TEXT PRIMARY KEY,
                run_id      TEXT NOT NULL,
                event_kind  TEXT NOT NULL,
                summary     TEXT NOT NULL,
                metadata    TEXT,
                occurred_at TEXT NOT NULL,
                FOREIGN KEY (run_id) REFERENCES runs(run_id)
            );",
        )
        .context("failed to create audit_trail table")?;

        Ok(())
    }

    /// Persist a run state (insert or replace).
    pub fn save_run(&self, state: &RunState) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let plan_json =
            serde_json::to_string(&state.plan).context("failed to serialise plan")?;
        let completed_json = serde_json::to_string(&state.completed_steps)
            .context("failed to serialise completed_steps")?;
        let pending_json = serde_json::to_string(&state.pending_steps)
            .context("failed to serialise pending_steps")?;
        let focus_paths_json = serde_json::to_string(&state.focus_paths)
            .context("failed to serialise focus_paths")?;
        let warnings_json =
            serde_json::to_string(&state.warnings).context("failed to serialise warnings")?;
        let retryable_action_json: Option<String> = state
            .retryable_action
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("failed to serialise retryable_action")?;
        let policy_profile_json = serde_json::to_string(&state.policy_profile)
            .context("failed to serialise policy_profile")?;
        // Milestone 10: persist finalized_outcome as JSON; also extract outcome_kind for querying.
        let outcome_kind: Option<&str> = state
            .finalized_outcome
            .as_ref()
            .map(|o| o.outcome_kind.as_str());
        let finalized_outcome_json: Option<String> = state
            .finalized_outcome
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("failed to serialise finalized_outcome")?;
        // Milestone 11: persist reopen_metadata as JSON.
        let reopen_metadata_json: Option<String> = state
            .reopen_metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("failed to serialise reopen_metadata")?;
        // Milestone 13: persist archive_metadata as JSON; also extract is_archived for filtering.
        // Milestone 14: a run is archived only if archive_metadata is set AND unarchive_metadata is not set.
        let is_archived: i64 = if state.archive_metadata.is_some() && state.unarchive_metadata.is_none() { 1 } else { 0 };
        let archive_metadata_json: Option<String> = state
            .archive_metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("failed to serialise archive_metadata")?;
        // Milestone 14: persist unarchive_metadata as JSON.
        let unarchive_metadata_json: Option<String> = state
            .unarchive_metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("failed to serialise unarchive_metadata")?;
        // Milestone 15: persist annotation as JSON.
        let annotation_json: Option<String> = state
            .annotation
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("failed to serialise annotation")?;
        // Milestone 16: persist pin_metadata as JSON.
        let pin_metadata_json: Option<String> = state
            .pin_metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("failed to serialise pin_metadata")?;
        // Milestone 17: persist snooze_metadata as JSON; also extract is_snoozed for filtering.
        let is_snoozed: i64 = if state.snooze_metadata.is_some() { 1 } else { 0 };
        let snooze_metadata_json: Option<String> = state
            .snooze_metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("failed to serialise snooze_metadata")?;
        // Milestone 18: persist priority as string.
        let priority_str = state.priority.as_str();
        conn.execute(
            "INSERT OR REPLACE INTO runs
                (run_id, workspace_id, user_goal, status, plan, current_step,
                 completed_steps, pending_steps, last_action, last_observation,
                 recommended_next_action, recommended_tool,
                 latest_diff_summary, latest_test_result, focus_paths, warnings,
                 retryable_action, policy_profile, outcome_kind, finalized_outcome,
                 reopen_metadata, supersedes_run_id, superseded_by_run_id,
                 supersession_reason, superseded_at, is_archived, archive_metadata,
                 unarchive_metadata, annotation, pin_metadata,
                 is_snoozed, snooze_metadata, priority,
                 assignee, ownership_note, due_date,
                 created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37, ?38)",
            rusqlite::params![
                state.run_id,
                state.workspace_id,
                state.user_goal,
                state.status,
                plan_json,
                state.current_step,
                completed_json,
                pending_json,
                state.last_action,
                state.last_observation,
                state.recommended_next_action,
                state.recommended_tool,
                state.latest_diff_summary,
                state.latest_test_result,
                focus_paths_json,
                warnings_json,
                retryable_action_json,
                policy_profile_json,
                outcome_kind,
                finalized_outcome_json,
                reopen_metadata_json,
                state.supersedes_run_id,
                state.superseded_by_run_id,
                state.supersession_reason,
                state.superseded_at,
                is_archived,
                archive_metadata_json,
                unarchive_metadata_json,
                annotation_json,
                pin_metadata_json,
                is_snoozed,
                snooze_metadata_json,
                priority_str,
                state.assignee,
                state.ownership_note,
                state.due_date,
                state.created_at,
                state.updated_at,
            ],
        )
        .context("failed to save run")?;
        Ok(())
    }

    /// Retrieve a run state by ID.
    pub fn get_run(&self, run_id: &str) -> Result<Option<RunState>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn
            .prepare(
                "SELECT run_id, workspace_id, user_goal, status, plan,
                        current_step, completed_steps, pending_steps,
                        last_action, last_observation,
                        recommended_next_action, recommended_tool,
                        latest_diff_summary, latest_test_result, focus_paths, warnings,
                        retryable_action, policy_profile, finalized_outcome,
                        reopen_metadata, supersedes_run_id, superseded_by_run_id,
                        supersession_reason, superseded_at, archive_metadata,
                        unarchive_metadata, annotation, pin_metadata,
                        snooze_metadata, priority,
                        assignee, ownership_note, due_date,
                        created_at, updated_at
                 FROM runs WHERE run_id = ?1",
            )
            .context("failed to prepare statement")?;

        let mut rows = stmt
            .query_map(rusqlite::params![run_id], |row| {
                let plan_json: String = row.get(4)?;
                let completed_json: String = row.get(6)?;
                let pending_json: String = row.get(7)?;
                let focus_paths_json: String = row.get(14)?;
                let warnings_json: String = row.get(15)?;

                let plan: Vec<String> =
                    serde_json::from_str(&plan_json).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            4,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;
                let completed_steps: Vec<String> =
                    serde_json::from_str(&completed_json).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            6,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;
                let pending_steps: Vec<String> =
                    serde_json::from_str(&pending_json).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            7,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;
                let focus_paths: Vec<String> =
                    serde_json::from_str(&focus_paths_json).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            14,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;
                let warnings: Vec<String> =
                    serde_json::from_str(&warnings_json).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            15,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;

                let retryable_action_json: Option<String> = row.get(16)?;
                let retryable_action: Option<RetryableAction> = retryable_action_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            16,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;

                // Milestone 8: policy_profile — fall back to defaults if the column is
                // empty (older databases migrated with DEFAULT '{}').
                let policy_profile_json: Option<String> = row.get(17)?;
                let policy_profile: RunPolicy = policy_profile_json
                    .as_deref()
                    .filter(|s| !s.is_empty() && *s != "{}")
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            17,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?
                    .unwrap_or_default();

                // Milestone 10: finalized_outcome — NULL for unfinalized runs.
                let finalized_outcome_json: Option<String> = row.get(18)?;
                let finalized_outcome: Option<RunOutcome> = finalized_outcome_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            18,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;

                // Milestone 11: reopen_metadata — NULL for runs never reopened.
                let reopen_metadata_json: Option<String> = row.get(19)?;
                let reopen_metadata: Option<ReopenMetadata> = reopen_metadata_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            19,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;

                // Milestone 12: supersession lineage fields.
                let supersedes_run_id: Option<String> = row.get(20)?;
                let superseded_by_run_id: Option<String> = row.get(21)?;
                let supersession_reason: Option<String> = row.get(22)?;
                let superseded_at: Option<String> = row.get(23)?;

                // Milestone 13: archive metadata.
                let archive_metadata_json: Option<String> = row.get(24)?;
                let archive_metadata: Option<ArchiveMetadata> = archive_metadata_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            24,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;

                // Milestone 14: unarchive metadata.
                let unarchive_metadata_json: Option<String> = row.get(25)?;
                let unarchive_metadata: Option<UnarchiveMetadata> = unarchive_metadata_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            25,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;

                // Milestone 15: annotation metadata.
                let annotation_json: Option<String> = row.get(26)?;
                let annotation: Option<RunAnnotation> = annotation_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            26,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;

                // Milestone 16: pin metadata.
                let pin_metadata_json: Option<String> = row.get(27)?;
                let pin_metadata: Option<PinMetadata> = pin_metadata_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            27,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;

                // Milestone 17: snooze metadata.
                let snooze_metadata_json: Option<String> = row.get(28)?;
                let snooze_metadata: Option<SnoozeMetadata> = snooze_metadata_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            28,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;
                // Milestone 18: explicit priority metadata.
                let priority_str: String = row.get(29)?;
                let priority = RunPriority::parse(&priority_str).ok_or_else(|| {
                    rusqlite::Error::FromSqlConversionFailure(
                        29,
                        rusqlite::types::Type::Text,
                        format!("invalid run priority in SQLite: '{priority_str}'").into(),
                    )
                })?;

                // Milestone 19: ownership metadata.
                let assignee: Option<String> = row.get(30)?;
                let ownership_note: Option<String> = row.get(31)?;

                // Milestone 20: due date.
                let due_date: Option<String> = row.get(32)?;

                Ok(RunState {
                    run_id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    user_goal: row.get(2)?,
                    status: row.get(3)?,
                    plan,
                    current_step: row.get::<_, i64>(5)? as usize,
                    completed_steps,
                    pending_steps,
                    last_action: row.get(8)?,
                    last_observation: row.get(9)?,
                    recommended_next_action: row.get(10)?,
                    recommended_tool: row.get(11)?,
                    latest_diff_summary: row.get(12)?,
                    latest_test_result: row.get(13)?,
                    focus_paths,
                    warnings,
                    retryable_action,
                    policy_profile,
                    finalized_outcome,
                    reopen_metadata,
                    supersedes_run_id,
                    superseded_by_run_id,
                    supersession_reason,
                    superseded_at,
                    archive_metadata,
                    unarchive_metadata,
                    annotation,
                    pin_metadata,
                    snooze_metadata,
                    priority,
                    assignee,
                    ownership_note,
                    due_date,
                    created_at: row.get(33)?,
                    updated_at: row.get(34)?,
                })
            })
            .context("failed to query run")?;

        match rows.next() {
            Some(Ok(state)) => Ok(Some(state)),
            Some(Err(e)) => Err(anyhow::anyhow!("failed to read row: {e}")),
            None => Ok(None),
        }
    }

    /// Get the workspace root for a run.
    pub fn workspace_for_run(&self, run_id: &str) -> Result<Option<String>> {
        Ok(self.get_run(run_id)?.map(|r| r.workspace_id))
    }

    // ----- Approval persistence -----

    /// Insert a new pending approval.
    pub fn save_approval(&self, approval: &PendingApproval) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT OR REPLACE INTO approvals
                (approval_id, run_id, action_description, risk_reason, policy_rationale, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                approval.approval_id,
                approval.run_id,
                approval.action_description,
                approval.risk_reason,
                approval.policy_rationale,
                approval.status,
                approval.created_at,
            ],
        )
        .context("failed to save approval")?;
        Ok(())
    }

    /// Retrieve all pending approvals for a run.
    pub fn get_pending_approvals(&self, run_id: &str) -> Result<Vec<PendingApproval>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn
            .prepare(
                "SELECT approval_id, run_id, action_description, risk_reason, policy_rationale, status, created_at
                 FROM approvals WHERE run_id = ?1 AND status = 'pending'",
            )
            .context("failed to prepare statement")?;

        let rows = stmt
            .query_map(rusqlite::params![run_id], |row| {
                Ok(PendingApproval {
                    approval_id: row.get(0)?,
                    run_id: row.get(1)?,
                    action_description: row.get(2)?,
                    risk_reason: row.get(3)?,
                    policy_rationale: row.get(4)?,
                    status: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .context("failed to query approvals")?;

        let mut approvals = Vec::new();
        for row in rows {
            approvals.push(row.map_err(|e| anyhow::anyhow!("failed to read approval: {e}"))?);
        }
        Ok(approvals)
    }

    /// Retrieve a specific approval by ID.
    pub fn get_approval(&self, approval_id: &str) -> Result<Option<PendingApproval>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn
            .prepare(
                "SELECT approval_id, run_id, action_description, risk_reason, policy_rationale, status, created_at
                 FROM approvals WHERE approval_id = ?1",
            )
            .context("failed to prepare statement")?;

        let mut rows = stmt
            .query_map(rusqlite::params![approval_id], |row| {
                Ok(PendingApproval {
                    approval_id: row.get(0)?,
                    run_id: row.get(1)?,
                    action_description: row.get(2)?,
                    risk_reason: row.get(3)?,
                    policy_rationale: row.get(4)?,
                    status: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .context("failed to query approval")?;

        match rows.next() {
            Some(Ok(a)) => Ok(Some(a)),
            Some(Err(e)) => Err(anyhow::anyhow!("failed to read approval: {e}")),
            None => Ok(None),
        }
    }

    /// Resolve an approval by ID (set decision/status).
    pub fn resolve_approval(
        &self,
        approval_id: &str,
        decision: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let now = chrono::Utc::now().to_rfc3339();
        let status = match decision {
            "approve" => "approved",
            "deny" => "denied",
            _ => return Err(anyhow::anyhow!("invalid decision: must be 'approve' or 'deny'")),
        };
        let affected = conn
            .execute(
                "UPDATE approvals SET status = ?1, decision = ?2, decision_reason = ?3, resolved_at = ?4
                 WHERE approval_id = ?5 AND status = 'pending'",
                rusqlite::params![status, decision, reason, now, approval_id],
            )
            .context("failed to resolve approval")?;
        if affected == 0 {
            return Err(anyhow::anyhow!(
                "approval not found or already resolved: {approval_id}"
            ));
        }
        Ok(())
    }

    // ----- Milestone 7: run listing -----

    /// List runs, ordered by pinned-first then updated_at descending.
    ///
    /// Milestone 15: `label_filter` performs an exact normalized label match.
    /// Milestone 16: `pinned_only` filters to only pinned runs.
    ///              Pinned runs are always returned before non-pinned runs.
    /// Milestone 17: `include_snoozed` includes snoozed runs; `snoozed_only` returns only snoozed runs.
    ///              Default: snoozed runs are excluded.
    #[allow(clippy::too_many_arguments)]
    pub fn list_runs(
        &self,
        limit: usize,
        workspace_id: Option<&str>,
        status_filter: Option<&str>,
        include_archived: bool,
        archived_only: bool,
        label_filter: Option<&str>,
        pinned_only: bool,
        include_snoozed: bool,
        snoozed_only: bool,
    ) -> Result<Vec<RunSummary>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;

        // Build WHERE conditions and bind parameters dynamically.
        // Parameter ?1 is always `limit` (i64).  Additional parameters start at ?2.
        let mut conditions: Vec<String> = Vec::new();
        let mut next_param: usize = 2; // ?1 is limit

        // Track which positional index each optional param maps to.
        let mut workspace_idx: Option<usize> = None;
        let mut status_idx: Option<usize> = None;

        if workspace_id.is_some() {
            workspace_idx = Some(next_param);
            conditions.push(format!("workspace_id = ?{next_param}"));
            next_param += 1;
        }
        if status_filter.is_some() {
            status_idx = Some(next_param);
            conditions.push(format!("status = ?{next_param}"));
            let _ = next_param;
        }

        // Milestone 13: archive filtering.
        // archived_only takes precedence over include_archived.
        if archived_only {
            conditions.push("is_archived = 1".to_string());
        } else if !include_archived {
            // Default: exclude archived runs.
            conditions.push("is_archived = 0".to_string());
        }
        // If include_archived=true and archived_only=false, no condition is added (show all).

        // Milestone 17: snooze filtering (SQL-level for performance).
        // snoozed_only takes precedence over include_snoozed.
        if snoozed_only {
            conditions.push("is_snoozed = 1".to_string());
        } else if !include_snoozed {
            // Default: exclude snoozed runs.
            conditions.push("is_snoozed = 0".to_string());
        }
        // If include_snoozed=true and snoozed_only=false, no condition is added (show all).

        // Milestone 15: label filtering.
        // SQLite JSON functions are not always available, so we filter in Rust after fetching.
        // The label_filter is used post-query.

        // Milestone 16: pinned_only.
        // The pin_metadata column is post-query filtered in Rust.

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        // Milestone 16: pinned-first ordering.
        // Runs with pin_metadata IS NOT NULL come first, then sorted by updated_at DESC.
        let sql = format!(
            "SELECT run_id, workspace_id, user_goal, status, current_step, plan,
                    created_at, updated_at, outcome_kind, reopen_metadata,
                    supersedes_run_id, superseded_by_run_id, is_archived, archive_metadata,
                    unarchive_metadata, annotation, pin_metadata, snooze_metadata, priority,
                    assignee, due_date
             FROM runs {where_clause}
             ORDER BY CASE WHEN pin_metadata IS NOT NULL THEN 0 ELSE 1 END ASC, updated_at DESC
             LIMIT ?1"
        );

        let mut stmt = conn.prepare(&sql).context("failed to prepare list_runs")?;

        let mapper = |row: &rusqlite::Row<'_>| -> rusqlite::Result<RunSummary> {
            // Column index map (must stay in sync with SELECT above):
            //  0: run_id  1: workspace_id  2: user_goal  3: status
            //  4: current_step  5: plan  6: created_at  7: updated_at
            //  8: outcome_kind  9: reopen_metadata
            // 10: supersedes_run_id  11: superseded_by_run_id
            // 12: is_archived  13: archive_metadata  14: unarchive_metadata
            // 15: annotation  16: pin_metadata  17: snooze_metadata  18: priority
            // 19: assignee  20: due_date
            let plan_json: String = row.get(5)?;
            let total_steps: usize = serde_json::from_str::<Vec<String>>(&plan_json)
                .map(|v| v.len())
                .unwrap_or(0);
            // Extract reopen_count from reopen_metadata JSON if present.
            let reopen_metadata_json: Option<String> = row.get(9)?;
            let reopen_count: Option<u32> = reopen_metadata_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<ReopenMetadata>(s).ok())
                .map(|m| m.reopen_count);
            // Milestone 13: archive summary fields.
            let is_archived_int: i64 = row.get(12)?;
            let is_archived = if is_archived_int != 0 { Some(true) } else { None };
            let archive_metadata_json: Option<String> = row.get(13)?;
            let (archive_reason, archived_at) = archive_metadata_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<ArchiveMetadata>(s).ok())
                .map(|m| (Some(m.reason), Some(m.archived_at)))
                .unwrap_or((None, None));
            // Milestone 14: unarchive summary fields.
            let unarchive_metadata_json: Option<String> = row.get(14)?;
            let (unarchive_reason, unarchived_at) = unarchive_metadata_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<UnarchiveMetadata>(s).ok())
                .map(|m| (Some(m.reason), Some(m.unarchived_at)))
                .unwrap_or((None, None));
            // Milestone 15: annotation summary fields.
            let annotation_json: Option<String> = row.get(15)?;
            let annotation = annotation_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<RunAnnotation>(s).ok());
            let (labels, operator_note) = annotation
                .map(|a| (a.labels, a.operator_note))
                .unwrap_or_default();
            // Milestone 16: pin summary fields.
            let pin_metadata_json: Option<String> = row.get(16)?;
            let pin_metadata = pin_metadata_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<PinMetadata>(s).ok());
            let (is_pinned, pin_reason, pinned_at) = pin_metadata
                .map(|m| (Some(true), Some(m.reason), Some(m.pinned_at)))
                .unwrap_or((None, None, None));
            // Milestone 17: snooze summary fields.
            let snooze_metadata_json: Option<String> = row.get(17)?;
            let snooze_metadata = snooze_metadata_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<SnoozeMetadata>(s).ok());
            let (is_snoozed, snooze_reason, snoozed_at) = snooze_metadata
                .map(|m| (Some(true), Some(m.reason), Some(m.snoozed_at)))
                .unwrap_or((None, None, None));
            let priority_str: String = row.get(18)?;
            let priority = RunPriority::parse(&priority_str).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    18,
                    rusqlite::types::Type::Text,
                    format!("invalid run priority in SQLite: '{priority_str}'").into(),
                )
            })?;
            Ok(RunSummary {
                run_id: row.get(0)?,
                workspace_id: row.get(1)?,
                user_goal: row.get(2)?,
                status: row.get(3)?,
                current_step: row.get::<_, i64>(4)? as usize,
                total_steps,
                outcome_kind: row.get(8)?,
                reopen_count,
                supersedes_run_id: row.get(10)?,
                superseded_by_run_id: row.get(11)?,
                is_archived,
                archive_reason,
                archived_at,
                unarchive_reason,
                unarchived_at,
                labels,
                operator_note,
                is_pinned,
                pin_reason,
                pinned_at,
                is_snoozed,
                snooze_reason,
                snoozed_at,
                priority,
                assignee: row.get(19)?,
                due_date: row.get(20)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        };

        // Execute with the appropriate parameter combination.
        let rows = match (workspace_idx, status_idx) {
            (None, None) => stmt
                .query_map(rusqlite::params![limit as i64], mapper)
                .context("failed to query runs")?,
            (Some(_), None) => stmt
                .query_map(rusqlite::params![limit as i64, workspace_id], mapper)
                .context("failed to query runs")?,
            (None, Some(_)) => stmt
                .query_map(rusqlite::params![limit as i64, status_filter], mapper)
                .context("failed to query runs")?,
            (Some(_), Some(_)) => stmt
                .query_map(rusqlite::params![limit as i64, workspace_id, status_filter], mapper)
                .context("failed to query runs")?,
        };

        let mut summaries = Vec::new();
        for row in rows {
            let summary = row.map_err(|e| anyhow::anyhow!("failed to read run row: {e}"))?;
            // Milestone 15: post-filter by label if requested.
            if let Some(label) = label_filter
                && !summary.labels.iter().any(|l| l == label)
            {
                continue;
            }
            // Milestone 16: post-filter pinned_only.
            if pinned_only && summary.is_pinned != Some(true) {
                continue;
            }
            summaries.push(summary);
        }
        Ok(summaries)
    }

    // ----- Milestone 7: audit trail -----

    /// Append an entry to the audit trail for a run.
    pub fn append_audit_entry(
        &self,
        run_id: &str,
        event_kind: &str,
        summary: &str,
        metadata: Option<&str>,
    ) -> Result<String> {
        let entry_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        conn.execute(
            "INSERT INTO audit_trail (entry_id, run_id, event_kind, summary, metadata, occurred_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![entry_id, run_id, event_kind, summary, metadata, now],
        )
        .context("failed to append audit entry")?;
        Ok(entry_id)
    }

    /// Retrieve audit trail entries for a run, newest first.
    pub fn get_audit_entries(
        &self,
        run_id: &str,
        limit: usize,
    ) -> Result<Vec<RunHistoryEntry>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut stmt = conn
            .prepare(
                "SELECT entry_id, run_id, event_kind, summary, metadata, occurred_at
                 FROM audit_trail
                 WHERE run_id = ?1
                 ORDER BY occurred_at DESC, rowid DESC
                 LIMIT ?2",
            )
            .context("failed to prepare audit query")?;

        let rows = stmt
            .query_map(rusqlite::params![run_id, limit as i64], |row| {
                Ok(RunHistoryEntry {
                    entry_id: row.get(0)?,
                    run_id: row.get(1)?,
                    event_kind: row.get(2)?,
                    summary: row.get(3)?,
                    metadata: row.get(4)?,
                    occurred_at: row.get(5)?,
                })
            })
            .context("failed to query audit trail")?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row.map_err(|e| anyhow::anyhow!("failed to read audit row: {e}"))?);
        }
        Ok(entries)
    }

    /// Backward-compatible alias used by daemon handler tests.
    pub fn get_run_history(&self, run_id: &str, limit: usize) -> Result<Vec<RunHistoryEntry>> {
        self.get_audit_entries(run_id, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_run_state(run_id: &str, status: &str) -> RunState {
        RunState {
            run_id: run_id.into(),
            workspace_id: "/tmp/ws".into(),
            user_goal: "fix".into(),
            status: status.into(),
            plan: vec!["step 1".into(), "step 2".into()],
            current_step: 0,
            completed_steps: vec![],
            pending_steps: vec!["step 1".into(), "step 2".into()],
            last_action: None,
            last_observation: None,
            recommended_next_action: Some("inspect".into()),
            recommended_tool: Some("get_workspace_summary".into()),
            latest_diff_summary: None,
            latest_test_result: None,
            focus_paths: vec![],
            warnings: vec![],
            retryable_action: None,
            policy_profile: RunPolicy::default(),
            finalized_outcome: None,
            reopen_metadata: None,
            supersedes_run_id: None,
            superseded_by_run_id: None,
            supersession_reason: None,
            superseded_at: None,
            archive_metadata: None,
            unarchive_metadata: None,
            annotation: None,
            pin_metadata: None,
            snooze_metadata: None,
            priority: deterministic_protocol::RunPriority::Normal,
            assignee: None,
            ownership_note: None,
            due_date: None,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn roundtrip_in_memory() {
        let store = Store::open_in_memory().unwrap();

        let state = make_run_state("r1", "prepared");
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r1").unwrap().unwrap();
        assert_eq!(loaded.workspace_id, "/tmp/ws");
        assert_eq!(loaded.plan.len(), 2);
        assert_eq!(loaded.completed_steps.len(), 0);
        assert_eq!(loaded.pending_steps.len(), 2);
        assert_eq!(
            loaded.recommended_next_action.as_deref(),
            Some("inspect")
        );
    }

    #[test]
    fn roundtrip_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();

        let state = make_run_state("r1", "prepared");
        store.save_run(&state).unwrap();

        // Re-open from disk
        let store2 = Store::open(dir.path()).unwrap();
        let loaded = store2.get_run("r1").unwrap().unwrap();
        assert_eq!(loaded.user_goal, "fix");
    }

    #[test]
    fn missing_run_returns_none() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.get_run("nonexistent").unwrap().is_none());
    }

    #[test]
    fn upsert_updates_existing() {
        let store = Store::open_in_memory().unwrap();

        let mut state = make_run_state("r1", "prepared");
        store.save_run(&state).unwrap();

        state.status = "active".to_string();
        state.current_step = 1;
        state.completed_steps = vec!["step 1".into()];
        state.pending_steps = vec!["step 2".into()];
        state.last_action = Some("inspected workspace".into());
        state.last_observation = Some("found 3 files".into());
        state.latest_diff_summary = Some("2 files changed".into());
        state.latest_test_result = Some("all passed".into());
        state.warnings = vec!["watch out".into()];
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r1").unwrap().unwrap();
        assert_eq!(loaded.status, "active");
        assert_eq!(loaded.current_step, 1);
        assert_eq!(loaded.completed_steps, vec!["step 1"]);
        assert_eq!(loaded.pending_steps, vec!["step 2"]);
        assert_eq!(loaded.last_action.as_deref(), Some("inspected workspace"));
        assert_eq!(loaded.last_observation.as_deref(), Some("found 3 files"));
        assert_eq!(
            loaded.latest_diff_summary.as_deref(),
            Some("2 files changed")
        );
        assert_eq!(
            loaded.latest_test_result.as_deref(),
            Some("all passed")
        );
        assert_eq!(loaded.warnings, vec!["watch out"]);
    }

    #[test]
    fn expanded_status_values() {
        let store = Store::open_in_memory().unwrap();
        for status in ["prepared", "active", "blocked", "awaiting_approval", "done", "failed"] {
            let state = make_run_state(&format!("r_{status}"), status);
            store.save_run(&state).unwrap();
            let loaded = store.get_run(&format!("r_{status}")).unwrap().unwrap();
            assert_eq!(loaded.status, status);
        }
    }

    #[test]
    fn approval_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r1", "awaiting_approval");
        store.save_run(&state).unwrap();

        let approval = PendingApproval {
            approval_id: "a1".into(),
            run_id: "r1".into(),
            action_description: "delete file".into(),
            risk_reason: "destructive operation".into(),
            policy_rationale: "Policy: file deletion requires approval".into(),
            status: "pending".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        };
        store.save_approval(&approval).unwrap();

        let pending = store.get_pending_approvals("r1").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].approval_id, "a1");
        assert_eq!(pending[0].risk_reason, "destructive operation");
    }

    #[test]
    fn approval_resolve() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r1", "awaiting_approval");
        store.save_run(&state).unwrap();

        let approval = PendingApproval {
            approval_id: "a1".into(),
            run_id: "r1".into(),
            action_description: "delete file".into(),
            risk_reason: "destructive operation".into(),
            policy_rationale: "Policy: file deletion requires approval".into(),
            status: "pending".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        };
        store.save_approval(&approval).unwrap();

        store.resolve_approval("a1", "approve", Some("LGTM")).unwrap();
        let pending = store.get_pending_approvals("r1").unwrap();
        assert_eq!(pending.len(), 0);

        let resolved = store.get_approval("a1").unwrap().unwrap();
        assert_eq!(resolved.status, "approved");
    }

    #[test]
    fn approval_deny() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r1", "awaiting_approval");
        store.save_run(&state).unwrap();

        let approval = PendingApproval {
            approval_id: "a1".into(),
            run_id: "r1".into(),
            action_description: "rm -rf /".into(),
            risk_reason: "extremely dangerous".into(),
            policy_rationale: "Policy: destructive command".into(),
            status: "pending".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        };
        store.save_approval(&approval).unwrap();

        store.resolve_approval("a1", "deny", Some("too risky")).unwrap();
        let resolved = store.get_approval("a1").unwrap().unwrap();
        assert_eq!(resolved.status, "denied");
    }

    #[test]
    fn resolve_nonexistent_approval_fails() {
        let store = Store::open_in_memory().unwrap();
        let result = store.resolve_approval("nope", "approve", None);
        assert!(result.is_err());
    }

    /// Simulate an old Milestone 3 database schema and verify it upgrades correctly.
    #[test]
    fn migration_from_milestone3_schema() {
        use rusqlite::Connection;

        // Create a temporary directory for the test database.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("runs.db");

        // Create an old Milestone 3 schema (missing Milestone 4 columns).
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    run_id          TEXT PRIMARY KEY,
                    workspace_id    TEXT NOT NULL,
                    user_goal       TEXT NOT NULL,
                    status          TEXT NOT NULL,
                    plan            TEXT NOT NULL,
                    current_step    INTEGER NOT NULL DEFAULT 0,
                    created_at      TEXT NOT NULL,
                    updated_at      TEXT NOT NULL
                );
                CREATE TABLE approvals (
                    approval_id         TEXT PRIMARY KEY,
                    run_id              TEXT NOT NULL,
                    action_description  TEXT NOT NULL,
                    risk_reason         TEXT NOT NULL,
                    status              TEXT NOT NULL DEFAULT 'pending',
                    decision            TEXT,
                    decision_reason     TEXT,
                    created_at          TEXT NOT NULL,
                    resolved_at         TEXT,
                    FOREIGN KEY (run_id) REFERENCES runs(run_id)
                );",
            )
            .unwrap();

            // Insert a run using the old schema.
            conn.execute(
                "INSERT INTO runs (run_id, workspace_id, user_goal, status, plan, current_step, created_at, updated_at)
                 VALUES ('r_old', '/tmp/ws', 'fix bug', 'prepared', '[\"step 1\"]', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        // Now open with our Store — this should migrate the schema.
        let store = Store::open(dir.path()).unwrap();

        // Verify the old run can be loaded with new schema defaults.
        let loaded = store.get_run("r_old").unwrap().unwrap();
        assert_eq!(loaded.run_id, "r_old");
        assert_eq!(loaded.workspace_id, "/tmp/ws");
        assert_eq!(loaded.status, "prepared");
        // New columns should have default values.
        assert!(loaded.completed_steps.is_empty());
        assert!(loaded.pending_steps.is_empty());
        assert!(loaded.warnings.is_empty());
        assert!(loaded.last_action.is_none());
        assert!(loaded.last_observation.is_none());

        // Verify we can save the run with expanded state.
        let mut state = loaded;
        state.completed_steps = vec!["step 1".into()];
        state.pending_steps = vec!["step 2".into()];
        state.last_action = Some("inspected".into());
        state.warnings = vec!["caution".into()];
        store.save_run(&state).unwrap();

        // Verify roundtrip.
        let reloaded = store.get_run("r_old").unwrap().unwrap();
        assert_eq!(reloaded.completed_steps, vec!["step 1"]);
        assert_eq!(reloaded.pending_steps, vec!["step 2"]);
        assert_eq!(reloaded.last_action.as_deref(), Some("inspected"));
        assert_eq!(reloaded.warnings, vec!["caution"]);
    }

    /// Verify that opening a fresh database creates the full schema.
    #[test]
    fn fresh_database_has_full_schema() {
        let store = Store::open_in_memory().unwrap();

        let state = make_run_state("r_fresh", "active");
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_fresh").unwrap().unwrap();
        assert_eq!(loaded.run_id, "r_fresh");
        assert_eq!(loaded.status, "active");
        assert!(!loaded.plan.is_empty());
    }

    // ---- Milestone 5 policy-hardening tests ----

    /// Verify focus_paths roundtrip through SQLite.
    #[test]
    fn focus_paths_roundtrip() {
        let store = Store::open_in_memory().unwrap();

        let mut state = make_run_state("r_focus", "active");
        state.focus_paths = vec!["src/".into(), "tests/".into()];
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_focus").unwrap().unwrap();
        assert_eq!(loaded.focus_paths, vec!["src/", "tests/"]);
    }

    /// Verify policy_rationale is persisted in approval records.
    #[test]
    fn approval_policy_rationale_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r1", "awaiting_approval");
        store.save_run(&state).unwrap();

        let approval = PendingApproval {
            approval_id: "a_pol".into(),
            run_id: "r1".into(),
            action_description: "Delete src/main.rs".into(),
            risk_reason: "File deletion is destructive".into(),
            policy_rationale: "Policy: file deletion requires approval".into(),
            status: "pending".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        };
        store.save_approval(&approval).unwrap();

        let loaded = store.get_approval("a_pol").unwrap().unwrap();
        assert_eq!(loaded.policy_rationale, "Policy: file deletion requires approval");
        assert_eq!(loaded.action_description, "Delete src/main.rs");
    }

    /// Verify that multiple pending approvals can be tracked for a single run.
    #[test]
    fn multiple_pending_approvals() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r1", "awaiting_approval");
        store.save_run(&state).unwrap();

        for i in 0..3 {
            let approval = PendingApproval {
                approval_id: format!("a{i}"),
                run_id: "r1".into(),
                action_description: format!("Action {i}"),
                risk_reason: "risky".into(),
                policy_rationale: "Policy: test".into(),
                status: "pending".into(),
                created_at: "2024-01-01T00:00:00Z".into(),
            };
            store.save_approval(&approval).unwrap();
        }

        let pending = store.get_pending_approvals("r1").unwrap();
        assert_eq!(pending.len(), 3);

        // Resolve one, check remaining.
        store.resolve_approval("a0", "approve", None).unwrap();
        let pending = store.get_pending_approvals("r1").unwrap();
        assert_eq!(pending.len(), 2);

        // Deny another.
        store.resolve_approval("a1", "deny", Some("no")).unwrap();
        let pending = store.get_pending_approvals("r1").unwrap();
        assert_eq!(pending.len(), 1);
    }

    /// Verify that Milestone 3 → Milestone 5 migration adds focus_paths and policy_rationale.
    #[test]
    fn migration_from_milestone3_adds_m5_columns() {
        use rusqlite::Connection;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("runs.db");

        // Create old M3 schema.
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    run_id          TEXT PRIMARY KEY,
                    workspace_id    TEXT NOT NULL,
                    user_goal       TEXT NOT NULL,
                    status          TEXT NOT NULL,
                    plan            TEXT NOT NULL,
                    current_step    INTEGER NOT NULL DEFAULT 0,
                    created_at      TEXT NOT NULL,
                    updated_at      TEXT NOT NULL
                );
                CREATE TABLE approvals (
                    approval_id         TEXT PRIMARY KEY,
                    run_id              TEXT NOT NULL,
                    action_description  TEXT NOT NULL,
                    risk_reason         TEXT NOT NULL,
                    status              TEXT NOT NULL DEFAULT 'pending',
                    decision            TEXT,
                    decision_reason     TEXT,
                    created_at          TEXT NOT NULL,
                    resolved_at         TEXT,
                    FOREIGN KEY (run_id) REFERENCES runs(run_id)
                );",
            )
            .unwrap();

            conn.execute(
                "INSERT INTO runs (run_id, workspace_id, user_goal, status, plan, current_step, created_at, updated_at)
                 VALUES ('r_m3', '/tmp/ws', 'fix', 'active', '[\"step 1\"]', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).unwrap();
        }

        // Open with Store — should migrate.
        let store = Store::open(dir.path()).unwrap();
        let loaded = store.get_run("r_m3").unwrap().unwrap();
        // focus_paths should default to empty array.
        assert!(loaded.focus_paths.is_empty());

        // Should be able to save a run with focus_paths now.
        let mut updated = loaded;
        updated.focus_paths = vec!["src/".into()];
        store.save_run(&updated).unwrap();

        let reloaded = store.get_run("r_m3").unwrap().unwrap();
        assert_eq!(reloaded.focus_paths, vec!["src/"]);
    }

    // ---- Milestone 6: retryable action persistence tests ----

    #[test]
    fn retryable_action_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_retry", "awaiting_approval");
        state.retryable_action = Some(RetryableAction {
            kind: "patch.apply".into(),
            summary: "Edit src/main.rs".into(),
            payload: Some(r#"{"run_id":"r_retry","edits":[]}"#.into()),
            retryable_reason: "Blocked by approval policy".into(),
            is_valid: true,
            is_recommended: false,
            invalidation_reason: None,
            recommended_tool: "apply_patch".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        });
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_retry").unwrap().unwrap();
        let ra = loaded.retryable_action.as_ref().unwrap();
        assert_eq!(ra.kind, "patch.apply");
        assert_eq!(ra.summary, "Edit src/main.rs");
        assert!(ra.is_valid);
        assert!(!ra.is_recommended);
        assert_eq!(ra.recommended_tool, "apply_patch");
        assert!(ra.payload.is_some());
    }

    #[test]
    fn retryable_action_null_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_null", "active");
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_null").unwrap().unwrap();
        assert!(loaded.retryable_action.is_none());
    }

    #[test]
    fn retryable_action_update_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_upd", "awaiting_approval");
        state.retryable_action = Some(RetryableAction {
            kind: "tests.run".into(),
            summary: "Run make test".into(),
            payload: None,
            retryable_reason: "Blocked".into(),
            is_valid: true,
            is_recommended: false,
            invalidation_reason: None,
            recommended_tool: "run_tests".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        });
        store.save_run(&state).unwrap();

        // Simulate denial: invalidate the retryable action.
        if let Some(ref mut ra) = state.retryable_action {
            ra.is_valid = false;
            ra.is_recommended = false;
            ra.invalidation_reason = Some("Denied: too risky".into());
        }
        state.status = "blocked".into();
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_upd").unwrap().unwrap();
        let ra = loaded.retryable_action.as_ref().unwrap();
        assert!(!ra.is_valid);
        assert_eq!(ra.invalidation_reason.as_deref(), Some("Denied: too risky"));
    }

    #[test]
    fn retryable_action_cleared_after_success() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_clr", "active");
        state.retryable_action = Some(RetryableAction {
            kind: "patch.apply".into(),
            summary: "Edit file".into(),
            payload: None,
            retryable_reason: "Blocked".into(),
            is_valid: true,
            is_recommended: true,
            invalidation_reason: None,
            recommended_tool: "apply_patch".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        });
        store.save_run(&state).unwrap();

        // Simulate successful patch: clear retryable action.
        state.retryable_action = None;
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_clr").unwrap().unwrap();
        assert!(loaded.retryable_action.is_none());
    }

    #[test]
    fn migration_from_m5_adds_retryable_action_column() {
        use rusqlite::Connection;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("runs.db");

        // Create Milestone 5 schema (without retryable_action column).
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    run_id                   TEXT PRIMARY KEY,
                    workspace_id             TEXT NOT NULL,
                    user_goal                TEXT NOT NULL,
                    status                   TEXT NOT NULL,
                    plan                     TEXT NOT NULL,
                    current_step             INTEGER NOT NULL DEFAULT 0,
                    completed_steps          TEXT NOT NULL DEFAULT '[]',
                    pending_steps            TEXT NOT NULL DEFAULT '[]',
                    last_action              TEXT,
                    last_observation         TEXT,
                    recommended_next_action  TEXT,
                    recommended_tool         TEXT,
                    latest_diff_summary      TEXT,
                    latest_test_result       TEXT,
                    focus_paths              TEXT NOT NULL DEFAULT '[]',
                    warnings                 TEXT NOT NULL DEFAULT '[]',
                    created_at               TEXT NOT NULL,
                    updated_at               TEXT NOT NULL
                );
                CREATE TABLE approvals (
                    approval_id         TEXT PRIMARY KEY,
                    run_id              TEXT NOT NULL,
                    action_description  TEXT NOT NULL,
                    risk_reason         TEXT NOT NULL,
                    policy_rationale    TEXT NOT NULL DEFAULT '',
                    status              TEXT NOT NULL DEFAULT 'pending',
                    decision            TEXT,
                    decision_reason     TEXT,
                    created_at          TEXT NOT NULL,
                    resolved_at         TEXT,
                    FOREIGN KEY (run_id) REFERENCES runs(run_id)
                );",
            )
            .unwrap();

            conn.execute(
                "INSERT INTO runs (run_id, workspace_id, user_goal, status, plan, current_step,
                                   completed_steps, pending_steps, focus_paths, warnings,
                                   created_at, updated_at)
                 VALUES ('r_m5', '/tmp/ws', 'fix', 'active', '[\"step 1\"]', 0,
                         '[]', '[\"step 1\"]', '[]', '[]',
                         '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        // Open with Store — should migrate and add retryable_action column.
        let store = Store::open(dir.path()).unwrap();
        let loaded = store.get_run("r_m5").unwrap().unwrap();
        // retryable_action should default to None.
        assert!(loaded.retryable_action.is_none());

        // Should be able to save a run with retryable_action.
        let mut updated = loaded;
        updated.retryable_action = Some(RetryableAction {
            kind: "patch.apply".into(),
            summary: "Edit file".into(),
            payload: None,
            retryable_reason: "Blocked".into(),
            is_valid: true,
            is_recommended: false,
            invalidation_reason: None,
            recommended_tool: "apply_patch".into(),
            created_at: "2024-01-01T00:00:00Z".into(),
        });
        store.save_run(&updated).unwrap();

        let reloaded = store.get_run("r_m5").unwrap().unwrap();
        let ra = reloaded.retryable_action.as_ref().unwrap();
        assert_eq!(ra.kind, "patch.apply");
    }

    // ---- Milestone 7: run listing tests ----

    #[test]
    fn list_runs_empty() {
        let store = Store::open_in_memory().unwrap();
        let runs = store.list_runs(20, None, None, false, false, None, false, false, false).unwrap();
        assert!(runs.is_empty());
    }

    #[test]
    fn list_runs_returns_summaries() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_a", "active")).unwrap();
        store.save_run(&make_run_state("r_b", "prepared")).unwrap();
        store.save_run(&make_run_state("r_c", "done")).unwrap();

        let runs = store.list_runs(20, None, None, false, false, None, false, false, false).unwrap();
        assert_eq!(runs.len(), 3);
        // Each summary should have basic fields.
        for r in &runs {
            assert!(!r.run_id.is_empty());
            assert!(!r.workspace_id.is_empty());
            assert!(!r.user_goal.is_empty());
        }
    }

    #[test]
    fn list_runs_respects_limit() {
        let store = Store::open_in_memory().unwrap();
        for i in 0..10 {
            store
                .save_run(&make_run_state(&format!("r_{i}"), "active"))
                .unwrap();
        }
        let runs = store.list_runs(3, None, None, false, false, None, false, false, false).unwrap();
        assert_eq!(runs.len(), 3);
    }

    #[test]
    fn list_runs_filters_by_status() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_a", "active")).unwrap();
        store.save_run(&make_run_state("r_b", "active")).unwrap();
        store.save_run(&make_run_state("r_c", "done")).unwrap();

        let active = store.list_runs(20, None, Some("active"), false, false, None, false, false, false).unwrap();
        assert_eq!(active.len(), 2);

        let done = store.list_runs(20, None, Some("done"), false, false, None, false, false, false).unwrap();
        assert_eq!(done.len(), 1);
    }

    #[test]
    fn list_runs_filters_by_workspace() {
        let store = Store::open_in_memory().unwrap();
        let mut s1 = make_run_state("r_ws1a", "active");
        s1.workspace_id = "/ws/one".into();
        let mut s2 = make_run_state("r_ws1b", "active");
        s2.workspace_id = "/ws/one".into();
        let mut s3 = make_run_state("r_ws2", "active");
        s3.workspace_id = "/ws/two".into();
        store.save_run(&s1).unwrap();
        store.save_run(&s2).unwrap();
        store.save_run(&s3).unwrap();

        let ws1 = store.list_runs(20, Some("/ws/one"), None, false, false, None, false, false, false).unwrap();
        assert_eq!(ws1.len(), 2);

        let ws2 = store.list_runs(20, Some("/ws/two"), None, false, false, None, false, false, false).unwrap();
        assert_eq!(ws2.len(), 1);
    }

    #[test]
    fn list_runs_total_steps_matches_plan() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_steps", "active"); // plan has 2 steps
        store.save_run(&state).unwrap();

        let runs = store.list_runs(20, None, None, false, false, None, false, false, false).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].total_steps, 2);
    }

    // ---- Milestone 7: audit trail tests ----

    #[test]
    fn audit_entry_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_aud", "active")).unwrap();

        let id = store
            .append_audit_entry("r_aud", "run_prepared", "Run prepared: fix", None)
            .unwrap();
        assert!(!id.is_empty());

        let entries = store.get_audit_entries("r_aud", 50).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_kind, "run_prepared");
        assert_eq!(entries[0].summary, "Run prepared: fix");
        assert!(entries[0].metadata.is_none());
    }

    #[test]
    fn audit_multiple_entries_ordered_newest_first() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_ord", "active")).unwrap();

        store
            .append_audit_entry("r_ord", "run_prepared", "prepared", None)
            .unwrap();
        store
            .append_audit_entry("r_ord", "refresh_performed", "refreshed", None)
            .unwrap();
        store
            .append_audit_entry("r_ord", "patch_applied", "patched", None)
            .unwrap();

        let entries = store.get_audit_entries("r_ord", 50).unwrap();
        assert_eq!(entries.len(), 3);
        // Newest first — patch_applied was inserted last.
        assert_eq!(entries[0].event_kind, "patch_applied");
    }

    #[test]
    fn audit_entry_with_metadata() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_meta", "active")).unwrap();

        store
            .append_audit_entry(
                "r_meta",
                "patch_applied",
                "Patch applied: 2 file(s) changed",
                Some(r#"{"files":["src/main.rs","src/lib.rs"]}"#),
            )
            .unwrap();

        let entries = store.get_audit_entries("r_meta", 50).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].metadata.as_deref(),
            Some(r#"{"files":["src/main.rs","src/lib.rs"]}"#)
        );
    }

    #[test]
    fn audit_limit_respected() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_lim", "active")).unwrap();

        for i in 0..10 {
            store
                .append_audit_entry("r_lim", "refresh_performed", &format!("refresh {i}"), None)
                .unwrap();
        }

        let entries = store.get_audit_entries("r_lim", 3).unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn audit_isolated_by_run_id() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_x", "active")).unwrap();
        store.save_run(&make_run_state("r_y", "active")).unwrap();

        store
            .append_audit_entry("r_x", "run_prepared", "prepared x", None)
            .unwrap();
        store
            .append_audit_entry("r_y", "run_prepared", "prepared y", None)
            .unwrap();

        let x_entries = store.get_audit_entries("r_x", 50).unwrap();
        assert_eq!(x_entries.len(), 1);
        assert_eq!(x_entries[0].run_id, "r_x");

        let y_entries = store.get_audit_entries("r_y", 50).unwrap();
        assert_eq!(y_entries.len(), 1);
        assert_eq!(y_entries[0].run_id, "r_y");
    }

    #[test]
    fn migration_from_m6_adds_audit_trail_table() {
        use rusqlite::Connection;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("runs.db");

        // Create Milestone 6 schema (without audit_trail table).
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    run_id                   TEXT PRIMARY KEY,
                    workspace_id             TEXT NOT NULL,
                    user_goal                TEXT NOT NULL,
                    status                   TEXT NOT NULL,
                    plan                     TEXT NOT NULL,
                    current_step             INTEGER NOT NULL DEFAULT 0,
                    completed_steps          TEXT NOT NULL DEFAULT '[]',
                    pending_steps            TEXT NOT NULL DEFAULT '[]',
                    last_action              TEXT,
                    last_observation         TEXT,
                    recommended_next_action  TEXT,
                    recommended_tool         TEXT,
                    latest_diff_summary      TEXT,
                    latest_test_result       TEXT,
                    focus_paths              TEXT NOT NULL DEFAULT '[]',
                    warnings                 TEXT NOT NULL DEFAULT '[]',
                    retryable_action         TEXT,
                    created_at               TEXT NOT NULL,
                    updated_at               TEXT NOT NULL
                );
                CREATE TABLE approvals (
                    approval_id         TEXT PRIMARY KEY,
                    run_id              TEXT NOT NULL,
                    action_description  TEXT NOT NULL,
                    risk_reason         TEXT NOT NULL,
                    policy_rationale    TEXT NOT NULL DEFAULT '',
                    status              TEXT NOT NULL DEFAULT 'pending',
                    decision            TEXT,
                    decision_reason     TEXT,
                    created_at          TEXT NOT NULL,
                    resolved_at         TEXT,
                    FOREIGN KEY (run_id) REFERENCES runs(run_id)
                );",
            )
            .unwrap();

            conn.execute(
                "INSERT INTO runs (run_id, workspace_id, user_goal, status, plan, current_step,
                                   completed_steps, pending_steps, focus_paths, warnings,
                                   created_at, updated_at)
                 VALUES ('r_m6', '/tmp/ws', 'fix', 'active', '[\"step 1\"]', 0,
                         '[]', '[\"step 1\"]', '[]', '[]',
                         '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        // Open with Store — should migrate and add audit_trail table.
        let store = Store::open(dir.path()).unwrap();
        let loaded = store.get_run("r_m6").unwrap().unwrap();
        assert_eq!(loaded.run_id, "r_m6");

        // Should be able to write audit entries now.
        store
            .append_audit_entry("r_m6", "run_prepared", "migrated run", None)
            .unwrap();
        let entries = store.get_audit_entries("r_m6", 10).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn fresh_database_has_audit_trail_table() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_fresh", "active")).unwrap();
        // Just verify we can write without error.
        store
            .append_audit_entry("r_fresh", "run_prepared", "fresh db test", None)
            .unwrap();
        let entries = store.get_audit_entries("r_fresh", 10).unwrap();
        assert_eq!(entries.len(), 1);
    }

    // ---- Milestone 8: policy_profile persistence tests ----

    #[test]
    fn policy_profile_default_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_pol_def", "active"); // uses RunPolicy::default()
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_pol_def").unwrap().unwrap();
        let defaults = RunPolicy::default();
        assert_eq!(loaded.policy_profile.patch_edit_threshold, defaults.patch_edit_threshold);
        assert_eq!(loaded.policy_profile.delete_requires_approval, defaults.delete_requires_approval);
        assert_eq!(loaded.policy_profile.sensitive_path_requires_approval, defaults.sensitive_path_requires_approval);
        assert!(loaded.policy_profile.extra_safe_make_targets.is_empty());
    }

    #[test]
    fn policy_profile_custom_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_pol_cust", "active");
        state.policy_profile = RunPolicy {
            patch_edit_threshold: 20,
            delete_requires_approval: false,
            sensitive_path_requires_approval: true,
            outside_focus_requires_approval: false,
            extra_safe_make_targets: vec!["deploy".to_string()],
            focus_paths: vec!["src/".to_string()],
        };
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_pol_cust").unwrap().unwrap();
        assert_eq!(loaded.policy_profile.patch_edit_threshold, 20);
        assert!(!loaded.policy_profile.delete_requires_approval);
        assert!(!loaded.policy_profile.outside_focus_requires_approval);
        assert_eq!(loaded.policy_profile.extra_safe_make_targets, vec!["deploy"]);
        assert_eq!(loaded.policy_profile.focus_paths, vec!["src/"]);
    }

    /// Verify that migration from M7 (no policy_profile column) produces defaults.
    #[test]
    fn migration_from_m7_adds_policy_profile_column() {
        use rusqlite::Connection;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("runs.db");

        // Create Milestone 7 schema (without policy_profile column).
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    run_id                   TEXT PRIMARY KEY,
                    workspace_id             TEXT NOT NULL,
                    user_goal                TEXT NOT NULL,
                    status                   TEXT NOT NULL,
                    plan                     TEXT NOT NULL,
                    current_step             INTEGER NOT NULL DEFAULT 0,
                    completed_steps          TEXT NOT NULL DEFAULT '[]',
                    pending_steps            TEXT NOT NULL DEFAULT '[]',
                    last_action              TEXT,
                    last_observation         TEXT,
                    recommended_next_action  TEXT,
                    recommended_tool         TEXT,
                    latest_diff_summary      TEXT,
                    latest_test_result       TEXT,
                    focus_paths              TEXT NOT NULL DEFAULT '[]',
                    warnings                 TEXT NOT NULL DEFAULT '[]',
                    retryable_action         TEXT,
                    created_at               TEXT NOT NULL,
                    updated_at               TEXT NOT NULL
                );
                CREATE TABLE approvals (
                    approval_id         TEXT PRIMARY KEY,
                    run_id              TEXT NOT NULL,
                    action_description  TEXT NOT NULL,
                    risk_reason         TEXT NOT NULL,
                    policy_rationale    TEXT NOT NULL DEFAULT '',
                    status              TEXT NOT NULL DEFAULT 'pending',
                    decision            TEXT,
                    decision_reason     TEXT,
                    created_at          TEXT NOT NULL,
                    resolved_at         TEXT,
                    FOREIGN KEY (run_id) REFERENCES runs(run_id)
                );
                CREATE TABLE IF NOT EXISTS audit_trail (
                    entry_id    TEXT PRIMARY KEY,
                    run_id      TEXT NOT NULL,
                    event_kind  TEXT NOT NULL,
                    summary     TEXT NOT NULL,
                    metadata    TEXT,
                    occurred_at TEXT NOT NULL,
                    FOREIGN KEY (run_id) REFERENCES runs(run_id)
                );",
            )
            .unwrap();

            conn.execute(
                "INSERT INTO runs (run_id, workspace_id, user_goal, status, plan, current_step,
                                   completed_steps, pending_steps, focus_paths, warnings,
                                   created_at, updated_at)
                 VALUES ('r_m7', '/tmp/ws', 'fix', 'active', '[\"step 1\"]', 0,
                         '[]', '[\"step 1\"]', '[]', '[]',
                         '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        // Open with Store — should migrate and add policy_profile column.
        let store = Store::open(dir.path()).unwrap();
        let loaded = store.get_run("r_m7").unwrap().unwrap();
        // policy_profile should default to RunPolicy::default().
        let defaults = RunPolicy::default();
        assert_eq!(loaded.policy_profile.patch_edit_threshold, defaults.patch_edit_threshold);
        assert_eq!(loaded.policy_profile.delete_requires_approval, defaults.delete_requires_approval);

        // Should be able to save with a custom policy now.
        let mut updated = loaded;
        updated.policy_profile.patch_edit_threshold = 15;
        store.save_run(&updated).unwrap();

        let reloaded = store.get_run("r_m7").unwrap().unwrap();
        assert_eq!(reloaded.policy_profile.patch_edit_threshold, 15);
    }

    // ---- Milestone 10: finalized_outcome persistence tests ----

    #[test]
    fn finalized_outcome_null_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_fo_null", "active");
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_fo_null").unwrap().unwrap();
        assert!(loaded.finalized_outcome.is_none());
    }

    #[test]
    fn finalized_outcome_completed_roundtrip() {
        use deterministic_protocol::RunOutcome;

        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_fo_c", "finalized:completed");
        state.finalized_outcome = Some(RunOutcome {
            outcome_kind: "completed".into(),
            summary: "All done".into(),
            reason: None,
            finalized_at: "2024-06-01T12:00:00Z".into(),
        });
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_fo_c").unwrap().unwrap();
        let outcome = loaded.finalized_outcome.as_ref().unwrap();
        assert_eq!(outcome.outcome_kind, "completed");
        assert_eq!(outcome.summary, "All done");
        assert!(outcome.reason.is_none());
        assert_eq!(outcome.finalized_at, "2024-06-01T12:00:00Z");
        assert_eq!(loaded.status, "finalized:completed");
    }

    #[test]
    fn finalized_outcome_failed_with_reason_roundtrip() {
        use deterministic_protocol::RunOutcome;

        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_fo_f", "finalized:failed");
        state.finalized_outcome = Some(RunOutcome {
            outcome_kind: "failed".into(),
            summary: "Build broke".into(),
            reason: Some("compiler error in step 2".into()),
            finalized_at: "2024-06-02T08:00:00Z".into(),
        });
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_fo_f").unwrap().unwrap();
        let outcome = loaded.finalized_outcome.as_ref().unwrap();
        assert_eq!(outcome.outcome_kind, "failed");
        assert_eq!(outcome.reason.as_deref(), Some("compiler error in step 2"));
    }

    #[test]
    fn list_runs_includes_outcome_kind() {
        use deterministic_protocol::RunOutcome;

        let store = Store::open_in_memory().unwrap();
        // Active run (no outcome).
        store.save_run(&make_run_state("r_lk_a", "active")).unwrap();
        // Finalized run.
        let mut finalized = make_run_state("r_lk_f", "finalized:completed");
        finalized.finalized_outcome = Some(RunOutcome {
            outcome_kind: "completed".into(),
            summary: "done".into(),
            reason: None,
            finalized_at: "2024-06-01T12:00:00Z".into(),
        });
        store.save_run(&finalized).unwrap();

        let runs = store.list_runs(20, None, None, false, false, None, false, false, false).unwrap();
        assert_eq!(runs.len(), 2);

        let active = runs.iter().find(|r| r.run_id == "r_lk_a").unwrap();
        assert!(active.outcome_kind.is_none());

        let closed = runs.iter().find(|r| r.run_id == "r_lk_f").unwrap();
        assert_eq!(closed.outcome_kind.as_deref(), Some("completed"));
    }

    /// Verify that migration from M9 (no outcome columns) succeeds and defaults outcome to None.
    #[test]
    fn migration_from_m9_adds_outcome_columns() {
        use rusqlite::Connection;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("runs.db");

        // Create Milestone 9 schema (without outcome_kind / finalized_outcome columns).
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    run_id                   TEXT PRIMARY KEY,
                    workspace_id             TEXT NOT NULL,
                    user_goal                TEXT NOT NULL,
                    status                   TEXT NOT NULL,
                    plan                     TEXT NOT NULL,
                    current_step             INTEGER NOT NULL DEFAULT 0,
                    completed_steps          TEXT NOT NULL DEFAULT '[]',
                    pending_steps            TEXT NOT NULL DEFAULT '[]',
                    last_action              TEXT,
                    last_observation         TEXT,
                    recommended_next_action  TEXT,
                    recommended_tool         TEXT,
                    latest_diff_summary      TEXT,
                    latest_test_result       TEXT,
                    focus_paths              TEXT NOT NULL DEFAULT '[]',
                    warnings                 TEXT NOT NULL DEFAULT '[]',
                    retryable_action         TEXT,
                    policy_profile           TEXT NOT NULL DEFAULT '{}',
                    created_at               TEXT NOT NULL,
                    updated_at               TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS approvals (
                    approval_id         TEXT PRIMARY KEY,
                    run_id              TEXT NOT NULL,
                    action_description  TEXT NOT NULL,
                    risk_reason         TEXT NOT NULL,
                    policy_rationale    TEXT NOT NULL DEFAULT '',
                    status              TEXT NOT NULL DEFAULT 'pending',
                    decision            TEXT,
                    decision_reason     TEXT,
                    created_at          TEXT NOT NULL,
                    resolved_at         TEXT,
                    FOREIGN KEY (run_id) REFERENCES runs(run_id)
                );
                CREATE TABLE IF NOT EXISTS audit_trail (
                    entry_id    TEXT PRIMARY KEY,
                    run_id      TEXT NOT NULL,
                    event_kind  TEXT NOT NULL,
                    summary     TEXT NOT NULL,
                    metadata    TEXT,
                    occurred_at TEXT NOT NULL,
                    FOREIGN KEY (run_id) REFERENCES runs(run_id)
                );",
            )
            .unwrap();

            conn.execute(
                "INSERT INTO runs (run_id, workspace_id, user_goal, status, plan, current_step,
                                   completed_steps, pending_steps, focus_paths, warnings,
                                   policy_profile, created_at, updated_at)
                 VALUES ('r_m9', '/tmp/ws', 'fix', 'active', '[\"step 1\"]', 0,
                         '[]', '[\"step 1\"]', '[]', '[]',
                         '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        // Open with Store — should migrate and add outcome columns.
        let store = Store::open(dir.path()).unwrap();
        let loaded = store.get_run("r_m9").unwrap().unwrap();
        // finalized_outcome should default to None.
        assert!(loaded.finalized_outcome.is_none());

        // Should be able to finalize now.
        let mut updated = loaded;
        updated.status = "finalized:abandoned".into();
        updated.finalized_outcome = Some(deterministic_protocol::RunOutcome {
            outcome_kind: "abandoned".into(),
            summary: "goal changed".into(),
            reason: None,
            finalized_at: "2024-06-01T12:00:00Z".into(),
        });
        store.save_run(&updated).unwrap();

        let reloaded = store.get_run("r_m9").unwrap().unwrap();
        let outcome = reloaded.finalized_outcome.as_ref().unwrap();
        assert_eq!(outcome.outcome_kind, "abandoned");
    }

    // -----------------------------------------------------------------------
    // Milestone 11: reopen_metadata persistence
    // -----------------------------------------------------------------------

    #[test]
    fn reopen_metadata_null_for_fresh_run() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ro_1", "active");
        store.save_run(&state).unwrap();
        let loaded = store.get_run("r_ro_1").unwrap().unwrap();
        assert!(loaded.reopen_metadata.is_none());
    }

    #[test]
    fn reopen_metadata_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_ro_2", "finalized:completed");
        state.reopen_metadata = Some(deterministic_protocol::ReopenMetadata {
            reason: "found another issue".into(),
            reopened_at: "2024-07-01T10:00:00Z".into(),
            reopened_from_outcome_kind: "completed".into(),
            reopen_count: 1,
        });
        state.status = "active".into();
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_ro_2").unwrap().unwrap();
        let meta = loaded.reopen_metadata.as_ref().unwrap();
        assert_eq!(meta.reason, "found another issue");
        assert_eq!(meta.reopened_at, "2024-07-01T10:00:00Z");
        assert_eq!(meta.reopened_from_outcome_kind, "completed");
        assert_eq!(meta.reopen_count, 1);
    }

    #[test]
    fn reopen_metadata_increments_reopen_count() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_ro_3", "active");

        // First reopen.
        state.reopen_metadata = Some(deterministic_protocol::ReopenMetadata {
            reason: "first reopen".into(),
            reopened_at: "2024-07-01T10:00:00Z".into(),
            reopened_from_outcome_kind: "failed".into(),
            reopen_count: 1,
        });
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_ro_3").unwrap().unwrap();
        assert_eq!(loaded.reopen_metadata.as_ref().unwrap().reopen_count, 1);

        // Second reopen: overwrite with updated metadata.
        let mut state2 = loaded;
        state2.reopen_metadata = Some(deterministic_protocol::ReopenMetadata {
            reason: "second reopen".into(),
            reopened_at: "2024-08-01T10:00:00Z".into(),
            reopened_from_outcome_kind: "completed".into(),
            reopen_count: 2,
        });
        store.save_run(&state2).unwrap();

        let loaded2 = store.get_run("r_ro_3").unwrap().unwrap();
        let meta = loaded2.reopen_metadata.as_ref().unwrap();
        assert_eq!(meta.reopen_count, 2);
        assert_eq!(meta.reopened_from_outcome_kind, "completed");
    }

    #[test]
    fn reopen_metadata_migration_safe_for_old_rows() {
        // Simulate an older database without the reopen_metadata column:
        // open a store, add a run, then re-query.  The migration should add
        // the column with NULL default, so get_run should succeed and return
        // reopen_metadata = None.
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ro_mig", "active");
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_ro_mig").unwrap().unwrap();
        // Migration provided a NULL default — should not be present.
        assert!(loaded.reopen_metadata.is_none());
    }

    #[test]
    fn list_runs_includes_reopen_count() {
        let store = Store::open_in_memory().unwrap();

        let state_no_reopen = make_run_state("r_list_1", "active");
        store.save_run(&state_no_reopen).unwrap();

        let mut state_reopened = make_run_state("r_list_2", "active");
        state_reopened.reopen_metadata = Some(deterministic_protocol::ReopenMetadata {
            reason: "reopened".into(),
            reopened_at: "2024-07-01T10:00:00Z".into(),
            reopened_from_outcome_kind: "failed".into(),
            reopen_count: 3,
        });
        store.save_run(&state_reopened).unwrap();

        let summaries = store.list_runs(10, None, None, false, false, None, false, false, false).unwrap();
        let s1 = summaries.iter().find(|s| s.run_id == "r_list_1").unwrap();
        let s2 = summaries.iter().find(|s| s.run_id == "r_list_2").unwrap();
        assert!(s1.reopen_count.is_none());
        assert_eq!(s2.reopen_count, Some(3));
    }

    // ---- Milestone 12: supersession lineage persistence tests ----

    /// Verify supersession lineage fields roundtrip through SQLite.
    #[test]
    fn supersession_lineage_roundtrip() {
        let store = Store::open_in_memory().unwrap();

        // Original run: superseded by "r_successor"
        let mut original = make_run_state("r_orig", "finalized:completed");
        original.superseded_by_run_id = Some("r_successor".into());
        original.supersession_reason = Some("scope changed".into());
        original.superseded_at = Some("2024-08-01T12:00:00Z".into());
        store.save_run(&original).unwrap();

        // Successor run: supersedes "r_orig"
        let mut successor = make_run_state("r_successor", "prepared");
        successor.supersedes_run_id = Some("r_orig".into());
        successor.supersession_reason = Some("scope changed".into());
        successor.superseded_at = Some("2024-08-01T12:00:00Z".into());
        store.save_run(&successor).unwrap();

        // Load and verify original.
        let loaded_orig = store.get_run("r_orig").unwrap().unwrap();
        assert_eq!(loaded_orig.superseded_by_run_id.as_deref(), Some("r_successor"));
        assert_eq!(loaded_orig.supersession_reason.as_deref(), Some("scope changed"));
        assert_eq!(loaded_orig.superseded_at.as_deref(), Some("2024-08-01T12:00:00Z"));
        assert!(loaded_orig.supersedes_run_id.is_none());

        // Load and verify successor.
        let loaded_succ = store.get_run("r_successor").unwrap().unwrap();
        assert_eq!(loaded_succ.supersedes_run_id.as_deref(), Some("r_orig"));
        assert_eq!(loaded_succ.supersession_reason.as_deref(), Some("scope changed"));
        assert_eq!(loaded_succ.superseded_at.as_deref(), Some("2024-08-01T12:00:00Z"));
        assert!(loaded_succ.superseded_by_run_id.is_none());
    }

    /// Verify that supersedes_run_id and superseded_by_run_id are None by default.
    #[test]
    fn supersession_fields_default_to_none() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_no_super", "active");
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_no_super").unwrap().unwrap();
        assert!(loaded.supersedes_run_id.is_none());
        assert!(loaded.superseded_by_run_id.is_none());
        assert!(loaded.supersession_reason.is_none());
        assert!(loaded.superseded_at.is_none());
    }

    /// Verify list_runs includes supersession lineage fields.
    #[test]
    fn list_runs_includes_supersession_lineage() {
        let store = Store::open_in_memory().unwrap();

        let state_plain = make_run_state("r_plain", "active");
        store.save_run(&state_plain).unwrap();

        let mut state_superseded = make_run_state("r_superseded", "finalized:completed");
        state_superseded.superseded_by_run_id = Some("r_new".into());
        store.save_run(&state_superseded).unwrap();

        let mut state_successor = make_run_state("r_new", "prepared");
        state_successor.supersedes_run_id = Some("r_superseded".into());
        store.save_run(&state_successor).unwrap();

        let summaries = store.list_runs(10, None, None, false, false, None, false, false, false).unwrap();
        let plain = summaries.iter().find(|s| s.run_id == "r_plain").unwrap();
        let superseded = summaries.iter().find(|s| s.run_id == "r_superseded").unwrap();
        let successor = summaries.iter().find(|s| s.run_id == "r_new").unwrap();

        assert!(plain.supersedes_run_id.is_none());
        assert!(plain.superseded_by_run_id.is_none());

        assert_eq!(superseded.superseded_by_run_id.as_deref(), Some("r_new"));
        assert!(superseded.supersedes_run_id.is_none());

        assert_eq!(successor.supersedes_run_id.as_deref(), Some("r_superseded"));
        assert!(successor.superseded_by_run_id.is_none());
    }

    /// Verify that migration from an older schema (pre-M12) correctly handles
    /// the new supersession columns being absent (NULL default).
    #[test]
    fn migration_m12_columns_default_to_null() {
        use rusqlite::Connection;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("runs.db");

        // Simulate a Milestone 11 schema (no M12 columns).
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    run_id               TEXT PRIMARY KEY,
                    workspace_id         TEXT NOT NULL,
                    user_goal            TEXT NOT NULL,
                    status               TEXT NOT NULL,
                    plan                 TEXT NOT NULL DEFAULT '[]',
                    current_step         INTEGER NOT NULL DEFAULT 0,
                    completed_steps      TEXT NOT NULL DEFAULT '[]',
                    pending_steps        TEXT NOT NULL DEFAULT '[]',
                    last_action          TEXT,
                    last_observation     TEXT,
                    recommended_next_action TEXT,
                    recommended_tool     TEXT,
                    latest_diff_summary  TEXT,
                    latest_test_result   TEXT,
                    focus_paths          TEXT NOT NULL DEFAULT '[]',
                    warnings             TEXT NOT NULL DEFAULT '[]',
                    retryable_action     TEXT,
                    policy_profile       TEXT NOT NULL DEFAULT '{}',
                    outcome_kind         TEXT,
                    finalized_outcome    TEXT,
                    reopen_metadata      TEXT,
                    created_at           TEXT NOT NULL,
                    updated_at           TEXT NOT NULL
                );",
            ).unwrap();
            conn.execute(
                "INSERT INTO runs (run_id, workspace_id, user_goal, status, plan, created_at, updated_at)
                 VALUES ('r_m11', '/tmp/ws', 'old goal', 'active', '[]', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).unwrap();
        }

        // Open with migration — should add M12 columns.
        let store = Store::open(dir.path()).unwrap();
        let loaded = store.get_run("r_m11").unwrap().unwrap();

        // M12 columns must have NULL defaults.
        assert!(loaded.supersedes_run_id.is_none());
        assert!(loaded.superseded_by_run_id.is_none());
        assert!(loaded.supersession_reason.is_none());
        assert!(loaded.superseded_at.is_none());
        // M13 columns must also have safe defaults.
        assert!(loaded.archive_metadata.is_none());
    }

    // ---- Milestone 13: archive metadata persistence tests ----

    /// Verify that archive_metadata is correctly persisted and retrieved.
    #[test]
    fn archive_metadata_roundtrip() {
        use deterministic_protocol::ArchiveMetadata;

        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_arch", "finalized:completed");
        state.archive_metadata = Some(ArchiveMetadata {
            reason: "Archiving completed run".into(),
            archived_at: "2024-09-01T10:00:00Z".into(),
        });
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_arch").unwrap().unwrap();
        let meta = loaded.archive_metadata.expect("archive_metadata must be present");
        assert_eq!(meta.reason, "Archiving completed run");
        assert_eq!(meta.archived_at, "2024-09-01T10:00:00Z");
    }

    /// Verify that archive_metadata defaults to None for unarchived runs.
    #[test]
    fn archive_metadata_defaults_to_none() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_noarch", "active");
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_noarch").unwrap().unwrap();
        assert!(loaded.archive_metadata.is_none());
    }

    /// Verify list_runs excludes archived runs by default.
    #[test]
    fn list_runs_excludes_archived_by_default() {
        use deterministic_protocol::ArchiveMetadata;

        let store = Store::open_in_memory().unwrap();

        let active = make_run_state("r_active", "active");
        store.save_run(&active).unwrap();

        let mut archived = make_run_state("r_archived", "finalized:completed");
        archived.archive_metadata = Some(ArchiveMetadata {
            reason: "hygiene".into(),
            archived_at: "2024-09-01T10:00:00Z".into(),
        });
        store.save_run(&archived).unwrap();

        // Default: exclude archived.
        let runs = store.list_runs(20, None, None, false, false, None, false, false, false).unwrap();
        assert!(
            runs.iter().any(|r| r.run_id == "r_active"),
            "active run must be included"
        );
        assert!(
            !runs.iter().any(|r| r.run_id == "r_archived"),
            "archived run must be excluded by default"
        );
    }

    /// Verify list_runs includes archived runs when include_archived=true.
    #[test]
    fn list_runs_include_archived_shows_all() {
        use deterministic_protocol::ArchiveMetadata;

        let store = Store::open_in_memory().unwrap();

        let active = make_run_state("r_active2", "active");
        store.save_run(&active).unwrap();

        let mut archived = make_run_state("r_archived2", "finalized:completed");
        archived.archive_metadata = Some(ArchiveMetadata {
            reason: "include test".into(),
            archived_at: "2024-09-01T10:00:00Z".into(),
        });
        store.save_run(&archived).unwrap();

        let runs = store.list_runs(20, None, None, true, false, None, false, false, false).unwrap();
        assert!(
            runs.iter().any(|r| r.run_id == "r_active2"),
            "active run must be included"
        );
        assert!(
            runs.iter().any(|r| r.run_id == "r_archived2"),
            "archived run must be included when include_archived=true"
        );
    }

    /// Verify list_runs returns only archived runs when archived_only=true.
    #[test]
    fn list_runs_archived_only() {
        use deterministic_protocol::ArchiveMetadata;

        let store = Store::open_in_memory().unwrap();

        let active = make_run_state("r_active3", "active");
        store.save_run(&active).unwrap();

        let mut archived = make_run_state("r_archived3", "finalized:completed");
        archived.archive_metadata = Some(ArchiveMetadata {
            reason: "only test".into(),
            archived_at: "2024-09-01T10:00:00Z".into(),
        });
        store.save_run(&archived).unwrap();

        let runs = store.list_runs(20, None, None, false, true, None, false, false, false).unwrap();
        assert!(
            !runs.iter().any(|r| r.run_id == "r_active3"),
            "active run must NOT be included"
        );
        assert!(
            runs.iter().any(|r| r.run_id == "r_archived3"),
            "archived run must be included when archived_only=true"
        );
    }

    /// Verify list_runs RunSummary carries archive fields when present.
    #[test]
    fn list_runs_summary_carries_archive_fields() {
        use deterministic_protocol::ArchiveMetadata;

        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_arch_sum", "finalized:completed");
        state.archive_metadata = Some(ArchiveMetadata {
            reason: "summary field test".into(),
            archived_at: "2024-09-02T12:00:00Z".into(),
        });
        store.save_run(&state).unwrap();

        let runs = store.list_runs(20, None, None, true, false, None, false, false, false).unwrap();
        let summary = runs.iter().find(|r| r.run_id == "r_arch_sum").unwrap();
        assert_eq!(summary.is_archived, Some(true));
        assert_eq!(summary.archive_reason.as_deref(), Some("summary field test"));
        assert_eq!(summary.archived_at.as_deref(), Some("2024-09-02T12:00:00Z"));
    }

    /// Verify that migration from an older schema (pre-M13) correctly handles
    /// the new archive columns being absent (safe defaults).
    #[test]
    fn migration_m13_columns_default_safely() {
        use rusqlite::Connection;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("runs.db");

        // Simulate a Milestone 12 schema (no M13 columns).
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    run_id               TEXT PRIMARY KEY,
                    workspace_id         TEXT NOT NULL,
                    user_goal            TEXT NOT NULL,
                    status               TEXT NOT NULL,
                    plan                 TEXT NOT NULL DEFAULT '[]',
                    current_step         INTEGER NOT NULL DEFAULT 0,
                    completed_steps      TEXT NOT NULL DEFAULT '[]',
                    pending_steps        TEXT NOT NULL DEFAULT '[]',
                    last_action          TEXT,
                    last_observation     TEXT,
                    recommended_next_action TEXT,
                    recommended_tool     TEXT,
                    latest_diff_summary  TEXT,
                    latest_test_result   TEXT,
                    focus_paths          TEXT NOT NULL DEFAULT '[]',
                    warnings             TEXT NOT NULL DEFAULT '[]',
                    retryable_action     TEXT,
                    policy_profile       TEXT NOT NULL DEFAULT '{}',
                    outcome_kind         TEXT,
                    finalized_outcome    TEXT,
                    reopen_metadata      TEXT,
                    supersedes_run_id    TEXT,
                    superseded_by_run_id TEXT,
                    supersession_reason  TEXT,
                    superseded_at        TEXT,
                    created_at           TEXT NOT NULL,
                    updated_at           TEXT NOT NULL
                );",
            ).unwrap();
            conn.execute(
                "INSERT INTO runs (run_id, workspace_id, user_goal, status, plan, created_at, updated_at)
                 VALUES ('r_m12', '/tmp/ws', 'old goal', 'finalized:completed', '[]', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).unwrap();
        }

        // Open with migration — should add M13 columns.
        let store = Store::open(dir.path()).unwrap();
        let loaded = store.get_run("r_m12").unwrap().unwrap();

        // M13 archive_metadata must default to None.
        assert!(loaded.archive_metadata.is_none());
    }

    // ---- Milestone 14: unarchive metadata persistence tests ----

    /// Verify that unarchive_metadata is correctly persisted and retrieved.
    #[test]
    fn unarchive_metadata_roundtrip() {
        use deterministic_protocol::{ArchiveMetadata, UnarchiveMetadata};

        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_unarch_rt", "finalized:completed");
        state.archive_metadata = Some(ArchiveMetadata {
            reason: "archived".into(),
            archived_at: "2024-09-01T10:00:00Z".into(),
        });
        state.unarchive_metadata = Some(UnarchiveMetadata {
            reason: "restoring".into(),
            unarchived_at: "2024-09-02T10:00:00Z".into(),
        });
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_unarch_rt").unwrap().unwrap();
        let meta = loaded.unarchive_metadata.expect("unarchive_metadata must be present");
        assert_eq!(meta.reason, "restoring");
        assert_eq!(meta.unarchived_at, "2024-09-02T10:00:00Z");
        // Archive metadata must remain intact.
        assert!(loaded.archive_metadata.is_some());
    }

    /// Verify that unarchive_metadata defaults to None for runs never unarchived.
    #[test]
    fn unarchive_metadata_defaults_to_none() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_no_unarch", "active");
        store.save_run(&state).unwrap();
        let loaded = store.get_run("r_no_unarch").unwrap().unwrap();
        assert!(loaded.unarchive_metadata.is_none());
    }

    /// Verify that is_archived = 0 after a run is unarchived (restored run returns to default list).
    #[test]
    fn list_runs_restored_run_returns_to_default_list() {
        use deterministic_protocol::{ArchiveMetadata, UnarchiveMetadata};

        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_restored", "finalized:completed");
        state.archive_metadata = Some(ArchiveMetadata {
            reason: "archived".into(),
            archived_at: "2024-09-01T10:00:00Z".into(),
        });
        state.unarchive_metadata = Some(UnarchiveMetadata {
            reason: "restoring".into(),
            unarchived_at: "2024-09-02T10:00:00Z".into(),
        });
        store.save_run(&state).unwrap();

        // Default list should include the restored run (is_archived=0 since unarchive_metadata is set).
        let runs = store.list_runs(20, None, None, false, false, None, false, false, false).unwrap();
        assert!(
            runs.iter().any(|r| r.run_id == "r_restored"),
            "restored run must appear in default list"
        );

        // archived_only=true must NOT include the restored run.
        let runs_ao = store.list_runs(20, None, None, false, true, None, false, false, false).unwrap();
        assert!(
            !runs_ao.iter().any(|r| r.run_id == "r_restored"),
            "restored run must NOT appear when archived_only=true"
        );
    }

    /// Verify list_runs RunSummary carries unarchive fields when present.
    #[test]
    fn list_runs_summary_carries_unarchive_fields() {
        use deterministic_protocol::{ArchiveMetadata, UnarchiveMetadata};

        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_unarch_sum", "finalized:completed");
        state.archive_metadata = Some(ArchiveMetadata {
            reason: "summary test archive".into(),
            archived_at: "2024-09-01T10:00:00Z".into(),
        });
        state.unarchive_metadata = Some(UnarchiveMetadata {
            reason: "summary test unarchive".into(),
            unarchived_at: "2024-09-03T12:00:00Z".into(),
        });
        store.save_run(&state).unwrap();

        // include_archived=true to include runs with archive_metadata (even if unarchived).
        let runs = store.list_runs(20, None, None, true, false, None, false, false, false).unwrap();
        let summary = runs.iter().find(|r| r.run_id == "r_unarch_sum").unwrap();
        // is_archived must be None/false since the run is unarchived.
        assert_eq!(summary.is_archived, None, "is_archived must be None for unarchived run");
        assert_eq!(summary.unarchive_reason.as_deref(), Some("summary test unarchive"));
        assert_eq!(summary.unarchived_at.as_deref(), Some("2024-09-03T12:00:00Z"));
    }

    /// Verify that migration from M13 schema (no M14 unarchive_metadata column) works safely.
    #[test]
    fn migration_m14_unarchive_metadata_defaults_safely() {
        use rusqlite::Connection;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("runs.db");

        // Simulate a Milestone 13 schema (no M14 unarchive_metadata column).
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    run_id               TEXT PRIMARY KEY,
                    workspace_id         TEXT NOT NULL,
                    user_goal            TEXT NOT NULL,
                    status               TEXT NOT NULL,
                    plan                 TEXT NOT NULL DEFAULT '[]',
                    current_step         INTEGER NOT NULL DEFAULT 0,
                    completed_steps      TEXT NOT NULL DEFAULT '[]',
                    pending_steps        TEXT NOT NULL DEFAULT '[]',
                    last_action          TEXT,
                    last_observation     TEXT,
                    recommended_next_action TEXT,
                    recommended_tool     TEXT,
                    latest_diff_summary  TEXT,
                    latest_test_result   TEXT,
                    focus_paths          TEXT NOT NULL DEFAULT '[]',
                    warnings             TEXT NOT NULL DEFAULT '[]',
                    retryable_action     TEXT,
                    policy_profile       TEXT NOT NULL DEFAULT '{}',
                    outcome_kind         TEXT,
                    finalized_outcome    TEXT,
                    reopen_metadata      TEXT,
                    supersedes_run_id    TEXT,
                    superseded_by_run_id TEXT,
                    supersession_reason  TEXT,
                    superseded_at        TEXT,
                    is_archived          INTEGER DEFAULT 0,
                    archive_metadata     TEXT,
                    created_at           TEXT NOT NULL,
                    updated_at           TEXT NOT NULL
                );",
            ).unwrap();
            conn.execute(
                "INSERT INTO runs (run_id, workspace_id, user_goal, status, plan, created_at, updated_at)
                 VALUES ('r_m13', '/tmp/ws', 'old goal', 'finalized:completed', '[]', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).unwrap();
        }

        // Open with migration — should add M14 unarchive_metadata column.
        let store = Store::open(dir.path()).unwrap();
        let loaded = store.get_run("r_m13").unwrap().unwrap();

        // M14 unarchive_metadata must default to None.
        assert!(loaded.unarchive_metadata.is_none());
    }

    // ---- Milestone 15: annotation persistence ----

    #[test]
    fn annotation_roundtrip() {
        use deterministic_protocol::RunAnnotation;

        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_ann_rt", "active");
        state.annotation = Some(RunAnnotation {
            labels: vec!["auth".into(), "ci".into()],
            operator_note: Some("tracking regression".into()),
        });
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_ann_rt").unwrap().unwrap();
        let annotation = loaded.annotation.expect("annotation must persist");
        assert_eq!(annotation.labels, vec!["auth", "ci"]);
        assert_eq!(annotation.operator_note.as_deref(), Some("tracking regression"));
    }

    #[test]
    fn annotation_defaults_to_none() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_ann_none", "active");
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_ann_none").unwrap().unwrap();
        assert!(loaded.annotation.is_none(), "annotation must default to None");
    }

    #[test]
    fn list_runs_filter_by_label() {
        use deterministic_protocol::RunAnnotation;

        let store = Store::open_in_memory().unwrap();
        let mut auth_state = make_run_state("r_lbl_auth", "active");
        auth_state.annotation = Some(RunAnnotation {
            labels: vec!["auth".into()],
            operator_note: None,
        });
        store.save_run(&auth_state).unwrap();

        let mut infra_state = make_run_state("r_lbl_infra", "active");
        infra_state.annotation = Some(RunAnnotation {
            labels: vec!["infra".into()],
            operator_note: None,
        });
        store.save_run(&infra_state).unwrap();

        let unlabeled = make_run_state("r_lbl_none", "active");
        store.save_run(&unlabeled).unwrap();

        // Filter by label=auth
        let auth_runs = store.list_runs(20, None, None, false, false, Some("auth"), false, false, false).unwrap();
        assert!(auth_runs.iter().any(|r| r.run_id == "r_lbl_auth"), "auth-labeled run must match");
        assert!(!auth_runs.iter().any(|r| r.run_id == "r_lbl_infra"), "infra-labeled run must not match");
        assert!(!auth_runs.iter().any(|r| r.run_id == "r_lbl_none"), "unlabeled run must not match");

        // Filter by label=infra
        let infra_runs = store.list_runs(20, None, None, false, false, Some("infra"), false, false, false).unwrap();
        assert!(infra_runs.iter().any(|r| r.run_id == "r_lbl_infra"), "infra-labeled run must match");
        assert!(!infra_runs.iter().any(|r| r.run_id == "r_lbl_auth"), "auth-labeled run must not match");

        // No label filter — all non-archived runs returned.
        let all_runs = store.list_runs(20, None, None, false, false, None, false, false, false).unwrap();
        assert_eq!(all_runs.len(), 3);
    }

    #[test]
    fn list_runs_summary_carries_annotation_fields() {
        use deterministic_protocol::RunAnnotation;

        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_ann_sum", "active");
        state.annotation = Some(RunAnnotation {
            labels: vec!["blocked".into()],
            operator_note: Some("waiting for review".into()),
        });
        store.save_run(&state).unwrap();

        let runs = store.list_runs(20, None, None, false, false, None, false, false, false).unwrap();
        let summary = runs.iter().find(|r| r.run_id == "r_ann_sum").unwrap();
        assert_eq!(summary.labels, vec!["blocked"]);
        assert_eq!(summary.operator_note.as_deref(), Some("waiting for review"));
    }

    /// Verify that migration from M14 schema (no M15 annotation column) works safely.
    #[test]
    fn migration_m15_annotation_defaults_safely() {
        use rusqlite::Connection;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("runs.db");

        // Simulate a Milestone 14 schema (no M15 annotation column).
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    run_id               TEXT PRIMARY KEY,
                    workspace_id         TEXT NOT NULL,
                    user_goal            TEXT NOT NULL,
                    status               TEXT NOT NULL,
                    plan                 TEXT NOT NULL DEFAULT '[]',
                    current_step         INTEGER NOT NULL DEFAULT 0,
                    completed_steps      TEXT NOT NULL DEFAULT '[]',
                    pending_steps        TEXT NOT NULL DEFAULT '[]',
                    last_action          TEXT,
                    last_observation     TEXT,
                    recommended_next_action TEXT,
                    recommended_tool     TEXT,
                    latest_diff_summary  TEXT,
                    latest_test_result   TEXT,
                    focus_paths          TEXT NOT NULL DEFAULT '[]',
                    warnings             TEXT NOT NULL DEFAULT '[]',
                    retryable_action     TEXT,
                    policy_profile       TEXT NOT NULL DEFAULT '{}',
                    outcome_kind         TEXT,
                    finalized_outcome    TEXT,
                    reopen_metadata      TEXT,
                    supersedes_run_id    TEXT,
                    superseded_by_run_id TEXT,
                    supersession_reason  TEXT,
                    superseded_at        TEXT,
                    is_archived          INTEGER DEFAULT 0,
                    archive_metadata     TEXT,
                    unarchive_metadata   TEXT,
                    created_at           TEXT NOT NULL,
                    updated_at           TEXT NOT NULL
                );",
            ).unwrap();
            conn.execute(
                "INSERT INTO runs (run_id, workspace_id, user_goal, status, plan, created_at, updated_at)
                 VALUES ('r_m14', '/tmp/ws', 'old goal', 'active', '[]', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).unwrap();
        }

        // Open with migration — should add M15 annotation column.
        let store = Store::open(dir.path()).unwrap();
        let loaded = store.get_run("r_m14").unwrap().unwrap();

        // M15 annotation must default to None.
        assert!(loaded.annotation.is_none());
    }

    /// Verify that migration from M16 schema (no M17 snooze columns) works safely.
    #[test]
    fn migration_m17_snooze_defaults_safely() {
        use rusqlite::Connection;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("runs.db");

        // Simulate a Milestone 16 schema (no M17 snooze columns).
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE runs (
                    run_id               TEXT PRIMARY KEY,
                    workspace_id         TEXT NOT NULL,
                    user_goal            TEXT NOT NULL,
                    status               TEXT NOT NULL,
                    plan                 TEXT NOT NULL DEFAULT '[]',
                    current_step         INTEGER NOT NULL DEFAULT 0,
                    completed_steps      TEXT NOT NULL DEFAULT '[]',
                    pending_steps        TEXT NOT NULL DEFAULT '[]',
                    last_action          TEXT,
                    last_observation     TEXT,
                    recommended_next_action TEXT,
                    recommended_tool     TEXT,
                    latest_diff_summary  TEXT,
                    latest_test_result   TEXT,
                    focus_paths          TEXT NOT NULL DEFAULT '[]',
                    warnings             TEXT NOT NULL DEFAULT '[]',
                    retryable_action     TEXT,
                    policy_profile       TEXT NOT NULL DEFAULT '{}',
                    outcome_kind         TEXT,
                    finalized_outcome    TEXT,
                    reopen_metadata      TEXT,
                    supersedes_run_id    TEXT,
                    superseded_by_run_id TEXT,
                    supersession_reason  TEXT,
                    superseded_at        TEXT,
                    is_archived          INTEGER DEFAULT 0,
                    archive_metadata     TEXT,
                    unarchive_metadata   TEXT,
                    annotation           TEXT,
                    pin_metadata         TEXT,
                    created_at           TEXT NOT NULL,
                    updated_at           TEXT NOT NULL
                );",
            ).unwrap();
            conn.execute(
                "INSERT INTO runs (run_id, workspace_id, user_goal, status, plan, created_at, updated_at)
                 VALUES ('r_m16', '/tmp/ws', 'old goal', 'active', '[]', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).unwrap();
        }

        // Open with migration — should add M17 is_snoozed and snooze_metadata columns.
        let store = Store::open(dir.path()).unwrap();
        let loaded = store.get_run("r_m16").unwrap().unwrap();

        // M17 snooze_metadata must default to None.
        assert!(loaded.snooze_metadata.is_none());
    }

    // -----------------------------------------------------------------------
    // Milestone 17: snooze persistence roundtrip tests
    // -----------------------------------------------------------------------

    #[test]
    fn snooze_metadata_roundtrips() {
        use deterministic_protocol::SnoozeMetadata;

        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_snz_rt", "active");
        state.snooze_metadata = Some(SnoozeMetadata {
            reason: "blocked on review".into(),
            snoozed_at: "2024-06-01T12:00:00Z".into(),
        });
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_snz_rt").unwrap().unwrap();
        let meta = loaded.snooze_metadata.unwrap();
        assert_eq!(meta.reason, "blocked on review");
        assert_eq!(meta.snoozed_at, "2024-06-01T12:00:00Z");
    }

    #[test]
    fn snoozed_run_excluded_from_default_list() {
        use deterministic_protocol::SnoozeMetadata;

        let store = Store::open_in_memory().unwrap();
        let mut snoozed = make_run_state("r_snz_excl", "active");
        snoozed.snooze_metadata = Some(SnoozeMetadata {
            reason: "deferred".into(),
            snoozed_at: "2024-06-01T12:00:00Z".into(),
        });
        store.save_run(&snoozed).unwrap();

        let normal = make_run_state("r_snz_normal", "active");
        store.save_run(&normal).unwrap();

        // Default: snoozed excluded.
        let runs = store.list_runs(20, None, None, false, false, None, false, false, false).unwrap();
        assert!(!runs.iter().any(|r| r.run_id == "r_snz_excl"), "snoozed must be excluded by default");
        assert!(runs.iter().any(|r| r.run_id == "r_snz_normal"), "normal must be included by default");
    }

    #[test]
    fn snoozed_run_included_with_include_snoozed() {
        use deterministic_protocol::SnoozeMetadata;

        let store = Store::open_in_memory().unwrap();
        let mut snoozed = make_run_state("r_snz_incl2", "active");
        snoozed.snooze_metadata = Some(SnoozeMetadata {
            reason: "deferred".into(),
            snoozed_at: "2024-06-01T12:00:00Z".into(),
        });
        store.save_run(&snoozed).unwrap();

        let normal = make_run_state("r_snz_normal2", "active");
        store.save_run(&normal).unwrap();

        // include_snoozed=true: both returned.
        let runs = store.list_runs(20, None, None, false, false, None, false, true, false).unwrap();
        assert!(runs.iter().any(|r| r.run_id == "r_snz_incl2"), "snoozed must be included");
        assert!(runs.iter().any(|r| r.run_id == "r_snz_normal2"), "normal must also be included");
    }

    #[test]
    fn snoozed_only_filter() {
        use deterministic_protocol::SnoozeMetadata;

        let store = Store::open_in_memory().unwrap();
        let mut snoozed = make_run_state("r_snz_only2", "active");
        snoozed.snooze_metadata = Some(SnoozeMetadata {
            reason: "deferred".into(),
            snoozed_at: "2024-06-01T12:00:00Z".into(),
        });
        store.save_run(&snoozed).unwrap();

        let normal = make_run_state("r_snz_norm3", "active");
        store.save_run(&normal).unwrap();

        // snoozed_only=true: only snoozed returned.
        let runs = store.list_runs(20, None, None, false, false, None, false, false, true).unwrap();
        assert!(runs.iter().any(|r| r.run_id == "r_snz_only2"), "snoozed must appear");
        assert!(!runs.iter().any(|r| r.run_id == "r_snz_norm3"), "normal must not appear");
    }

    #[test]
    fn list_runs_summary_carries_snooze_fields() {
        use deterministic_protocol::SnoozeMetadata;

        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_snz_sum", "active");
        state.snooze_metadata = Some(SnoozeMetadata {
            reason: "blocked".into(),
            snoozed_at: "2024-06-01T12:00:00Z".into(),
        });
        store.save_run(&state).unwrap();

        // Use include_snoozed to see it.
        let runs = store.list_runs(20, None, None, false, false, None, false, true, false).unwrap();
        let summary = runs.iter().find(|r| r.run_id == "r_snz_sum").unwrap();
        assert_eq!(summary.is_snoozed, Some(true));
        assert_eq!(summary.snooze_reason.as_deref(), Some("blocked"));
        assert_eq!(summary.snoozed_at.as_deref(), Some("2024-06-01T12:00:00Z"));
    }

    // ----- Milestone 20: due date persistence -----

    #[test]
    fn due_date_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_dd_1", "active");
        state.due_date = Some("2026-03-31".into());
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_dd_1").unwrap().unwrap();
        assert_eq!(loaded.due_date.as_deref(), Some("2026-03-31"));
    }

    #[test]
    fn due_date_clear_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_dd_2", "active");
        state.due_date = Some("2026-03-31".into());
        store.save_run(&state).unwrap();

        state.due_date = None;
        store.save_run(&state).unwrap();
        let loaded = store.get_run("r_dd_2").unwrap().unwrap();
        assert_eq!(loaded.due_date, None);
    }

    #[test]
    fn list_runs_summary_carries_due_date() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_dd_sum", "active");
        state.due_date = Some("2026-06-30".into());
        store.save_run(&state).unwrap();

        let runs = store.list_runs(20, None, None, false, false, None, false, false, false).unwrap();
        let summary = runs.iter().find(|r| r.run_id == "r_dd_sum").unwrap();
        assert_eq!(summary.due_date.as_deref(), Some("2026-06-30"));
    }

    #[test]
    fn due_date_none_in_summary_when_not_set() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_dd_none", "active");
        store.save_run(&state).unwrap();

        let runs = store.list_runs(20, None, None, false, false, None, false, false, false).unwrap();
        let summary = runs.iter().find(|r| r.run_id == "r_dd_none").unwrap();
        assert_eq!(summary.due_date, None);
    }
}
