# MCP tool contracts

These are the public tools exposed to ChatGPT.

## codex_prepare_run

Input:
- `workspaceId: string`
- `userGoal: string`
- `focusPaths?: string[]`
- `mode?: "plan" | "refresh" | "repair" | "review"`

Returns structured content:
- `runId`
- `objective`
- `assistantBrief`
- `constraints`
- `plan`
- `currentStep`
- `recommendedNextAction`
- `recommendedTool`
- `status`

## get_workspace_summary

Input:
- `workspaceId: string`
- `focusPaths?: string[]`

Returns:
- root info
- detected language/tooling
- dirty files
- likely commands
- relevant paths

## read_file

Input:
- `runId`
- `path`
- `startLine?`
- `endLine?`
- `purpose?`

Returns:
- file content
- range metadata
- updated run state summary

## git_status

Input:
- `runId`

Returns:
- branch
- dirty files
- untracked files
- ahead/behind if available

## search_code

Input:
- `runId`
- `query`
- `pathGlob?`
- `maxResults?`

Returns:
- ranked matches
- snippets
- updated run-state summary

## apply_patch

Input:
- `runId`
- `edits[]`

Each edit:
- `path`
- `operation`
- `startLine?`
- `endLine?`
- `oldText?`
- `newText`
- `anchorText?`
- `reason`

Returns:
- changed files
- diff stats
- updated run-state summary

## run_tests

Execute a whitelisted test command in the workspace.

### Input

- `runId`: string — Run ID from codex_prepare_run
- `scope`: string — **Semantic test scope**. Accepted values:
  - **Framework names** (explicit): `"cargo"`, `"npm"`, `"pytest"`, `"make"`
  - **Semantic labels** (auto-resolved): `"unit"`, `"integration"`, `"all"`
- `target?`: string — Specific test target (e.g., test name, file path)
- `reason`: string — Why tests are being run (required for audit trail)

### Scope Resolution

1. If `scope` is a framework name, use it directly
2. If `scope` is a semantic label, detect framework via workspace files:
   - `Cargo.toml` exists → "cargo"
   - `package.json` exists → "npm"
   - `setup.py` or `pyproject.toml` exists → "pytest"
   - `Makefile` exists → "make"
3. If no framework detected, return error

### Validation

- `scope` must be non-empty and a supported value
- `reason` must be non-empty (for audit trail)
- Scope matching is case-insensitive

### Returns

- `resolvedCommand`: string — The actual command that was executed
- `exitCode`: number — Exit code from the test command
- `stdout`: string — Standard output (truncated to 4096 chars)
- `stderr`: string — Standard error (truncated to 4096 chars)
- `summary`: string — Human-readable summary of results

### Errors

- Returns error for unsupported scope values
- Returns error if workspace framework cannot be auto-detected
- Returns error if test command fails to execute

## show_diff

Input:
- `runId`
- `paths?: string[]`
- `format?: "summary" | "patch"`

Returns:
- changed files
- diff summary
- optionally patch text
- updated run-state summary

## Forbidden public tools

Do not expose:
- `continue_run`
- `resume_codex_thread`
- `fix_end_to_end`
- `agent_step`
- `turn_start`
- `codex_reply`
