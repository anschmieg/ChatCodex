//! SQLite-backed persistence for run state.
//!
//! The daemon stores all run state in a local SQLite database.  This
//! provides ACID transactions, schema enforcement, and safe concurrent
//! access — unlike the previous JSON-file approach.

use anyhow::{Context, Result};
use deterministic_protocol::{PendingApproval, RunState};
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
        // Note: This creates tables with the full Milestone 4 schema for new databases.
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
                warnings                 TEXT NOT NULL DEFAULT '[]',
                created_at               TEXT NOT NULL,
                updated_at               TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS approvals (
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
        .context("failed to create tables")?;

        // Migrate older databases: add columns that may be missing from Milestone 3 era.
        // SQLite supports ALTER TABLE ADD COLUMN; we ignore errors if columns already exist.
        let migrations = [
            ("runs", "completed_steps", "TEXT NOT NULL DEFAULT '[]'"),
            ("runs", "pending_steps", "TEXT NOT NULL DEFAULT '[]'"),
            ("runs", "last_action", "TEXT"),
            ("runs", "last_observation", "TEXT"),
            ("runs", "recommended_next_action", "TEXT"),
            ("runs", "recommended_tool", "TEXT"),
            ("runs", "latest_diff_summary", "TEXT"),
            ("runs", "latest_test_result", "TEXT"),
            ("runs", "warnings", "TEXT NOT NULL DEFAULT '[]'"),
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
        let warnings_json =
            serde_json::to_string(&state.warnings).context("failed to serialise warnings")?;
        conn.execute(
            "INSERT OR REPLACE INTO runs
                (run_id, workspace_id, user_goal, status, plan, current_step,
                 completed_steps, pending_steps, last_action, last_observation,
                 recommended_next_action, recommended_tool,
                 latest_diff_summary, latest_test_result, warnings,
                 created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
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
                warnings_json,
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
                        latest_diff_summary, latest_test_result, warnings,
                        created_at, updated_at
                 FROM runs WHERE run_id = ?1",
            )
            .context("failed to prepare statement")?;

        let mut rows = stmt
            .query_map(rusqlite::params![run_id], |row| {
                let plan_json: String = row.get(4)?;
                let completed_json: String = row.get(6)?;
                let pending_json: String = row.get(7)?;
                let warnings_json: String = row.get(14)?;

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
                let warnings: Vec<String> =
                    serde_json::from_str(&warnings_json).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            14,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;

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
                    warnings,
                    created_at: row.get(15)?,
                    updated_at: row.get(16)?,
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
                (approval_id, run_id, action_description, risk_reason, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                approval.approval_id,
                approval.run_id,
                approval.action_description,
                approval.risk_reason,
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
                "SELECT approval_id, run_id, action_description, risk_reason, status, created_at
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
                    status: row.get(4)?,
                    created_at: row.get(5)?,
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
                "SELECT approval_id, run_id, action_description, risk_reason, status, created_at
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
                    status: row.get(4)?,
                    created_at: row.get(5)?,
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
            warnings: vec![],
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
}

