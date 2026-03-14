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

Input:
- `runId`: string
- `scope`: string — semantic test scope. Well-known values:
  - Framework names: `"cargo"`, `"npm"`, `"pytest"`, `"make"`
  - Semantic labels: `"unit"`, `"integration"`, `"all"`
  - The daemon resolves semantic labels to framework commands based on workspace detection
- `target?`: string — specific test target (e.g., test name, file path)
- `reason`: string — why tests are being run

Returns:
- resolved command
- exit code
- summary counts
- stdout/stderr (truncated in structured content)
- updated run-state summary

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
