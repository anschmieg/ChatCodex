//! JSON-RPC handler dispatch.

use anyhow::Result;
use deterministic_protocol::methods::Method;
use deterministic_protocol::*;

use crate::persistence::Store;

/// Dispatch a JSON-RPC request to the appropriate handler.
///
/// Returns `(result_value, optional_run_state)` so the router can wrap
/// both in the response envelope.
pub fn dispatch(
    method: Method,
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    match method {
        Method::RunPrepare => handle_run_prepare(params, store),
        Method::WorkspaceSummary => handle_workspace_summary(params),
        Method::FileRead => handle_file_read(params, store),
        Method::GitStatus => handle_git_status(params, store),
        Method::CodeSearch => handle_code_search(params, store),
        Method::PatchApply => handle_patch_apply(params, store),
        Method::TestsRun => handle_tests_run(params, store),
        Method::GitDiff => handle_git_diff(params, store),
    }
}

fn handle_run_prepare(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: RunPrepareParams = serde_json::from_value(params)?;
    let (result, state) = deterministic_core::run_prepare::prepare(&p)?;
    store.save_run(&state)?;
    Ok((serde_json::to_value(result)?, Some(state)))
}

fn handle_workspace_summary(
    params: serde_json::Value,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: WorkspaceSummaryParams = serde_json::from_value(params)?;
    let result = deterministic_core::workspace_summary::summary(&p)?;
    Ok((serde_json::to_value(result)?, None))
}

fn handle_file_read(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: FileReadParams = serde_json::from_value(params)?;
    let ws = store
        .workspace_for_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;
    let result = deterministic_core::file_read::read(&p, &ws)?;
    let run_state = store.get_run(&p.run_id)?;
    Ok((serde_json::to_value(result)?, run_state))
}

fn handle_git_status(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: GitStatusParams = serde_json::from_value(params)?;
    let ws = store
        .workspace_for_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;
    let result = deterministic_core::git_status::status(&p, &ws)?;
    let run_state = store.get_run(&p.run_id)?;
    Ok((serde_json::to_value(result)?, run_state))
}

fn handle_code_search(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: CodeSearchParams = serde_json::from_value(params)?;
    let ws = store
        .workspace_for_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;
    let result = deterministic_core::code_search::search(&p, &ws)?;
    let run_state = store.get_run(&p.run_id)?;
    Ok((serde_json::to_value(result)?, run_state))
}

fn handle_patch_apply(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: PatchApplyParams = serde_json::from_value(params)?;
    let ws = store
        .workspace_for_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;
    let result = deterministic_core::patch_apply::apply(&p, &ws)?;
    let run_state = store.get_run(&p.run_id)?;
    Ok((serde_json::to_value(result)?, run_state))
}

fn handle_tests_run(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: TestsRunParams = serde_json::from_value(params)?;
    let ws = store
        .workspace_for_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;
    let result = deterministic_core::tests_run::run(&p, &ws)?;
    let run_state = store.get_run(&p.run_id)?;
    Ok((serde_json::to_value(result)?, run_state))
}

fn handle_git_diff(
    params: serde_json::Value,
    store: &Store,
) -> Result<(serde_json::Value, Option<RunState>)> {
    let p: GitDiffParams = serde_json::from_value(params)?;
    let ws = store
        .workspace_for_run(&p.run_id)?
        .ok_or_else(|| anyhow::anyhow!("unknown run: {}", p.run_id))?;
    let result = deterministic_core::git_diff::diff(&p, &ws)?;
    let run_state = store.get_run(&p.run_id)?;
    Ok((serde_json::to_value(result)?, run_state))
}
