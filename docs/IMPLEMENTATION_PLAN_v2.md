# Dual-Mode ChatCodex Implementation Plan

## Summary

Build two explicit ChatCodex modes:

- **Deterministic mode:** current default. ChatGPT is the only LLM. Rust daemon and MCP gateway stay deterministic and code-only.
- **Hybrid mode:** ChatGPT remains the orchestrator, but may start bounded implementation-worker runs using a configured external/local LLM. For v1, implement **OpenAI-compatible HTTP** only. This covers OpenAI-compatible servers and Ollama when exposed through an OpenAI-compatible endpoint. Do not implement Anthropic in this pass.

Hybrid workers must never write workspace files directly. They may read files, search code, and produce proposed `PatchEdit[]` output. ChatGPT then chooses whether to apply that patch through the existing `apply_patch` policy path.

## Non-Negotiable Rules

- Deterministic mode must behave exactly like today: no backend model calls.
- Hybrid mode must be opt-in through server config and per-run mode.
- TypeScript MCP remains thin: schemas, tool registration, daemon calls.
- Rust owns state, policy, worker execution, provider calls, and persistence.
- No public tool may use forbidden names: `continue_run`, `resume_thread`, `agent_step`, `fix_end_to_end`, `turn_start`, `turn_steer`, `review_start`.
- No hybrid worker may apply patches, run tests, commit, or mutate files.
- All actual file changes still go through `apply_patch`.
- All actual test execution still goes through `run_tests`.

## Phase 0: Repo Hygiene And Verification Setup

### Task 0.1: Ignore runtime databases

Files:
- Modify `.gitignore`

Steps:
- Add these ignore entries:
  ```gitignore
  apps/chatgpt-mcp/.data/
  codex-rs/runs/
  ```
- Verify:
  ```bash
  git status --short
  ```
- Expected: `.data/` and `codex-rs/runs/` no longer appear as untracked files.

### Task 0.2: Fix Rust toolchain verification docs

Files:
- Modify `docs/DEVELOPMENT.md`
- Modify `docs/PROJECT_STATUS.md`

Steps:
- Document that Rust tests require `codex-rs/rust-toolchain.toml`, currently pinned to `1.93.0`.
- Add this command as the expected Rust verification:
  ```bash
  cd codex-rs
  cargo test -p deterministic-protocol -p deterministic-core -p deterministic-daemon
  ```
- Add note: if Cargo reports edition 2024 unsupported, the local Rust toolchain is too old.

### Task 0.3: Run current baseline checks

Commands:
```bash
cd apps/chatgpt-mcp
npm run build
npm run typecheck
npm test
```

Expected:
- Build passes.
- Typecheck passes.
- Node tests pass.

Rust command:
```bash
cd codex-rs
cargo test -p deterministic-protocol -p deterministic-core -p deterministic-daemon
```

Expected:
- Either tests pass with Rust 1.93.0, or fail only because local Cargo is too old. Do not proceed with Rust implementation until the correct toolchain is available.

## Phase 1: Update Project Goal And Invariants

### Task 1.1: Update architecture docs for dual-mode goal

Files:
- Modify `AGENTS.md`
- Modify `docs/ARCHITECTURE.md`
- Modify `docs/NO_HIDDEN_AGENT_INVARIANTS.md`
- Modify `docs/PROJECT_STATUS.md`
- Modify `docs/MCP_TOOL_CONTRACTS.md`

Steps:
- Replace “the only LLM in the stack is ChatGPT” with mode-specific wording:
  - In deterministic mode, ChatGPT is the only LLM.
  - In hybrid mode, ChatGPT orchestrates one or more bounded worker LLM runs.
- Keep deterministic mode as default.
- State clearly that hybrid worker LLMs are implementation workers, not orchestrators.
- State that hybrid workers return proposed patches only.
- State that ChatGPT must explicitly invoke `apply_patch` and `run_tests`.

### Task 1.2: Replace root README

Files:
- Modify `README.md`

Steps:
- Remove upstream Codex CLI marketing as the primary README content.
- Add ChatCodex overview:
  - What the repo is.
  - Deterministic mode.
  - Hybrid mode.
  - Rust daemon.
  - TypeScript MCP gateway.
  - Safety model.
  - Basic verification commands.
- Keep upstream attribution/license notes where appropriate.

## Phase 2: Fix Registry Drift Before Adding Hybrid Tools

### Task 2.1: Make MCP tool registry authoritative

Files:
- Modify `apps/chatgpt-mcp/src/tools.ts`
- Modify `apps/chatgpt-mcp/src/invariants.test.ts`

Steps:
- Replace the manually maintained `REGISTERED_TOOL_NAMES` array with a single exported `TOOL_DEFINITIONS` array.
- Each entry must contain:
  - `name`
  - `description`
  - `schema`
  - `daemonMethod`
  - `mapParams`
- Register tools by iterating over `TOOL_DEFINITIONS`.
- Export:
  ```ts
  export const REGISTERED_TOOL_NAMES = TOOL_DEFINITIONS.map((tool) => tool.name);
  ```
- Include all currently registered tools, including:
  - `get_run_queue_overview`
  - `create_queue_view`
  - `update_queue_view`
  - `delete_queue_view`
  - `get_queue_view`
  - `list_queue_views`

Tests:
- Add test that every `TOOL_DEFINITIONS` name appears exactly once.
- Add test that every registered tool avoids forbidden names.
- Add test that queue-view tools are included in `REGISTERED_TOOL_NAMES`.

Verification:
```bash
cd apps/chatgpt-mcp
npm run build
npm test
```

### Task 2.2: Fix Rust method registry completeness

Files:
- Modify `codex-rs/deterministic-protocol/src/methods.rs`

Steps:
- Add missing queue view methods to `Method::all()`:
  - `QueueViewCreate`
  - `QueueViewUpdate`
  - `QueueViewDelete`
  - `QueueViewGet`
  - `QueueViewList`
- Add a test that every string accepted by `parse_method` roundtrips through `Method::all()`.

Verification:
```bash
cd codex-rs
cargo test -p deterministic-protocol methods
```

## Phase 3: Add Dual-Mode Protocol Types

### Task 3.1: Add harness mode protocol types

Files:
- Modify `codex-rs/deterministic-protocol/src/types.rs`
- Modify `apps/chatgpt-mcp/src/schemas.ts`

Rust types:
- Add enum:
  ```rust
  pub enum HarnessMode {
      Deterministic,
      Hybrid,
  }
  ```
- Serialize as camel-case strings:
  - `deterministic`
  - `hybrid`

Run state changes:
- Add `harness_mode: HarnessMode` to `RunState`.
- Add `harness_mode: Option<HarnessMode>` to `RunPolicyInput` or `RunPrepareParams`.
- Default to `deterministic`.

TypeScript schema:
- Add:
  ```ts
  harnessMode: z.enum(["deterministic", "hybrid"]).optional()
  ```
- Add it to `CodexPrepareRunInput`.

### Task 3.2: Persist harness mode

Files:
- Modify `codex-rs/deterministic-daemon/src/persistence.rs`

Steps:
- Add SQLite migration:
  ```sql
  ALTER TABLE runs ADD COLUMN harness_mode TEXT NOT NULL DEFAULT 'deterministic'
  ```
- Save `RunState.harness_mode`.
- Load missing/unknown values as `deterministic`.
- Add persistence tests:
  - default mode is deterministic.
  - hybrid mode roundtrips.
  - old DB migration sets deterministic.

Verification:
```bash
cd codex-rs
cargo test -p deterministic-daemon persistence
```

## Phase 4: Add Hybrid Provider Configuration

### Task 4.1: Add Rust provider config

Files:
- Create `codex-rs/deterministic-core/src/hybrid_provider.rs`
- Modify `codex-rs/deterministic-core/src/lib.rs`
- Modify `codex-rs/deterministic-daemon/src/main.rs`

Provider config:
- Define `HybridProviderProfile`:
  - `profile_id: String`
  - `kind: "openai_compatible"`
  - `base_url: String`
  - `api_key_env: Option<String>`
  - `model: String`
  - `timeout_seconds: u64`
  - `temperature: f32`
  - `max_output_tokens: u32`

Environment variables:
- `CHATCODEX_HYBRID_ENABLED=true`
- `CHATCODEX_HYBRID_PROVIDER_BASE_URL`
- `CHATCODEX_HYBRID_PROVIDER_MODEL`
- `CHATCODEX_HYBRID_PROVIDER_API_KEY_ENV`
- `CHATCODEX_HYBRID_PROVIDER_TIMEOUT_SECONDS`
- `CHATCODEX_HYBRID_PROVIDER_MAX_OUTPUT_TOKENS`
- `CHATCODEX_HYBRID_PROVIDER_TEMPERATURE`

Defaults:
- Hybrid disabled unless `CHATCODEX_HYBRID_ENABLED=true`.
- Provider kind is always `openai_compatible` in v1.
- Timeout default: `120`.
- Max output tokens default: `8000`.
- Temperature default: `0.2`.

Tests:
- Config rejects hybrid mode when no provider base URL/model is configured.
- Config accepts Ollama-style local endpoint with no API key.
- Config accepts OpenAI-compatible endpoint with API key env name.

## Phase 5: Add Hybrid Worker Run Protocol And Persistence

### Task 5.1: Add worker run DTOs

Files:
- Modify `codex-rs/deterministic-protocol/src/types.rs`
- Modify `codex-rs/deterministic-protocol/src/methods.rs`

Add methods:
- `hybrid.worker.prepare`
- `hybrid.worker.start`
- `hybrid.worker.get`
- `hybrid.worker.cancel`
- `hybrid.worker.list`

Add public-facing result types:
- `HybridWorkerStatus`:
  - `prepared`
  - `running`
  - `succeeded`
  - `failed`
  - `cancelled`
- `HybridWorkerRun`:
  - `worker_run_id`
  - `parent_run_id`
  - `status`
  - `provider_profile_id`
  - `task_goal`
  - `focus_paths`
  - `created_at`
  - `updated_at`
  - `started_at`
  - `completed_at`
  - `failure_message`
  - `proposed_edits`
  - `summary`
- Params/results for prepare/start/get/cancel/list.

Important behavior:
- `prepare` creates a worker run but does not call an LLM.
- `start` calls the configured provider and updates the worker run.
- `get` and `list` are read-only.
- `cancel` only cancels a prepared/running worker if possible; it does not mutate workspace files.

### Task 5.2: Add SQLite worker tables

Files:
- Modify `codex-rs/deterministic-daemon/src/persistence.rs`

Add table:
```sql
CREATE TABLE IF NOT EXISTS hybrid_worker_runs (
  worker_run_id TEXT PRIMARY KEY,
  parent_run_id TEXT NOT NULL,
  status TEXT NOT NULL,
  provider_profile_id TEXT NOT NULL,
  task_goal TEXT NOT NULL,
  focus_paths TEXT NOT NULL DEFAULT '[]',
  prompt TEXT NOT NULL,
  proposed_edits TEXT,
  summary TEXT,
  failure_message TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  started_at TEXT,
  completed_at TEXT,
  cancel_requested INTEGER NOT NULL DEFAULT 0,
  FOREIGN KEY (parent_run_id) REFERENCES runs(run_id)
);
```

Add persistence methods:
- `save_worker_run`
- `get_worker_run`
- `list_worker_runs`
- `mark_worker_cancel_requested`

Tests:
- prepare/persist/get.
- list by parent run.
- cancellation flag persists.
- proposed edits roundtrip.

## Phase 6: Implement OpenAI-Compatible Worker Client

### Task 6.1: Add provider HTTP client

Files:
- Create `codex-rs/deterministic-core/src/openai_compatible_worker.rs`

Implementation:
- Use existing Rust HTTP dependency already available in workspace if possible.
- Send request to:
  ```text
  {base_url}/chat/completions
  ```
- Request body:
  ```json
  {
    "model": "<model>",
    "temperature": 0.2,
    "max_tokens": 8000,
    "messages": [
      { "role": "system", "content": "<worker system prompt>" },
      { "role": "user", "content": "<task prompt>" }
    ]
  }
  ```
- If API key env exists, send:
  ```text
  Authorization: Bearer <value>
  ```
- If API key env is absent, send no Authorization header.

Worker output contract:
- Require the model to return JSON only:
  ```json
  {
    "summary": "short summary",
    "edits": [
      {
        "path": "relative/file/path",
        "operation": "replace",
        "oldText": "exact old text",
        "newText": "replacement text",
        "reason": "why"
      }
    ]
  }
  ```
- Parse into `Vec<PatchEdit>`.
- Reject invalid JSON.
- Reject empty `edits`.
- Reject absolute paths.
- Reject paths containing `..`.

Tests:
- Parses valid provider response.
- Rejects malformed JSON.
- Rejects absolute paths.
- Rejects traversal paths.
- Builds auth header only when API key env exists.

## Phase 7: Implement Hybrid Worker Handlers

### Task 7.1: Add daemon handlers

Files:
- Modify `codex-rs/deterministic-daemon/src/handlers.rs`
- Modify `codex-rs/deterministic-daemon/src/router.rs` only if state needs provider config

Behavior:
- `hybrid.worker.prepare`
  - Load parent run.
  - Reject if parent run `harness_mode` is not `hybrid`.
  - Reject if daemon hybrid config is disabled.
  - Persist worker in `prepared` status.
- `hybrid.worker.start`
  - Load worker and parent run.
  - Reject if worker is not `prepared`.
  - Enforce concurrency limits:
    - global running max: `3`
    - per parent run running max: `3`
  - Build prompt from:
    - parent run goal
    - worker task goal
    - focus paths
    - relevant file excerpts supplied in params
  - Call OpenAI-compatible client.
  - Save `succeeded` with proposed edits or `failed` with failure message.
- `hybrid.worker.get`
  - Return worker run.
- `hybrid.worker.list`
  - Return workers for parent run.
- `hybrid.worker.cancel`
  - If prepared: mark cancelled.
  - If running: set cancel requested and return status.
  - If succeeded/failed/cancelled: return unchanged terminal status.

Tests:
- deterministic parent run rejects worker prepare.
- hybrid parent run allows worker prepare.
- start rejects when hybrid config disabled.
- start saves proposed edits on success.
- start saves failure on provider error.
- concurrency limit rejects fourth running worker.

## Phase 8: Add MCP Hybrid Tools

### Task 8.1: Add TypeScript schemas

Files:
- Modify `apps/chatgpt-mcp/src/schemas.ts`

Add schemas:
- `HybridPrepareWorkerRunInput`
  - `runId: string`
  - `taskGoal: string`
  - `focusPaths?: string[]`
  - `contextFiles?: { path: string; startLine?: number; endLine?: number }[]`
- `HybridStartWorkerRunInput`
  - `workerRunId: string`
- `HybridGetWorkerRunInput`
  - `workerRunId: string`
- `HybridCancelWorkerRunInput`
  - `workerRunId: string`
  - `reason?: string`
- `HybridListWorkerRunsInput`
  - `runId: string`
  - `status?: "prepared" | "running" | "succeeded" | "failed" | "cancelled"`

Validation:
- `taskGoal` min 1, max 1000.
- `focusPaths` max 20.
- `contextFiles` max 20.
- All paths must be relative strings; reject absolute paths and `..`.

### Task 8.2: Register MCP tools

Files:
- Modify `apps/chatgpt-mcp/src/tools.ts`

Add tool mappings:
- `hybrid_prepare_worker_run` → `hybrid.worker.prepare`
- `hybrid_start_worker_run` → `hybrid.worker.start`
- `hybrid_get_worker_run` → `hybrid.worker.get`
- `hybrid_cancel_worker_run` → `hybrid.worker.cancel`
- `hybrid_list_worker_runs` → `hybrid.worker.list`

Tests:
- Tool names appear in generated registry.
- Tool names do not match forbidden patterns.
- Schemas reject invalid paths.
- Schemas accept valid minimal input.

Verification:
```bash
cd apps/chatgpt-mcp
npm run build
npm run typecheck
npm test
```

## Phase 9: Fix Patch Application Safety

### Task 9.1: Validate create paths before writing

Files:
- Modify `codex-rs/deterministic-core/src/patch_apply.rs`

Steps:
- Compute canonical root once before edit loop.
- For create operations:
  - Reject absolute paths.
  - Reject components containing `..`.
  - Build normalized path under root.
  - Ensure parent directory is inside root before creating it.
  - Only then write file.
- For replace line ranges:
  - If `startLine` is greater than total lines, return an error.
  - If `startLine > endLine`, return an error.
- Keep old-text replacement behavior unchanged.

Tests:
- Create with `../../outside.txt` does not create any file outside workspace.
- Create with absolute path is rejected.
- Replace with `startLine` beyond file length returns error.
- Replace with `startLine > endLine` returns error.
- Normal create and replace still pass.

Verification:
```bash
cd codex-rs
cargo test -p deterministic-core patch_apply
```

## Phase 10: Tighten Test Command Safety

### Task 10.1: Validate test targets

Files:
- Modify `codex-rs/deterministic-core/src/tests_run.rs`
- Modify `codex-rs/deterministic-core/src/approval_policy.rs` if needed

Rules:
- `cargo` target may contain only ASCII alphanumeric, `_`, `-`, `:`, `/`, `.`
- `npm` target may contain only ASCII alphanumeric, `_`, `-`, `:`, `/`, `.`
- `pytest` target may contain only ASCII alphanumeric, `_`, `-`, `:`, `/`, `.`
- `make` target keeps existing approval policy and also uses the same character allowlist.
- Reject target strings containing whitespace, shell metacharacters, or path traversal.

Tests:
- Reject `target: "test && rm -rf /"`.
- Reject `target: "../secret"`.
- Accept `target: "my_test"`.
- Accept `target: "tests/foo_test.py::test_case"`.
- Existing safe make target tests still pass.

Verification:
```bash
cd codex-rs
cargo test -p deterministic-core tests_run approval_policy
```

## Phase 11: Improve Search

### Task 11.1: Prefer ripgrep

Files:
- Modify `codex-rs/deterministic-core/src/code_search.rs`

Behavior:
- Try `rg --line-number --no-heading --color never`.
- If `pathGlob` is provided, pass `--glob`.
- Respect `.gitignore` by default.
- If `rg` is unavailable, fall back to current `grep` behavior.
- Keep output shape unchanged.

Tests:
- Search finds text.
- Search respects max results.
- Search respects path glob.
- Empty query returns empty result.
- No matches returns empty result.

Verification:
```bash
cd codex-rs
cargo test -p deterministic-core code_search
```

## Phase 12: Final End-To-End Validation

### Deterministic mode scenario

Steps:
```bash
# Start daemon with hybrid disabled
DETERMINISTIC_BIND=127.0.0.1:19280 \
DETERMINISTIC_STORE_DIR=/tmp/chatcodex-deterministic \
cargo run -p deterministic-daemon
```

Then through MCP or direct RPC:
- Prepare run with `harnessMode: "deterministic"`.
- Read a file.
- Search code.
- Apply a safe patch.
- Run tests.
- Show diff.

Expected:
- No provider config required.
- No hybrid worker tool can start for this run.
- Patch/test behavior matches current deterministic behavior.

### Hybrid mode scenario

Steps:
```bash
CHATCODEX_HYBRID_ENABLED=true \
CHATCODEX_HYBRID_PROVIDER_BASE_URL=http://127.0.0.1:11434/v1 \
CHATCODEX_HYBRID_PROVIDER_MODEL=<local-model> \
DETERMINISTIC_BIND=127.0.0.1:19280 \
DETERMINISTIC_STORE_DIR=/tmp/chatcodex-hybrid \
cargo run -p deterministic-daemon
```

Then:
- Prepare run with `harnessMode: "hybrid"`.
- Start two worker runs with different `taskGoal` values.
- Inspect both worker outputs.
- Apply one proposed patch through `apply_patch`.
- Run tests through `run_tests`.

Expected:
- Worker runs may execute in parallel.
- Worker outputs are proposed edits only.
- Workspace changes only after `apply_patch`.
- Audit history shows worker lifecycle and applied patch separately.

## Acceptance Criteria

- Deterministic mode remains backward compatible and default.
- Hybrid mode is opt-in and provider-config gated.
- OpenAI-compatible provider works with a configurable base URL/model/API key env.
- Ollama works through an OpenAI-compatible local endpoint when available.
- Anthropic is not implemented in v1 and is documented as future work.
- Tool and method registries cannot drift silently.
- Runtime DB directories are ignored by git.
- TypeScript build, typecheck, and tests pass.
- Rust deterministic crates pass with the pinned Rust toolchain.
