//! SQLite-backed persistence for run state.
//!
//! The daemon stores all run state in a local SQLite database.  This
//! provides ACID transactions, schema enforcement, and safe concurrent
//! access — unlike the previous JSON-file approach.

use anyhow::{Context, Result};
use deterministic_protocol::RunState;
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
    fn migrate(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS runs (
                run_id       TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL,
                user_goal    TEXT NOT NULL,
                status       TEXT NOT NULL,
                plan         TEXT NOT NULL,   -- JSON array of strings
                current_step INTEGER NOT NULL DEFAULT 0,
                created_at   TEXT NOT NULL,
                updated_at   TEXT NOT NULL
            );",
        )
        .context("failed to run migrations")?;
        Ok(())
    }

    /// Persist a run state (insert or replace).
    pub fn save_run(&self, state: &RunState) -> Result<()> {
        let conn = self.conn.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let plan_json =
            serde_json::to_string(&state.plan).context("failed to serialise plan")?;
        conn.execute(
            "INSERT OR REPLACE INTO runs
                (run_id, workspace_id, user_goal, status, plan, current_step, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                state.run_id,
                state.workspace_id,
                state.user_goal,
                state.status,
                plan_json,
                state.current_step,
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
                        current_step, created_at, updated_at
                 FROM runs WHERE run_id = ?1",
            )
            .context("failed to prepare statement")?;

        let mut rows = stmt
            .query_map(rusqlite::params![run_id], |row| {
                let plan_json: String = row.get(4)?;
                let plan: Vec<String> =
                    serde_json::from_str(&plan_json).unwrap_or_default();
                Ok(RunState {
                    run_id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    user_goal: row.get(2)?,
                    status: row.get(3)?,
                    plan,
                    current_step: row.get::<_, i64>(5)? as usize,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_in_memory() {
        let store = Store::open_in_memory().unwrap();

        let state = RunState {
            run_id: "r1".into(),
            workspace_id: "/tmp/ws".into(),
            user_goal: "fix".into(),
            status: "prepared".into(),
            plan: vec!["step 1".into(), "step 2".into()],
            current_step: 0,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        };
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r1").unwrap().unwrap();
        assert_eq!(loaded.workspace_id, "/tmp/ws");
        assert_eq!(loaded.plan.len(), 2);
    }

    #[test]
    fn roundtrip_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();

        let state = RunState {
            run_id: "r1".into(),
            workspace_id: "/tmp/ws".into(),
            user_goal: "fix".into(),
            status: "prepared".into(),
            plan: vec!["step 1".into()],
            current_step: 0,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        };
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

        let mut state = RunState {
            run_id: "r1".into(),
            workspace_id: "/tmp/ws".into(),
            user_goal: "fix".into(),
            status: "prepared".into(),
            plan: vec!["step 1".into()],
            current_step: 0,
            created_at: "2024-01-01T00:00:00Z".into(),
            updated_at: "2024-01-01T00:00:00Z".into(),
        };
        store.save_run(&state).unwrap();

        state.status = "running".to_string();
        state.current_step = 1;
        store.save_run(&state).unwrap();

        let loaded = store.get_run("r1").unwrap().unwrap();
        assert_eq!(loaded.status, "running");
        assert_eq!(loaded.current_step, 1);
    }
}

