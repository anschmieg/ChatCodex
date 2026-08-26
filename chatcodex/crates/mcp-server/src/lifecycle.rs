use std::collections::BTreeMap;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

const STORE_DIR_NAME: &str = ".chatcodex";
const STATE_FILE_NAME: &str = "state.json";
const LOCK_FILE_NAME: &str = "state.lock";
const SCHEMA_VERSION: u32 = 1;
const LOCK_STALE_AFTER_MS: u64 = 30_000;
const LOCK_RETRY_COUNT: usize = 200;
const LOCK_RETRY_DELAY_MS: u64 = 10;
const DEFAULT_LEASE_TTL_MS: u64 = 120_000;
const DEFAULT_LEASE_DELAY_MS: u64 = 1_000;

#[derive(Clone, Debug)]
pub struct LifecycleStore {
    client_id: String,
    state_dir: PathBuf,
    state_file: PathBuf,
    lock_file: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ClientState {
    pub schema_version: u32,
    pub client_id: String,
    #[serde(default)]
    pub active_project_id: Option<String>,
    #[serde(default)]
    pub active_run_id: Option<String>,
    #[serde(default)]
    pub projects: BTreeMap<String, Project>,
    #[serde(default)]
    pub runs: BTreeMap<String, Run>,
    #[serde(default = "default_legacy_plan")]
    pub legacy_plan: Value,
    #[serde(default)]
    pub legacy_todo: Vec<ChecklistItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub kind: ProjectKind,
    pub workspace_root: PathBuf,
    pub source: ProjectSource,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectKind {
    Repo,
    Workspace,
    Scratch,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProjectSource {
    Scratch,
    Git {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host: Option<String>,
        path: String,
    },
    Workspace {
        registered_path: PathBuf,
    },
}

#[derive(Clone, Debug)]
pub struct ProjectUpsert {
    pub kind: ProjectKind,
    pub name: String,
    pub workspace_root: PathBuf,
    pub source: ProjectSource,
    pub select: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ProjectMutation {
    pub project: Project,
    pub action: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Run {
    pub id: String,
    pub project_id: String,
    pub objective: String,
    pub acceptance_criteria: Vec<String>,
    pub phase: RunPhase,
    pub status: RunStatus,
    pub plan: Vec<PlanItem>,
    pub checklist: Vec<ChecklistItem>,
    pub checkpoints: Vec<Checkpoint>,
    pub autonomy: AutonomyEnvelope,
    pub counters: RunCounters,
    pub continuation: ContinuationState,
    pub work_remaining: bool,
    pub next_action: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub started_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancelled_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    Inspect,
    Plan,
    Execute,
    Verify,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Active,
    Paused,
    Blocked,
    AwaitingApproval,
    Completed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct PlanItem {
    pub step: String,
    pub status: PlanStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ChecklistItem {
    pub id: String,
    pub description: String,
    pub status: ChecklistStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChecklistStatus {
    Pending,
    Checked,
    Dismissed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct Checkpoint {
    pub sequence: u64,
    pub message: String,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct CheckpointInput {
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct AutonomyEnvelope {
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    #[serde(default = "default_max_runtime_seconds")]
    pub max_runtime_seconds: u64,
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    #[serde(default = "default_true")]
    pub allow_local_commands: bool,
    #[serde(default = "default_true")]
    pub allow_file_edits: bool,
    #[serde(default)]
    pub allow_git_commits: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub struct RunCounters {
    pub turns_used: u32,
    pub runtime_seconds_used: u64,
    pub steps_used: u32,
    pub continuation_leases_issued: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub struct ContinuationState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acquired_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct RunStart {
    pub project_id: Option<String>,
    pub objective: String,
    pub acceptance_criteria: Vec<String>,
    pub autonomy: AutonomyEnvelope,
    pub select: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct RunMutation {
    pub run: Run,
}

#[derive(Clone, Debug, Default)]
pub struct RunUpdate {
    pub run_id: Option<String>,
    pub phase: Option<RunPhase>,
    pub status: Option<RunStatus>,
    pub acceptance_criteria: Option<Vec<String>>,
    pub plan: Option<Vec<PlanItem>>,
    pub checklist: Option<Vec<ChecklistItem>>,
    pub checkpoint: Option<CheckpointInput>,
    pub work_remaining: Option<bool>,
    pub next_action: Option<String>,
    pub step_delta: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct FollowupLeaseRequest {
    pub run_id: String,
    pub requested_nonce: Option<String>,
    pub now_ms: Option<u64>,
    pub ttl_ms: Option<u64>,
    pub delay_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct FollowupLeaseResponse {
    pub run_id: String,
    pub granted: bool,
    pub duplicate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acquired_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
    pub delay_ms: u64,
    pub max_turns: u32,
    pub max_runtime_seconds: u64,
    pub max_steps: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Default for AutonomyEnvelope {
    fn default() -> Self {
        Self {
            max_turns: default_max_turns(),
            max_runtime_seconds: default_max_runtime_seconds(),
            max_steps: default_max_steps(),
            allow_local_commands: true,
            allow_file_edits: true,
            allow_git_commits: false,
        }
    }
}

fn default_max_turns() -> u32 {
    8
}

fn default_max_runtime_seconds() -> u64 {
    7_200
}

fn default_max_steps() -> u32 {
    200
}

fn default_true() -> bool {
    true
}

impl LifecycleStore {
    pub fn open(workspace_base: impl AsRef<Path>, client_id: &str) -> anyhow::Result<Self> {
        let client_id = sanitize_id_component(client_id)?;
        let client_root = workspace_base.as_ref().join("clients").join(&client_id);
        let state_dir = client_root.join(STORE_DIR_NAME);
        Ok(Self {
            client_id,
            state_file: state_dir.join(STATE_FILE_NAME),
            lock_file: state_dir.join(LOCK_FILE_NAME),
            state_dir,
        })
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn snapshot(&self) -> anyhow::Result<ClientState> {
        self.load()
    }

    pub fn list_projects(&self) -> anyhow::Result<Vec<Project>> {
        Ok(self.load()?.projects.into_values().collect())
    }

    pub fn get_project(&self, project_id: &str) -> anyhow::Result<Project> {
        self.load()?
            .projects
            .get(project_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown project: {project_id}"))
    }

    pub fn active_project(&self) -> anyhow::Result<Option<Project>> {
        let state = self.load()?;
        Ok(state
            .active_project_id
            .as_deref()
            .and_then(|id| state.projects.get(id))
            .cloned())
    }

    pub fn active_run(&self) -> anyhow::Result<Option<Run>> {
        let state = self.load()?;
        Ok(state
            .active_run_id
            .as_deref()
            .and_then(|id| state.runs.get(id))
            .cloned())
    }

    pub fn consume_active_step(&self) -> anyhow::Result<Option<String>> {
        self.mutate(|state| {
            let Some(run_id) = state.active_run_id.clone() else {
                return Ok(None);
            };
            let run = state
                .runs
                .get_mut(&run_id)
                .ok_or_else(|| anyhow::anyhow!("unknown active run: {run_id}"))?;
            if run.status != RunStatus::Active {
                return Ok(Some(format!(
                    "selected run {} is {}; coding tools require an active run",
                    run.id,
                    run.status.as_str()
                )));
            }

            let now = now_ms();
            let elapsed_seconds = now
                .saturating_sub(run.started_at_ms)
                .checked_div(1_000)
                .unwrap_or(0);
            run.counters.runtime_seconds_used = elapsed_seconds;
            let limit_reason = if elapsed_seconds >= run.autonomy.max_runtime_seconds {
                Some("runtime limit reached")
            } else if run.counters.steps_used >= run.autonomy.max_steps {
                Some("step limit reached")
            } else {
                None
            };
            if let Some(reason) = limit_reason {
                apply_status(run, RunStatus::Paused, now);
                run.next_action = format!("{reason}; start a new run with a larger envelope");
                run.updated_at_ms = now;
                return Ok(Some(format!("run {} paused: {reason}", run.id)));
            }

            run.counters.steps_used = run.counters.steps_used.saturating_add(1);
            run.updated_at_ms = now;
            Ok(None)
        })
    }

    pub fn upsert_project(&self, input: ProjectUpsert) -> anyhow::Result<ProjectMutation> {
        self.mutate(|state| {
            let now = now_ms();
            let id = stable_project_id(&input.kind, &input.name, &input.source);
            let action = if state.projects.contains_key(&id) {
                "existing"
            } else {
                "created"
            }
            .to_string();
            let created_at_ms = state
                .projects
                .get(&id)
                .map(|project| project.created_at_ms)
                .unwrap_or(now);
            let project = Project {
                id: id.clone(),
                name: input.name,
                kind: input.kind,
                workspace_root: input.workspace_root,
                source: input.source,
                created_at_ms,
                updated_at_ms: now,
            };
            state.projects.insert(id.clone(), project.clone());
            if input.select {
                state.active_project_id = Some(id.clone());
                if state
                    .active_run_id
                    .as_deref()
                    .and_then(|run_id| state.runs.get(run_id))
                    .is_some_and(|run| run.project_id != id)
                {
                    state.active_run_id = None;
                }
            }
            Ok(ProjectMutation { project, action })
        })
    }

    pub fn select_project(&self, project_id: &str) -> anyhow::Result<Project> {
        self.mutate(|state| {
            let project = state
                .projects
                .get(project_id)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("unknown project: {project_id}"))?;
            state.active_project_id = Some(project_id.to_string());
            state.active_run_id = None;
            Ok(project)
        })
    }

    pub fn start_run(&self, input: RunStart) -> anyhow::Result<RunMutation> {
        self.mutate(|state| {
            let project_id = input
                .project_id
                .or_else(|| state.active_project_id.clone())
                .ok_or_else(|| {
                    anyhow::anyhow!("project_id is required when no project is selected")
                })?;
            if !state.projects.contains_key(&project_id) {
                anyhow::bail!("unknown project: {project_id}");
            }
            let now = now_ms();
            let run = Run {
                id: format!("run_{}", Uuid::new_v4().simple()),
                project_id: project_id.clone(),
                objective: input.objective,
                acceptance_criteria: input.acceptance_criteria,
                phase: RunPhase::Inspect,
                status: RunStatus::Active,
                plan: Vec::new(),
                checklist: Vec::new(),
                checkpoints: Vec::new(),
                autonomy: input.autonomy,
                counters: RunCounters::default(),
                continuation: ContinuationState::default(),
                work_remaining: true,
                next_action: "inspect the project and create a concrete plan".to_string(),
                created_at_ms: now,
                updated_at_ms: now,
                started_at_ms: now,
                completed_at_ms: None,
                cancelled_at_ms: None,
            };
            let run_id = run.id.clone();
            state.runs.insert(run_id.clone(), run.clone());
            if input.select {
                state.active_project_id = Some(project_id);
                state.active_run_id = Some(run_id);
            }
            Ok(RunMutation { run })
        })
    }

    pub fn list_runs(
        &self,
        project_id: Option<&str>,
        status: Option<RunStatus>,
    ) -> anyhow::Result<Vec<Run>> {
        let status_ref = status.as_ref();
        let runs = self
            .load()?
            .runs
            .into_values()
            .filter(|run| project_id.is_none_or(|id| run.project_id == id))
            .filter(|run| status_ref.is_none_or(|expected| run.status == *expected))
            .collect();
        Ok(runs)
    }

    pub fn get_run(&self, run_id: &str) -> anyhow::Result<Run> {
        self.load()?
            .runs
            .get(run_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown run: {run_id}"))
    }

    pub fn update_run(&self, update: RunUpdate) -> anyhow::Result<RunMutation> {
        self.mutate(|state| {
            let run_id = resolve_run_id(state, update.run_id.as_deref())?;
            let run = state
                .runs
                .get_mut(&run_id)
                .ok_or_else(|| anyhow::anyhow!("unknown run: {run_id}"))?;
            let now = now_ms();

            if let Some(next) = update.phase.as_ref() {
                validate_phase_transition(&run.phase, next)?;
            }
            if update.status.as_ref() == Some(&RunStatus::Completed)
                && run.phase != RunPhase::Verify
            {
                anyhow::bail!("run must reach verify phase before completion");
            }
            if let Some(next) = update.status {
                validate_status_transition(&run.status, &next)?;
                apply_status(run, next, now);
            }
            if run.status.is_terminal()
                && (update.phase.is_some()
                    || update.acceptance_criteria.is_some()
                    || update.plan.is_some()
                    || update.checklist.is_some()
                    || update.checkpoint.is_some()
                    || update.step_delta.is_some())
            {
                anyhow::bail!("terminal run cannot be modified");
            }
            if let Some(phase) = update.phase {
                run.phase = phase;
            }
            if let Some(acceptance_criteria) = update.acceptance_criteria {
                run.acceptance_criteria = acceptance_criteria;
            }
            if let Some(plan) = update.plan {
                validate_plan(&plan)?;
                run.plan = plan;
            }
            if let Some(checklist) = update.checklist {
                run.checklist = checklist;
            }
            if let Some(checkpoint) = update.checkpoint {
                let sequence = u64::try_from(run.checkpoints.len()).unwrap_or(0) + 1;
                run.checkpoints.push(Checkpoint {
                    sequence,
                    message: checkpoint.message,
                    created_at_ms: now,
                });
            }
            if let Some(delta) = update.step_delta {
                let next_steps = run
                    .counters
                    .steps_used
                    .checked_add(delta)
                    .ok_or_else(|| anyhow::anyhow!("step counter overflow"))?;
                if next_steps > run.autonomy.max_steps {
                    anyhow::bail!("step limit exceeded for run {}", run.id);
                }
                run.counters.steps_used = next_steps;
            }
            if let Some(work_remaining) = update.work_remaining
                && !run.status.is_terminal()
            {
                run.work_remaining = work_remaining;
            }
            if !run.status.is_terminal() {
                if let Some(next_action) = update.next_action {
                    run.next_action = next_action;
                } else {
                    run.next_action = default_next_action(run);
                }
            }
            if run.status.is_terminal() {
                run.work_remaining = false;
            }
            run.updated_at_ms = now;
            Ok(RunMutation { run: run.clone() })
        })
    }

    pub fn resume_run(&self, run_id: &str) -> anyhow::Result<RunMutation> {
        self.mutate(|state| {
            let run = state
                .runs
                .get_mut(run_id)
                .ok_or_else(|| anyhow::anyhow!("unknown run: {run_id}"))?;
            if run.status.is_terminal() {
                anyhow::bail!("terminal run cannot be resumed: {run_id}");
            }
            if run.counters.turns_used >= run.autonomy.max_turns {
                anyhow::bail!("turn limit exceeded for run {run_id}");
            }
            if run.counters.steps_used >= run.autonomy.max_steps {
                anyhow::bail!("step limit exceeded for run {run_id}");
            }
            let now = now_ms();
            let elapsed_seconds = now
                .saturating_sub(run.started_at_ms)
                .checked_div(1_000)
                .unwrap_or(0);
            if elapsed_seconds >= run.autonomy.max_runtime_seconds {
                anyhow::bail!("runtime limit exceeded for run {run_id}");
            }
            run.status = RunStatus::Active;
            run.counters.turns_used += 1;
            run.counters.runtime_seconds_used = elapsed_seconds;
            run.continuation.active_nonce = None;
            run.continuation.acquired_at_ms = None;
            run.continuation.expires_at_ms = None;
            run.next_action = default_next_action(run);
            run.updated_at_ms = now;
            state.active_project_id = Some(run.project_id.clone());
            state.active_run_id = Some(run_id.to_string());
            Ok(RunMutation { run: run.clone() })
        })
    }

    pub fn cancel_run(&self, run_id: &str) -> anyhow::Result<RunMutation> {
        self.mutate(|state| {
            let run = state
                .runs
                .get_mut(run_id)
                .ok_or_else(|| anyhow::anyhow!("unknown run: {run_id}"))?;
            if run.status == RunStatus::Completed {
                anyhow::bail!("completed run cannot be cancelled: {run_id}");
            }
            let now = now_ms();
            run.status = RunStatus::Cancelled;
            run.work_remaining = false;
            run.next_action = "run is cancelled".to_string();
            run.cancelled_at_ms = Some(now);
            run.continuation.active_nonce = None;
            run.continuation.acquired_at_ms = None;
            run.continuation.expires_at_ms = None;
            run.updated_at_ms = now;
            Ok(RunMutation { run: run.clone() })
        })
    }

    pub fn acquire_followup_lease(
        &self,
        request: FollowupLeaseRequest,
    ) -> anyhow::Result<FollowupLeaseResponse> {
        self.mutate(|state| {
            let run = state
                .runs
                .get_mut(&request.run_id)
                .ok_or_else(|| anyhow::anyhow!("unknown run: {}", request.run_id))?;
            let now = request.now_ms.unwrap_or_else(now_ms);
            let delay_ms = request.delay_ms.unwrap_or(DEFAULT_LEASE_DELAY_MS);
            let ttl_ms = request.ttl_ms.unwrap_or(DEFAULT_LEASE_TTL_MS);

            if let Some(reason) = lease_block_reason(run, now) {
                return Ok(FollowupLeaseResponse {
                    run_id: run.id.clone(),
                    granted: false,
                    duplicate: false,
                    nonce: run.continuation.active_nonce.clone(),
                    acquired_at_ms: run.continuation.acquired_at_ms,
                    expires_at_ms: run.continuation.expires_at_ms,
                    delay_ms,
                    max_turns: run.autonomy.max_turns,
                    max_runtime_seconds: run.autonomy.max_runtime_seconds,
                    max_steps: run.autonomy.max_steps,
                    reason: Some(reason),
                });
            }

            if run.continuation.is_active(now) {
                return Ok(FollowupLeaseResponse {
                    run_id: run.id.clone(),
                    granted: false,
                    duplicate: true,
                    nonce: run.continuation.active_nonce.clone(),
                    acquired_at_ms: run.continuation.acquired_at_ms,
                    expires_at_ms: run.continuation.expires_at_ms,
                    delay_ms,
                    max_turns: run.autonomy.max_turns,
                    max_runtime_seconds: run.autonomy.max_runtime_seconds,
                    max_steps: run.autonomy.max_steps,
                    reason: Some("continuation lease already active".to_string()),
                });
            }

            let nonce = request
                .requested_nonce
                .filter(|nonce| !nonce.trim().is_empty())
                .unwrap_or_else(|| format!("nonce_{}", Uuid::new_v4().simple()));
            let expires_at_ms = now.saturating_add(ttl_ms);
            run.counters.continuation_leases_issued =
                run.counters.continuation_leases_issued.saturating_add(1);
            run.continuation.active_nonce = Some(nonce.clone());
            run.continuation.acquired_at_ms = Some(now);
            run.continuation.expires_at_ms = Some(expires_at_ms);
            run.updated_at_ms = now;
            Ok(FollowupLeaseResponse {
                run_id: run.id.clone(),
                granted: true,
                duplicate: false,
                nonce: Some(nonce),
                acquired_at_ms: Some(now),
                expires_at_ms: Some(expires_at_ms),
                delay_ms,
                max_turns: run.autonomy.max_turns,
                max_runtime_seconds: run.autonomy.max_runtime_seconds,
                max_steps: run.autonomy.max_steps,
                reason: None,
            })
        })
    }

    pub fn set_legacy_plan(&self, plan: Value) -> anyhow::Result<()> {
        self.mutate(|state| {
            state.legacy_plan = plan;
            Ok(())
        })
    }

    pub fn set_legacy_todo(&self, todo: Vec<ChecklistItem>) -> anyhow::Result<()> {
        self.mutate(|state| {
            state.legacy_todo = todo;
            Ok(())
        })
    }

    fn mutate<R>(
        &self,
        f: impl FnOnce(&mut ClientState) -> anyhow::Result<R>,
    ) -> anyhow::Result<R> {
        let _lock = self.acquire_lock()?;
        let mut state = self.load()?;
        let result = f(&mut state)?;
        self.save(&state)?;
        Ok(result)
    }

    fn load(&self) -> anyhow::Result<ClientState> {
        if !self.state_file.exists() {
            return Ok(ClientState::new(self.client_id.clone()));
        }
        let data = std::fs::read_to_string(&self.state_file)
            .with_context(|| format!("failed to read {}", self.state_file.display()))?;
        let state: ClientState = serde_json::from_str(&data)
            .with_context(|| format!("state store corrupt at {}", self.state_file.display()))?;
        if state.schema_version != SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported state schema version {} at {}",
                state.schema_version,
                self.state_file.display()
            );
        }
        if state.client_id != self.client_id {
            anyhow::bail!(
                "state client id mismatch at {}: expected {}, found {}",
                self.state_file.display(),
                self.client_id,
                state.client_id
            );
        }
        Ok(state)
    }

    fn save(&self, state: &ClientState) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.state_dir)
            .with_context(|| format!("failed to create {}", self.state_dir.display()))?;
        let tmp = self.state_dir.join(format!(
            "{STATE_FILE_NAME}.tmp.{}.{}",
            std::process::id(),
            Uuid::new_v4().simple()
        ));
        let data =
            serde_json::to_vec_pretty(state).context("failed to serialize lifecycle state")?;
        {
            let mut file = File::create(&tmp)
                .with_context(|| format!("failed to create {}", tmp.display()))?;
            file.write_all(&data)
                .with_context(|| format!("failed to write {}", tmp.display()))?;
            file.write_all(b"\n")
                .with_context(|| format!("failed to finalize {}", tmp.display()))?;
            file.sync_all()
                .with_context(|| format!("failed to sync {}", tmp.display()))?;
        }
        std::fs::rename(&tmp, &self.state_file).with_context(|| {
            format!(
                "failed to atomically replace {} with {}",
                self.state_file.display(),
                tmp.display()
            )
        })?;
        sync_directory(&self.state_dir)?;
        Ok(())
    }

    fn acquire_lock(&self) -> anyhow::Result<StoreLock> {
        std::fs::create_dir_all(&self.state_dir)
            .with_context(|| format!("failed to create {}", self.state_dir.display()))?;
        for _ in 0..LOCK_RETRY_COUNT {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&self.lock_file)
            {
                Ok(mut file) => {
                    writeln!(
                        file,
                        "pid={} acquired_at_ms={}",
                        std::process::id(),
                        now_ms()
                    )
                    .ok();
                    file.sync_all().ok();
                    return Ok(StoreLock {
                        path: self.lock_file.clone(),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    remove_stale_lock(&self.lock_file)?;
                    std::thread::sleep(Duration::from_millis(LOCK_RETRY_DELAY_MS));
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to acquire {}", self.lock_file.display())
                    });
                }
            }
        }
        anyhow::bail!("timed out acquiring {}", self.lock_file.display());
    }
}

impl ClientState {
    fn new(client_id: String) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            client_id,
            active_project_id: None,
            active_run_id: None,
            projects: BTreeMap::new(),
            runs: BTreeMap::new(),
            legacy_plan: default_legacy_plan(),
            legacy_todo: Vec::new(),
        }
    }
}

impl RunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }

    pub fn is_non_continuable(&self) -> bool {
        !matches!(self, Self::Active)
    }
}

impl ContinuationState {
    fn is_active(&self, now_ms: u64) -> bool {
        self.active_nonce.is_some() && self.expires_at_ms.is_some_and(|expires| expires > now_ms)
    }
}

pub fn redacted_git_source(source: &str) -> anyhow::Result<ProjectSource> {
    let url_str = source.strip_prefix("git+").unwrap_or(source);
    let mut url = url_str.parse::<url::Url>().context("invalid git URL")?;
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    Ok(ProjectSource::Git {
        host: url.host_str().map(ToString::to_string),
        path: url.path().to_string(),
        url: url.to_string(),
    })
}

pub fn workspace_source(path: impl Into<PathBuf>) -> ProjectSource {
    ProjectSource::Workspace {
        registered_path: path.into(),
    }
}

pub fn run_metadata(run: &Run, now: u64) -> Value {
    json!({
        "run_id": &run.id,
        "project_id": &run.project_id,
        "phase": &run.phase,
        "status": &run.status,
        "work_remaining": run.work_remaining && !run.status.is_terminal(),
        "next_action": &run.next_action,
        "limits": {
            "max_turns": run.autonomy.max_turns,
            "turns_used": run.counters.turns_used,
            "turns_remaining": run.autonomy.max_turns.saturating_sub(run.counters.turns_used),
            "max_runtime_seconds": run.autonomy.max_runtime_seconds,
            "runtime_seconds_used": runtime_seconds_used(run, now),
            "runtime_seconds_remaining": run.autonomy.max_runtime_seconds.saturating_sub(runtime_seconds_used(run, now)),
            "max_steps": run.autonomy.max_steps,
            "steps_used": run.counters.steps_used,
            "steps_remaining": run.autonomy.max_steps.saturating_sub(run.counters.steps_used),
            "allow_local_commands": run.autonomy.allow_local_commands,
            "allow_file_edits": run.autonomy.allow_file_edits,
            "allow_git_commits": run.autonomy.allow_git_commits,
        },
        "lease": {
            "active": run.continuation.is_active(now),
            "nonce": &run.continuation.active_nonce,
            "acquired_at_ms": run.continuation.acquired_at_ms,
            "expires_at_ms": run.continuation.expires_at_ms,
            "counter": run.counters.continuation_leases_issued,
        }
    })
}

pub fn validate_plan(plan: &[PlanItem]) -> anyhow::Result<()> {
    let in_progress = plan
        .iter()
        .filter(|item| item.status == PlanStatus::InProgress)
        .count();
    if in_progress > 1 {
        anyhow::bail!("at most one plan item may be in progress");
    }
    Ok(())
}

fn apply_status(run: &mut Run, next: RunStatus, now: u64) {
    run.status = next;
    match run.status {
        RunStatus::Completed => {
            run.completed_at_ms = Some(now);
            run.work_remaining = false;
            run.next_action = "run is complete".to_string();
            clear_lease(run);
        }
        RunStatus::Cancelled => {
            run.cancelled_at_ms = Some(now);
            run.work_remaining = false;
            run.next_action = "run is cancelled".to_string();
            clear_lease(run);
        }
        RunStatus::Paused => {
            run.next_action = "run is paused until explicitly resumed".to_string();
            clear_lease(run);
        }
        RunStatus::Blocked => {
            run.next_action = "resolve the blocker before continuing".to_string();
            clear_lease(run);
        }
        RunStatus::AwaitingApproval => {
            run.next_action = "wait for explicit approval before continuing".to_string();
            clear_lease(run);
        }
        RunStatus::Active => {}
    }
}

fn clear_lease(run: &mut Run) {
    run.continuation.active_nonce = None;
    run.continuation.acquired_at_ms = None;
    run.continuation.expires_at_ms = None;
}

fn validate_phase_transition(current: &RunPhase, next: &RunPhase) -> anyhow::Result<()> {
    if current == next {
        return Ok(());
    }
    let allowed = matches!(
        (current, next),
        (RunPhase::Inspect, RunPhase::Plan)
            | (RunPhase::Plan, RunPhase::Execute)
            | (RunPhase::Execute, RunPhase::Verify)
            | (RunPhase::Verify, RunPhase::Execute)
    );
    if allowed {
        Ok(())
    } else {
        anyhow::bail!("invalid run phase transition: {current:?} -> {next:?}");
    }
}

fn validate_status_transition(current: &RunStatus, next: &RunStatus) -> anyhow::Result<()> {
    if current == next {
        return Ok(());
    }
    let allowed = match current {
        RunStatus::Active => matches!(
            next,
            RunStatus::Paused
                | RunStatus::Blocked
                | RunStatus::AwaitingApproval
                | RunStatus::Completed
                | RunStatus::Cancelled
        ),
        RunStatus::Paused | RunStatus::Blocked | RunStatus::AwaitingApproval => {
            matches!(next, RunStatus::Active | RunStatus::Cancelled)
        }
        RunStatus::Completed | RunStatus::Cancelled => false,
    };
    if allowed {
        Ok(())
    } else {
        anyhow::bail!("invalid run status transition: {current:?} -> {next:?}");
    }
}

fn lease_block_reason(run: &Run, now: u64) -> Option<String> {
    if run.status.is_non_continuable() {
        return Some(format!("run status is {}", run.status.as_str()));
    }
    if !run.work_remaining {
        return Some("work_remaining is false".to_string());
    }
    if run.counters.turns_used >= run.autonomy.max_turns {
        return Some("turn limit reached".to_string());
    }
    if run.counters.steps_used >= run.autonomy.max_steps {
        return Some("step limit reached".to_string());
    }
    if runtime_seconds_used(run, now) >= run.autonomy.max_runtime_seconds {
        return Some("runtime limit reached".to_string());
    }
    None
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Blocked => "blocked",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }
}

fn default_next_action(run: &Run) -> String {
    match run.status {
        RunStatus::Paused => "run is paused until explicitly resumed".to_string(),
        RunStatus::Blocked => "resolve the blocker before continuing".to_string(),
        RunStatus::AwaitingApproval => "wait for explicit approval before continuing".to_string(),
        RunStatus::Completed => "run is complete".to_string(),
        RunStatus::Cancelled => "run is cancelled".to_string(),
        RunStatus::Active => match run.phase {
            RunPhase::Inspect => "inspect the project and gather facts".to_string(),
            RunPhase::Plan => "create or update the plan and checklist".to_string(),
            RunPhase::Execute => "execute the next checklist item".to_string(),
            RunPhase::Verify => "verify all acceptance criteria".to_string(),
        },
    }
}

fn resolve_run_id(state: &ClientState, explicit: Option<&str>) -> anyhow::Result<String> {
    explicit
        .map(ToString::to_string)
        .or_else(|| state.active_run_id.clone())
        .ok_or_else(|| anyhow::anyhow!("run_id is required when no run is selected"))
}

fn stable_project_id(kind: &ProjectKind, name: &str, source: &ProjectSource) -> String {
    let kind_label = match kind {
        ProjectKind::Repo => "repo",
        ProjectKind::Workspace => "workspace",
        ProjectKind::Scratch => "scratch",
    };
    let identity = match source {
        ProjectSource::Scratch => format!("scratch:{name}"),
        ProjectSource::Git { url, .. } => format!("git:{url}"),
        ProjectSource::Workspace { registered_path } => {
            format!("workspace:{}", registered_path.display())
        }
    };
    format!(
        "proj_{kind_label}_{}_{:016x}",
        sanitize_id_lossy(name),
        fnv1a64(identity.as_bytes())
    )
}

fn sanitize_id_component(value: &str) -> anyhow::Result<String> {
    let sanitized = sanitize_id_lossy(value);
    if sanitized.is_empty() {
        anyhow::bail!("identifier must not be empty");
    }
    Ok(sanitized)
}

fn sanitize_id_lossy(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn now_ms() -> u64 {
    let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return 0;
    };
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

pub fn current_time_ms() -> u64 {
    now_ms()
}

fn runtime_seconds_used(run: &Run, now: u64) -> u64 {
    run.counters
        .runtime_seconds_used
        .max(now.saturating_sub(run.started_at_ms) / 1_000)
}

fn default_legacy_plan() -> Value {
    json!({"plan": []})
}

fn remove_stale_lock(lock_file: &Path) -> anyhow::Result<()> {
    let Ok(metadata) = std::fs::metadata(lock_file) else {
        return Ok(());
    };
    let Ok(modified) = metadata.modified() else {
        return Ok(());
    };
    let Ok(age) = SystemTime::now().duration_since(modified) else {
        return Ok(());
    };
    if u64::try_from(age.as_millis()).unwrap_or(u64::MAX) > LOCK_STALE_AFTER_MS {
        let _ = std::fs::remove_file(lock_file);
    }
    Ok(())
}

fn sync_directory(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|dir| dir.sync_all())
            .with_context(|| format!("failed to sync directory {}", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

struct StoreLock {
    path: PathBuf,
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store(root: &tempfile::TempDir, client_id: &str) -> LifecycleStore {
        LifecycleStore::open(root.path(), client_id).expect("store")
    }

    #[test]
    fn projects_and_runs_survive_restart_and_are_client_isolated() {
        let root = tempfile::tempdir().expect("root");
        let project_root = root.path().join("clients/alice/sandboxes/demo");
        std::fs::create_dir_all(&project_root).expect("project root");

        let first = store(&root, "alice");
        let project = first
            .upsert_project(ProjectUpsert {
                kind: ProjectKind::Scratch,
                name: "demo".to_string(),
                workspace_root: project_root.clone(),
                source: ProjectSource::Scratch,
                select: true,
            })
            .expect("project");
        let run = first
            .start_run(RunStart {
                project_id: Some(project.project.id.clone()),
                objective: "ship persistence".to_string(),
                acceptance_criteria: vec!["state reloads".to_string()],
                autonomy: AutonomyEnvelope::default(),
                select: true,
            })
            .expect("run");

        let second = store(&root, "alice");
        assert_eq!(
            second.get_project(&project.project.id).unwrap().id,
            project.project.id
        );
        assert_eq!(
            second.get_run(&run.run.id).unwrap().objective,
            "ship persistence"
        );
        assert_eq!(second.snapshot().unwrap().active_run_id, Some(run.run.id));

        let other = store(&root, "bob");
        assert!(other.list_projects().unwrap().is_empty());
        assert!(other.list_runs(None, None).unwrap().is_empty());
    }

    #[test]
    fn project_ids_are_stable_for_scratch_repo_and_registered_workspace_without_credentials() {
        let root = tempfile::tempdir().expect("root");
        let store = store(&root, "default");
        let scratch_root = root.path().join("clients/default/sandboxes/demo");
        let workspace_root = root.path().join("clients/default/registered/repo");
        std::fs::create_dir_all(&scratch_root).unwrap();
        std::fs::create_dir_all(&workspace_root).unwrap();

        let scratch_a = store
            .upsert_project(ProjectUpsert {
                kind: ProjectKind::Scratch,
                name: "demo".to_string(),
                workspace_root: scratch_root.clone(),
                source: ProjectSource::Scratch,
                select: false,
            })
            .unwrap();
        let scratch_b = store
            .upsert_project(ProjectUpsert {
                kind: ProjectKind::Scratch,
                name: "demo".to_string(),
                workspace_root: scratch_root,
                source: ProjectSource::Scratch,
                select: false,
            })
            .unwrap();
        assert_eq!(scratch_a.project.id, scratch_b.project.id);

        let repo_source =
            redacted_git_source("https://user:secret@example.test/acme/demo.git?token=hidden")
                .unwrap();
        let repo = store
            .upsert_project(ProjectUpsert {
                kind: ProjectKind::Repo,
                name: "demo".to_string(),
                workspace_root: root.path().join("clients/default/repos/demo"),
                source: repo_source,
                select: false,
            })
            .unwrap();
        let serialized = serde_json::to_string(&store.snapshot().unwrap()).unwrap();
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("hidden"));
        assert!(serialized.contains("https://example.test/acme/demo.git"));

        let workspace_a = store
            .upsert_project(ProjectUpsert {
                kind: ProjectKind::Workspace,
                name: "registered".to_string(),
                workspace_root: workspace_root.clone(),
                source: workspace_source(workspace_root.clone()),
                select: false,
            })
            .unwrap();
        let workspace_b = store
            .upsert_project(ProjectUpsert {
                kind: ProjectKind::Workspace,
                name: "registered".to_string(),
                workspace_root,
                source: workspace_source(workspace_a.project.workspace_root.clone()),
                select: false,
            })
            .unwrap();
        assert_eq!(workspace_a.project.id, workspace_b.project.id);
        assert!(repo.project.id.starts_with("proj_repo_demo_"));
    }

    #[test]
    fn invalid_status_transitions_are_rejected() {
        let root = tempfile::tempdir().expect("root");
        let project_root = root.path().join("clients/default/sandboxes/demo");
        std::fs::create_dir_all(&project_root).expect("project root");
        let store = store(&root, "default");
        let project = store
            .upsert_project(ProjectUpsert {
                kind: ProjectKind::Scratch,
                name: "demo".to_string(),
                workspace_root: project_root,
                source: ProjectSource::Scratch,
                select: true,
            })
            .unwrap();
        let run = store
            .start_run(RunStart {
                project_id: Some(project.project.id),
                objective: "finish".to_string(),
                acceptance_criteria: vec![],
                autonomy: AutonomyEnvelope::default(),
                select: true,
            })
            .unwrap();
        store
            .update_run(RunUpdate {
                run_id: Some(run.run.id.clone()),
                phase: Some(RunPhase::Plan),
                ..RunUpdate::default()
            })
            .unwrap();
        store
            .update_run(RunUpdate {
                run_id: Some(run.run.id.clone()),
                phase: Some(RunPhase::Execute),
                ..RunUpdate::default()
            })
            .unwrap();
        store
            .update_run(RunUpdate {
                run_id: Some(run.run.id.clone()),
                phase: Some(RunPhase::Verify),
                ..RunUpdate::default()
            })
            .unwrap();
        store
            .update_run(RunUpdate {
                run_id: Some(run.run.id.clone()),
                status: Some(RunStatus::Completed),
                ..RunUpdate::default()
            })
            .unwrap();

        let err = store
            .update_run(RunUpdate {
                run_id: Some(run.run.id),
                status: Some(RunStatus::Active),
                ..RunUpdate::default()
            })
            .expect_err("terminal run cannot become active");
        assert!(err.to_string().contains("invalid run status transition"));
    }

    #[test]
    fn phase_lifecycle_rejects_skips_and_requires_verify_before_completion() {
        let root = tempfile::tempdir().expect("root");
        let project_root = root.path().join("clients/default/sandboxes/demo");
        std::fs::create_dir_all(&project_root).expect("project root");
        let store = store(&root, "default");
        let project = store
            .upsert_project(ProjectUpsert {
                kind: ProjectKind::Scratch,
                name: "demo".to_string(),
                workspace_root: project_root,
                source: ProjectSource::Scratch,
                select: true,
            })
            .unwrap();
        let run = store
            .start_run(RunStart {
                project_id: Some(project.project.id),
                objective: "follow the lifecycle".to_string(),
                acceptance_criteria: vec!["verified".to_string()],
                autonomy: AutonomyEnvelope::default(),
                select: true,
            })
            .unwrap();

        let skipped = store
            .update_run(RunUpdate {
                run_id: Some(run.run.id.clone()),
                phase: Some(RunPhase::Execute),
                ..RunUpdate::default()
            })
            .expect_err("inspect cannot skip directly to execute");
        assert!(skipped.to_string().contains("invalid run phase transition"));

        store
            .update_run(RunUpdate {
                run_id: Some(run.run.id.clone()),
                phase: Some(RunPhase::Plan),
                ..RunUpdate::default()
            })
            .unwrap();
        store
            .update_run(RunUpdate {
                run_id: Some(run.run.id.clone()),
                phase: Some(RunPhase::Execute),
                ..RunUpdate::default()
            })
            .unwrap();

        let premature = store
            .update_run(RunUpdate {
                run_id: Some(run.run.id.clone()),
                status: Some(RunStatus::Completed),
                ..RunUpdate::default()
            })
            .expect_err("completion requires verify phase");
        assert!(premature.to_string().contains("verify phase"));

        store
            .update_run(RunUpdate {
                run_id: Some(run.run.id.clone()),
                phase: Some(RunPhase::Verify),
                ..RunUpdate::default()
            })
            .unwrap();
        let completed = store
            .update_run(RunUpdate {
                run_id: Some(run.run.id),
                status: Some(RunStatus::Completed),
                ..RunUpdate::default()
            })
            .unwrap();
        assert_eq!(completed.run.status, RunStatus::Completed);
    }

    #[test]
    fn active_run_step_gate_pauses_at_step_and_runtime_limits() {
        let root = tempfile::tempdir().expect("root");
        let project_root = root.path().join("clients/default/sandboxes/demo");
        std::fs::create_dir_all(&project_root).expect("project root");
        let store = store(&root, "default");
        let project = store
            .upsert_project(ProjectUpsert {
                kind: ProjectKind::Scratch,
                name: "demo".to_string(),
                workspace_root: project_root,
                source: ProjectSource::Scratch,
                select: true,
            })
            .unwrap();
        let run = store
            .start_run(RunStart {
                project_id: Some(project.project.id),
                objective: "bounded work".to_string(),
                acceptance_criteria: vec![],
                autonomy: AutonomyEnvelope {
                    max_steps: 1,
                    max_runtime_seconds: 3_600,
                    ..AutonomyEnvelope::default()
                },
                select: true,
            })
            .unwrap();

        assert_eq!(store.consume_active_step().unwrap(), None);
        let blocked = store
            .consume_active_step()
            .unwrap()
            .expect("second step must be blocked");
        assert!(blocked.contains("step limit"));
        let paused = store.get_run(&run.run.id).unwrap();
        assert_eq!(paused.status, RunStatus::Paused);
        assert_eq!(paused.counters.steps_used, 1);
        let resume_error = store
            .resume_run(&run.run.id)
            .expect_err("step-limited run cannot be resumed");
        assert!(resume_error.to_string().contains("step limit"));
    }

    #[test]
    fn continuation_leases_are_duplicate_safe_expire_and_respect_limits() {
        let root = tempfile::tempdir().expect("root");
        let project_root = root.path().join("clients/default/sandboxes/demo");
        std::fs::create_dir_all(&project_root).expect("project root");
        let store = store(&root, "default");
        let project = store
            .upsert_project(ProjectUpsert {
                kind: ProjectKind::Scratch,
                name: "demo".to_string(),
                workspace_root: project_root,
                source: ProjectSource::Scratch,
                select: true,
            })
            .unwrap();
        let run = store
            .start_run(RunStart {
                project_id: Some(project.project.id),
                objective: "continue".to_string(),
                acceptance_criteria: vec![],
                autonomy: AutonomyEnvelope {
                    max_turns: 1,
                    max_runtime_seconds: 3600,
                    max_steps: 10,
                    allow_local_commands: true,
                    allow_file_edits: true,
                    allow_git_commits: false,
                },
                select: true,
            })
            .unwrap();

        let first = store
            .acquire_followup_lease(FollowupLeaseRequest {
                run_id: run.run.id.clone(),
                requested_nonce: Some("nonce-1".to_string()),
                now_ms: Some(1_000),
                ttl_ms: Some(500),
                delay_ms: Some(25),
            })
            .unwrap();
        assert!(first.granted);
        assert_eq!(first.nonce.as_deref(), Some("nonce-1"));

        let duplicate = store
            .acquire_followup_lease(FollowupLeaseRequest {
                run_id: run.run.id.clone(),
                requested_nonce: Some("nonce-2".to_string()),
                now_ms: Some(1_100),
                ttl_ms: Some(500),
                delay_ms: Some(25),
            })
            .unwrap();
        assert!(!duplicate.granted);
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.nonce.as_deref(), Some("nonce-1"));

        let expired = store
            .acquire_followup_lease(FollowupLeaseRequest {
                run_id: run.run.id.clone(),
                requested_nonce: Some("nonce-3".to_string()),
                now_ms: Some(1_600),
                ttl_ms: Some(500),
                delay_ms: Some(25),
            })
            .unwrap();
        assert!(expired.granted);
        assert_eq!(expired.nonce.as_deref(), Some("nonce-3"));

        store.resume_run(&run.run.id).unwrap();
        let limited = store
            .acquire_followup_lease(FollowupLeaseRequest {
                run_id: run.run.id,
                requested_nonce: Some("nonce-4".to_string()),
                now_ms: Some(1_700),
                ttl_ms: Some(500),
                delay_ms: Some(25),
            })
            .unwrap();
        assert!(!limited.granted);
        assert!(limited.reason.unwrap().contains("turn limit"));
    }

    #[test]
    fn legacy_plan_and_todo_fallback_persist_without_active_run() {
        let root = tempfile::tempdir().expect("root");
        let first = store(&root, "default");
        first
            .set_legacy_plan(json!({"plan":[{"step":"legacy","status":"in_progress"}]}))
            .unwrap();
        first
            .set_legacy_todo(vec![ChecklistItem {
                id: "t1".to_string(),
                description: "legacy item".to_string(),
                status: ChecklistStatus::Pending,
            }])
            .unwrap();

        let restarted = store(&root, "default");
        let snapshot = restarted.snapshot().unwrap();
        assert_eq!(snapshot.legacy_plan["plan"][0]["step"], "legacy");
        assert_eq!(snapshot.legacy_todo[0].description, "legacy item");
    }

    #[test]
    fn corrupted_state_is_reported_without_overwriting_file() {
        let root = tempfile::tempdir().expect("root");
        let state_dir = root.path().join("clients/default/.chatcodex");
        std::fs::create_dir_all(&state_dir).expect("state dir");
        let state_file = state_dir.join("state.json");
        std::fs::write(&state_file, "{not-json").expect("corrupt state");

        let store = store(&root, "default");
        let err = store.snapshot().expect_err("corrupt state should fail");
        assert!(err.to_string().contains("corrupt"));
        assert_eq!(std::fs::read_to_string(state_file).unwrap(), "{not-json");
    }
}
