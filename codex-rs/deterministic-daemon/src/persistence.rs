//! SQLite-backed persistence for run state.
//!
//! The daemon stores all run state in a local SQLite database.  This
//! provides ACID transactions, schema enforcement, and safe concurrent
//! access — unlike the previous JSON-file approach.

use anyhow::{Context, Result};
use deterministic_protocol::{PendingApproval, RetryableAction, RunHistoryEntry, RunPolicy, RunState, RunSummary};
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
        conn.execute(
            "INSERT OR REPLACE INTO runs
                (run_id, workspace_id, user_goal, status, plan, current_step,
                 completed_steps, pending_steps, last_action, last_observation,
                 recommended_next_action, recommended_tool,
                 latest_diff_summary, latest_test_result, focus_paths, warnings,
                 retryable_action, policy_profile, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
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
                        retryable_action, policy_profile, created_at, updated_at
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

                // Milestone 8: load policy_profile.
                // Old rows may have NULL or '{}' — fall back to default policy
                // with focus_paths populated from the focus_paths column.
                let policy_profile_json: Option<String> = row.get(17)?;
                let mut policy_profile: RunPolicy =
                    policy_profile_json
                        .as_deref()
                        .filter(|s| !s.is_empty() && *s != "{}")
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or_default();
                // Back-fill focus_paths from the dedicated column when the
                // persisted policy profile doesn't have them set (old records).
                if policy_profile.focus_paths.is_empty() && !focus_paths.is_empty() {
                    policy_profile.focus_paths = focus_paths.clone();
                }

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
                    created_at: row.get(18)?,
                    updated_at: row.get(19)?,
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

    /// List runs, ordered by updated_at descending.
    pub fn list_runs(
        &self,
        limit: usize,
        workspace_id: Option<&str>,
        status_filter: Option<&str>,
    ) -> Result<Vec<RunSummary>> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;

        // Build query dynamically based on optional filters.
        let mut conditions = Vec::new();
        if workspace_id.is_some() {
            conditions.push("workspace_id = ?2");
        }
        if status_filter.is_some() {
            let idx = if workspace_id.is_some() { 3 } else { 2 };
            conditions.push(match idx {
                2 => "status = ?2",
                _ => "status = ?3",
            });
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let sql = format!(
            "SELECT run_id, workspace_id, user_goal, status, current_step, plan,
                    created_at, updated_at
             FROM runs {where_clause}
             ORDER BY updated_at DESC
             LIMIT ?1"
        );

        let mut stmt = conn.prepare(&sql).context("failed to prepare list_runs")?;

        let mapper = |row: &rusqlite::Row<'_>| -> rusqlite::Result<RunSummary> {
            let plan_json: String = row.get(5)?;
            let total_steps: usize = serde_json::from_str::<Vec<String>>(&plan_json)
                .map(|v| v.len())
                .unwrap_or(0);
            Ok(RunSummary {
                run_id: row.get(0)?,
                workspace_id: row.get(1)?,
                user_goal: row.get(2)?,
                status: row.get(3)?,
                current_step: row.get::<_, i64>(4)? as usize,
                total_steps,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        };

        let rows = match (workspace_id, status_filter) {
            (None, None) => stmt
                .query_map(rusqlite::params![limit as i64], mapper)
                .context("failed to query runs")?,
            (Some(ws), None) => stmt
                .query_map(rusqlite::params![limit as i64, ws], mapper)
                .context("failed to query runs")?,
            (None, Some(st)) => stmt
                .query_map(rusqlite::params![limit as i64, st], mapper)
                .context("failed to query runs")?,
            (Some(ws), Some(st)) => stmt
                .query_map(rusqlite::params![limit as i64, ws, st], mapper)
                .context("failed to query runs")?,
        };

        let mut summaries = Vec::new();
        for row in rows {
            summaries.push(row.map_err(|e| anyhow::anyhow!("failed to read run row: {e}"))?);
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
        let runs = store.list_runs(20, None, None).unwrap();
        assert!(runs.is_empty());
    }

    #[test]
    fn list_runs_returns_summaries() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_a", "active")).unwrap();
        store.save_run(&make_run_state("r_b", "prepared")).unwrap();
        store.save_run(&make_run_state("r_c", "done")).unwrap();

        let runs = store.list_runs(20, None, None).unwrap();
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
        let runs = store.list_runs(3, None, None).unwrap();
        assert_eq!(runs.len(), 3);
    }

    #[test]
    fn list_runs_filters_by_status() {
        let store = Store::open_in_memory().unwrap();
        store.save_run(&make_run_state("r_a", "active")).unwrap();
        store.save_run(&make_run_state("r_b", "active")).unwrap();
        store.save_run(&make_run_state("r_c", "done")).unwrap();

        let active = store.list_runs(20, None, Some("active")).unwrap();
        assert_eq!(active.len(), 2);

        let done = store.list_runs(20, None, Some("done")).unwrap();
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

        let ws1 = store.list_runs(20, Some("/ws/one"), None).unwrap();
        assert_eq!(ws1.len(), 2);

        let ws2 = store.list_runs(20, Some("/ws/two"), None).unwrap();
        assert_eq!(ws2.len(), 1);
    }

    #[test]
    fn list_runs_total_steps_matches_plan() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_steps", "active"); // plan has 2 steps
        store.save_run(&state).unwrap();

        let runs = store.list_runs(20, None, None).unwrap();
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

    // ---- Milestone 8: policy profile persistence tests ----

    #[test]
    fn policy_profile_default_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let state = make_run_state("r_pol_default", "active");
        // Default policy is set by make_run_state via RunPolicy::default().
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_pol_default").unwrap().unwrap();
        assert_eq!(loaded.policy_profile.patch_edit_threshold, 5);
        assert!(loaded.policy_profile.delete_requires_approval);
        assert!(loaded.policy_profile.sensitive_path_requires_approval);
        assert!(loaded.policy_profile.outside_focus_requires_approval);
        assert!(loaded.policy_profile.extra_safe_make_targets.is_empty());
        assert!(loaded.policy_profile.focus_paths.is_empty());
    }

    #[test]
    fn policy_profile_custom_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_pol_custom", "active");
        state.policy_profile = RunPolicy {
            patch_edit_threshold: 10,
            delete_requires_approval: false,
            sensitive_path_requires_approval: true,
            outside_focus_requires_approval: false,
            extra_safe_make_targets: vec!["deploy-staging".to_string(), "release".to_string()],
            focus_paths: vec!["src/".to_string(), "tests/".to_string()],
        };
        state.focus_paths = state.policy_profile.focus_paths.clone();
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r_pol_custom").unwrap().unwrap();
        assert_eq!(loaded.policy_profile.patch_edit_threshold, 10);
        assert!(!loaded.policy_profile.delete_requires_approval);
        assert!(loaded.policy_profile.sensitive_path_requires_approval);
        assert!(!loaded.policy_profile.outside_focus_requires_approval);
        assert_eq!(
            loaded.policy_profile.extra_safe_make_targets,
            vec!["deploy-staging", "release"]
        );
        assert_eq!(
            loaded.policy_profile.focus_paths,
            vec!["src/", "tests/"]
        );
    }

    #[test]
    fn policy_profile_focus_paths_backfilled_from_focus_paths_column() {
        // Simulate an old DB record that has focus_paths set but no policy_profile.
        let store = Store::open_in_memory().unwrap();
        let mut state = make_run_state("r_backfill", "active");
        state.focus_paths = vec!["src/".to_string()];
        // Explicitly set policy_profile.focus_paths empty to simulate old record.
        state.policy_profile = RunPolicy {
            focus_paths: vec![],
            ..RunPolicy::default()
        };
        store.save_run(&state).unwrap();

        // The policy_profile JSON stored will have empty focus_paths.
        // When loaded, the code should back-fill from the focus_paths column.
        let loaded = store.get_run("r_backfill").unwrap().unwrap();
        // focus_paths column is set, so policy_profile.focus_paths should be back-filled.
        assert_eq!(loaded.policy_profile.focus_paths, vec!["src/"]);
    }

    #[test]
    fn migration_from_m6_adds_policy_profile_column() {
        use rusqlite::Connection;

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("runs.db");

        // Create Milestone 6 schema (without policy_profile column).
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
                 VALUES ('r_m6_pol', '/tmp/ws', 'fix', 'active', '[\"step 1\"]', 0,
                         '[]', '[\"step 1\"]', '[\"src/\"]', '[]',
                         '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        }

        // Open with Store — should migrate and add policy_profile column.
        let store = Store::open(dir.path()).unwrap();
        let loaded = store.get_run("r_m6_pol").unwrap().unwrap();

        // Old run without explicit policy_profile gets default values.
        assert_eq!(loaded.policy_profile.patch_edit_threshold, 5);
        assert!(loaded.policy_profile.delete_requires_approval);
        // focus_paths should be back-filled from the focus_paths column.
        assert_eq!(loaded.policy_profile.focus_paths, vec!["src/"]);

        // Should be able to save a run with an explicit policy_profile.
        let mut updated = loaded;
        updated.policy_profile = RunPolicy {
            patch_edit_threshold: 15,
            ..RunPolicy::default()
        };
        store.save_run(&updated).unwrap();

        let reloaded = store.get_run("r_m6_pol").unwrap().unwrap();
        assert_eq!(reloaded.policy_profile.patch_edit_threshold, 15);
    }
}

