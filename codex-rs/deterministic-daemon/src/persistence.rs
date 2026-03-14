//! SQLite-backed persistence for run state.
//!
//! We use a simple file-backed JSON store (no external SQLite C
//! library needed) to keep the first slice minimal.  A future
//! milestone may upgrade to `rusqlite` or `sqlx`.

use anyhow::{Context, Result};
use deterministic_protocol::RunState;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// In-process run-state store backed by a JSON file.
pub struct Store {
    path: PathBuf,
    cache: Mutex<HashMap<String, RunState>>,
}

impl Store {
    /// Open (or create) the store at the given directory.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).context("cannot create store directory")?;
        let path = dir.join("runs.json");
        let cache = if path.exists() {
            let data = std::fs::read_to_string(&path).context("cannot read store file")?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            HashMap::new()
        };
        Ok(Self {
            path,
            cache: Mutex::new(cache),
        })
    }

    /// Save a run state.
    pub fn save_run(&self, state: &RunState) -> Result<()> {
        let mut cache = self.cache.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        cache.insert(state.run_id.clone(), state.clone());
        let data = serde_json::to_string_pretty(&*cache)?;
        std::fs::write(&self.path, data).context("cannot write store file")?;
        Ok(())
    }

    /// Get a run state by ID.
    pub fn get_run(&self, run_id: &str) -> Result<Option<RunState>> {
        let cache = self.cache.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(cache.get(run_id).cloned())
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
    fn roundtrip() {
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

        let loaded = store.get_run("r1").unwrap().unwrap();
        assert_eq!(loaded.workspace_id, "/tmp/ws");

        // Re-open from disk
        let store2 = Store::open(dir.path()).unwrap();
        let loaded2 = store2.get_run("r1").unwrap().unwrap();
        assert_eq!(loaded2.user_goal, "fix");
    }
}
