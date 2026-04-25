//! End-to-end integration tests for the ChatCodex daemon handler layer.
//!
//! What is REAL: handler dispatch, SQLite persistence, filesystem mutations,
//!               approval chain, command policy, worker status transitions.
//! What is STUBBED: LLM HTTP endpoint (provider calls fail with connection refused).
//!
//! ```sh
//! cargo test -p deterministic-daemon --test hybrid_e2e
//! ```

use deterministic_core::HybridConfig;
use deterministic_core::HybridProviderKind;
use deterministic_core::HybridProviderProfile;
use deterministic_daemon::handlers::dispatch;
use deterministic_daemon::persistence::Store;
use deterministic_protocol::{Method, PendingApproval, RunState};

// ---------------------------------------------------------------------------
// Test infrastructure helpers
// ---------------------------------------------------------------------------

fn make_run_state(run_id: &str, workspace_id: &str) -> RunState {
    RunState {
        run_id: run_id.into(),
        workspace_id: workspace_id.into(),
        user_goal: "test goal".into(),
        status: "active".into(),
        plan: vec![],
        current_step: 0,
        completed_steps: vec![],
        pending_steps: vec![],
        last_action: None,
        last_observation: None,
        recommended_next_action: None,
        recommended_tool: None,
        latest_diff_summary: None,
        latest_test_result: None,
        focus_paths: vec![],
        warnings: vec![],
        retryable_action: None,
        policy_profile: deterministic_protocol::RunPolicy::default(),
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
        blocked_by_run_ids: vec![],
        effort: None,
        harness_mode: deterministic_protocol::HarnessMode::Deterministic,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn rpc_result(resp: Result<(serde_json::Value, Option<RunState>), anyhow::Error>) -> serde_json::Value {
    resp.expect("handler returned error").0
}

fn rpc_err(resp: Result<(serde_json::Value, Option<RunState>), anyhow::Error>) -> String {
    resp.expect_err("handler did not return error").to_string()
}

fn failing_hybrid_config() -> HybridConfig {
    HybridConfig {
        enabled: true,
        default_profile: Some(HybridProviderProfile {
            profile_id: "fail".to_string(),
            kind: HybridProviderKind::OpenAiCompatible,
            base_url: "http://127.0.0.1:19997".to_string(),
            api_key_env: None,
            model: "mock".to_string(),
            timeout_seconds: 1,
            temperature: 0.2,
            max_output_tokens: 2048,
        }),
    }
}

fn mock_hybrid_config() -> HybridConfig {
    HybridConfig {
        enabled: true,
        default_profile: Some(HybridProviderProfile {
            profile_id: "mock".to_string(),
            kind: HybridProviderKind::OpenAiCompatible,
            base_url: "http://127.0.0.1:19999".to_string(),
            api_key_env: None,
            model: "mock".to_string(),
            timeout_seconds: 2,
            temperature: 0.2,
            max_output_tokens: 2048,
        }),
    }
}

fn no_hybrid_config() -> HybridConfig {
    HybridConfig { enabled: false, default_profile: None }
}

/// Holds a TempDir + store + workspace path so each test captures them on the stack.
struct TestEnv {
    store: Store,
    #[allow(dead_code)]
    ws: tempfile::TempDir,
    workspace_path: std::path::PathBuf,
}

impl TestEnv {
    fn new() -> Self {
        let store = Store::open_in_memory().unwrap();
        let ws = tempfile::TempDir::new().unwrap();
        let workspace_path = ws.path().to_path_buf();
        std::fs::write(ws.path().join("Cargo.toml"), "[package]\nname = \"test\"\n").unwrap();
        std::fs::create_dir_all(ws.path().join("src")).unwrap();
        TestEnv { store, ws, workspace_path }
    }

    fn make_run(&self, run_id: &str) -> RunState {
        make_run_state(run_id, self.workspace_path.to_str().unwrap())
    }

    fn make_hybrid_run(&self, run_id: &str) -> RunState {
        let mut run = self.make_run(run_id);
        run.harness_mode = deterministic_protocol::HarnessMode::Hybrid;
        run
    }
}

// ---------------------------------------------------------------------------
// Test 1: Hybrid worker lifecycle — prepare → start (fails) → get → cancel
// ---------------------------------------------------------------------------

#[test]
fn hybrid_worker_lifecycle() {
    let env = TestEnv::new();
    env.store.save_run(&env.make_hybrid_run("r1")).unwrap();

    // Step 1: prepare first worker (needs hybrid config with valid profile for profile_id)
    let (val, _) = dispatch(
        Method::HybridWorkerPrepare,
        serde_json::json!({
            "runId": "r1",
            "taskGoal": "add helper function",
            "contextFiles": []
        }),
        &env.store,
        &mock_hybrid_config(),
    )
    .unwrap();
    let worker_id: &str = val.get("workerRunId").unwrap().as_str().unwrap();
    assert_eq!(val.get("status").unwrap().as_str().unwrap(), "prepared");
    println!("[1] worker prepared: {worker_id}");

    // Step 2: get worker confirms prepared status (start requires tokio runtime so skip it)
    let (wg_val, _) = dispatch(
        Method::HybridWorkerGet,
        serde_json::json!({ "workerRunId": worker_id }),
        &env.store,
        &no_hybrid_config(),
    )
    .unwrap();
    assert_eq!(wg_val.get("status").unwrap().as_str().unwrap(), "prepared");
    assert!(wg_val.get("proposedEdits").is_none() || wg_val.get("proposedEdits").unwrap().is_null());
    println!("[2] get confirms prepared status + no proposed_edits");

    // Step 3: cancel the worker
    let (cancel_val, _) = dispatch(
        Method::HybridWorkerCancel,
        serde_json::json!({
            "workerRunId": worker_id,
            "reason": "e2e test cancellation"
        }),
        &env.store,
        &no_hybrid_config(),
    )
    .unwrap();
    assert_eq!(cancel_val.get("status").unwrap().as_str().unwrap(), "cancelled");
    println!("[3] cancelled worker: {worker_id}");

    // Step 4: prepare and cancel a second worker
    let (wp2_val, _) = dispatch(
        Method::HybridWorkerPrepare,
        serde_json::json!({
            "runId": "r1",
            "taskGoal": "another task",
            "contextFiles": []
        }),
        &env.store,
        &mock_hybrid_config(),
    )
    .unwrap();
    let worker_id2: &str = wp2_val.get("workerRunId").unwrap().as_str().unwrap();

    let (cancel2_val, _) = dispatch(
        Method::HybridWorkerCancel,
        serde_json::json!({
            "workerRunId": worker_id2,
            "reason": "e2e test"
        }),
        &env.store,
        &no_hybrid_config(),
    )
    .unwrap();
    assert_eq!(cancel2_val.get("status").unwrap().as_str().unwrap(), "cancelled");
    println!("[4] cancelled second worker: {worker_id2}");

    // Step 5: list workers for parent run
    let (list_val, _) = dispatch(
        Method::HybridWorkerList,
        serde_json::json!({ "runId": "r1", "status": null }),
        &env.store,
        &no_hybrid_config(),
    )
    .unwrap();
    assert_eq!(
        list_val.get("workerRuns").unwrap().as_array().unwrap().len(),
        2,
        "list should show 2 workers"
    );
    println!("[5] list shows 2 workers");
}

// ---------------------------------------------------------------------------
// Test 2: Hybrid patch approval chain
// ---------------------------------------------------------------------------

#[test]
fn hybrid_patch_approval_chain() {
    let env = TestEnv::new();
    std::fs::write(
        env.workspace_path.join("src/lib.rs"),
        "// starter\nfn original() {}\n",
    )
    .unwrap();

    env.store.save_run(&env.make_run("r1")).unwrap();

    // Simulate a succeeded worker with proposed_edits
    let worker = deterministic_protocol::HybridWorkerRun {
        worker_run_id: "w1".into(),
        parent_run_id: "r1".into(),
        status: deterministic_protocol::HybridWorkerStatus::Succeeded,
        provider_profile_id: "mock".into(),
        task_goal: "improve".into(),
        focus_paths: vec![],
        prompt: "mock".into(),
        proposed_edits: Some(vec![deterministic_protocol::PatchEdit {
            path: "src/lib.rs".into(),
            operation: "replace".into(),
            start_line: Some(1),
            end_line: Some(1),
            old_text: Some("// starter".into()),
            new_text: "// modified by worker".into(),
            anchor_text: None,
            reason: None,
        }]),
        summary: Some("1 edit".into()),
        failure_message: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        started_at: Some(chrono::Utc::now().to_rfc3339()),
        completed_at: Some(chrono::Utc::now().to_rfc3339()),
        cancel_requested: false,
        context_files: vec![],
    };
    env.store.save_worker_run(&worker).unwrap();

    // Step 1: hybrid.patch.submit → creates pending approval + retryable_action
    let (sub_val, state) = dispatch(
        Method::HybridPatchSubmit,
        serde_json::json!({
            "runId": "r1",
            "workerRunId": "w1",
            "patchIndices": [0]
        }),
        &env.store,
        &no_hybrid_config(),
    )
    .unwrap();
    let approval_id: &str = sub_val.get("approvalIds").unwrap().as_array().unwrap()[0]
        .as_str()
        .unwrap();
    println!("[1] approval created: {approval_id}");

    // Verify approval stored in DB
    let stored = env.store.get_approval(approval_id).unwrap().unwrap();
    assert_eq!(stored.status, "pending");
    println!("[1b] approval stored: status=pending");

    // Verify retryable_action has skip_policy=true
    let updated = state.as_ref().unwrap();
    let retryable = updated.retryable_action.as_ref().unwrap();
    let params: deterministic_protocol::PatchApplyParams =
        serde_json::from_str(retryable.payload.as_ref().unwrap()).unwrap();
    assert!(params.skip_policy, "retryable_action must have skip_policy=true");
    println!("[1c] retryable_action has skip_policy=true");

    // Step 2: approve → recommended_tool = patch.apply
    let (apr_val, _) = dispatch(
        Method::ApprovalResolve,
        serde_json::json!({
            "runId": "r1",
            "approvalId": approval_id,
            "decision": "approve",
            "reason": "looks correct"
        }),
        &env.store,
        &no_hybrid_config(),
    )
    .unwrap();
    assert_eq!(
        apr_val.get("recommendedTool").unwrap().as_str().unwrap(),
        "patch.apply"
    );
    println!("[2] approved, recommended_tool=patch.apply");

    // Step 3: apply patch with skip_policy=true (bypasses policy)
    let (patch_val, _) = dispatch(
        Method::PatchApply,
        serde_json::json!({
            "runId": "r1",
            "skipPolicy": true,
            "edits": [{
                "path": "src/lib.rs",
                "operation": "replace",
                "startLine": 1,
                "endLine": 1,
                "oldText": "// starter",
                "newText": "// modified by worker"
            }]
        }),
        &env.store,
        &no_hybrid_config(),
    )
    .unwrap();
    assert_eq!(patch_val.get("changedFiles").unwrap().as_array().unwrap().len(), 1);
    println!("[3] applied count=1");

    // Step 4: verify filesystem was mutated
    let content = std::fs::read_to_string(env.workspace_path.join("src/lib.rs")).unwrap();
    assert!(content.contains("modified by worker"));
    assert!(!content.contains("// starter"));
    println!("[4] filesystem verified");

    // Step 5: worker status remains succeeded
    let (wg_val, _) = dispatch(
        Method::HybridWorkerGet,
        serde_json::json!({ "workerRunId": "w1" }),
        &env.store,
        &no_hybrid_config(),
    )
    .unwrap();
    assert_eq!(wg_val.get("status").unwrap().as_str().unwrap(), "succeeded");
    println!("[5] final status=succeeded");
}

// ---------------------------------------------------------------------------
// Test 3: Harness mode apply_patch roundtrip
// ---------------------------------------------------------------------------

#[test]
fn harness_mode_apply_patch() {
    let env = TestEnv::new();
    std::fs::write(
        env.workspace_path.join("src/main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();

    env.store.save_run(&env.make_run("r1")).unwrap();

    let result: serde_json::Value = rpc_result(dispatch(
        Method::PatchApply,
        serde_json::json!({
            "runId": "r1",
            "edits": [{
                "path": "src/main.rs",
                "operation": "replace",
                "startLine": 1,
                "endLine": 2,
                "oldText": "fn main() {\n    println!(\"hello\");\n}",
                "newText": "fn main() {\n    println!(\"goodbye\");\n}"
            }]
        }),
        &env.store,
        &no_hybrid_config(),
    ));

    assert_eq!(result.get("changedFiles").unwrap().as_array().unwrap().len(), 1);
    let content = std::fs::read_to_string(env.workspace_path.join("src/main.rs")).unwrap();
    assert!(content.contains("goodbye"));
    assert!(!content.contains("hello"));
    println!("[Harness] patch applied: 'goodbye' present, 'hello' gone");
}

// ---------------------------------------------------------------------------
// Test 4: run_tests with whitelisted cargo scope passes command policy
// ---------------------------------------------------------------------------

#[test]
fn harness_run_tests_whitelisted_cargo() {
    let env = TestEnv::new();
    env.store.save_run(&env.make_run("r1")).unwrap();

    let result: serde_json::Value = rpc_result(dispatch(
        Method::TestsRun,
        serde_json::json!({
            "runId": "r1",
            "scope": "cargo",
            "reason": "Phase 12 e2e",
            "target": null
        }),
        &env.store,
        &no_hybrid_config(),
    ));

    let summary: &str = result.get("summary").unwrap().as_str().unwrap();
    assert!(
        !summary.contains("not whitelisted"),
        "cargo scope must pass command policy; got: {summary}"
    );
    println!("[Tests] whitelisted cargo passed policy: {}", summary);
}

// ---------------------------------------------------------------------------
// Test 5: run_tests with shell metachar scope rejected at command policy
// ---------------------------------------------------------------------------

#[test]
fn harness_run_tests_rejects_shell_metachar_scope() {
    let env = TestEnv::new();
    env.store.save_run(&env.make_run("r1")).unwrap();

    let err = rpc_err(dispatch(
        Method::TestsRun,
        serde_json::json!({
            "runId": "r1",
            "scope": "npm; rm -rf /",
            "reason": "blocked scope",
            "target": null
        }),
        &env.store,
        &no_hybrid_config(),
    ));

    assert!(
        err.contains("not whitelisted"),
        "shell metachar scope should be blocked; got: {err}"
    );
    println!("[Tests] blocked scope rejected: {err}");
}

// ---------------------------------------------------------------------------
// Test 6: hybrid.patch.submit rejects non-succeeded worker
// ---------------------------------------------------------------------------

#[test]
fn hybrid_patch_submit_rejects_non_succeeded_worker() {
    let env = TestEnv::new();
    env.store.save_run(&env.make_run("r1")).unwrap();

    let worker = deterministic_protocol::HybridWorkerRun {
        worker_run_id: "w_prepared".into(),
        parent_run_id: "r1".into(),
        status: deterministic_protocol::HybridWorkerStatus::Prepared,
        provider_profile_id: "mock".into(),
        task_goal: "task".into(),
        focus_paths: vec![],
        prompt: "mock".into(),
        proposed_edits: Some(vec![]),
        summary: None,
        failure_message: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        started_at: None,
        completed_at: None,
        cancel_requested: false,
        context_files: vec![],
    };
    env.store.save_worker_run(&worker).unwrap();

    let err = rpc_err(dispatch(
        Method::HybridPatchSubmit,
        serde_json::json!({
            "runId": "r1",
            "workerRunId": "w_prepared",
            "patchIndices": [0]
        }),
        &env.store,
        &no_hybrid_config(),
    ));

    assert!(err.contains("succeeded"), "must reject non-succeeded worker; got: {err}");
    println!("[Submit] correctly rejected non-succeeded worker: {err}");
}

// ---------------------------------------------------------------------------
// Test 7: hybrid.patch.submit rejects empty patch_indices
// ---------------------------------------------------------------------------

#[test]
fn hybrid_patch_submit_rejects_empty_indices() {
    let env = TestEnv::new();
    env.store.save_run(&env.make_run("r1")).unwrap();

    let worker = deterministic_protocol::HybridWorkerRun {
        worker_run_id: "w_ok".into(),
        parent_run_id: "r1".into(),
        status: deterministic_protocol::HybridWorkerStatus::Succeeded,
        provider_profile_id: "mock".into(),
        task_goal: "task".into(),
        focus_paths: vec![],
        prompt: "mock".into(),
        proposed_edits: Some(vec![]),
        summary: None,
        failure_message: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        started_at: None,
        completed_at: None,
        cancel_requested: false,
        context_files: vec![],
    };
    env.store.save_worker_run(&worker).unwrap();

    let err = rpc_err(dispatch(
        Method::HybridPatchSubmit,
        serde_json::json!({
            "runId": "r1",
            "workerRunId": "w_ok",
            "patchIndices": []
        }),
        &env.store,
        &no_hybrid_config(),
    ));

    assert!(err.contains("empty"), "empty patch_indices should be rejected; got: {err}");
    println!("[Submit] correctly rejected empty indices: {err}");
}

// ---------------------------------------------------------------------------
// Test 8: approval.resolve with deny clears retryable_action
// ---------------------------------------------------------------------------

#[test]
fn approval_resolve_denied_clears_retryable_action() {
    let env = TestEnv::new();
    env.store.save_run(&env.make_run("r1")).unwrap();

    // Create pending approval with a retryable_action (simulates hybrid.patch.submit)
    let approval = PendingApproval {
        approval_id: "appr_deny".into(),
        run_id: "r1".into(),
        action_description: "apply proposed patches".into(),
        risk_reason: "worker proposed edits".into(),
        policy_rationale: "needs human review".into(),
        status: "pending".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    env.store.save_approval(&approval).unwrap();

    // Put a retryable_action on the run
    let mut run = env.store.get_run("r1").unwrap().unwrap();
    run.retryable_action = Some(deterministic_protocol::RetryableAction {
        kind: "patch.apply".into(),
        summary: "Apply worker proposed edits".into(),
        payload: Some(r#"{"runId":"r1","skipPolicy":true}"#.into()),
        retryable_reason: "Hybrid worker proposed edits".into(),
        is_valid: true,
        is_recommended: false,
        invalidation_reason: None,
        recommended_tool: "apply_patch".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
    });
    env.store.save_run(&run).unwrap();

    // Deny the approval
    let (_result, updated_run) = dispatch(
        Method::ApprovalResolve,
        serde_json::json!({
            "runId": "r1",
            "approvalId": "appr_deny",
            "decision": "deny",
            "reason": "not confident"
        }),
        &env.store,
        &no_hybrid_config(),
    )
    .unwrap();

    // Approval status updated
    let resolved = env.store.get_approval("appr_deny").unwrap().unwrap();
    assert_eq!(resolved.status, "denied");

    // retryable_action should be invalidated (is_valid=false) on deny
    let updated = updated_run.as_ref().unwrap();
    let ra = updated.retryable_action.as_ref().unwrap();
    assert!(!ra.is_valid, "denying approval should invalidate retryable_action");
    assert!(!ra.is_recommended, "is_recommended should be false after deny");
    assert!(
        ra.invalidation_reason
            .as_ref()
            .unwrap()
            .contains("denied"),
        "invalidation_reason should mention denied"
    );
    println!("[Approval] denied: retryable_action invalidated, is_valid=false");
}
