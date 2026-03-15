# No hidden-agent invariants

These invariants are mandatory.

## Invariants

1. No backend component may call an LLM.
2. No public MCP tool may resume or continue an autonomous coding run.
3. Every file mutation must originate from `apply_patch`.
4. Every test execution must originate from `run_tests` or a tightly restricted `run_command`.
5. The TypeScript MCP gateway must not contain core planning logic.
6. The Rust daemon must not expose any method that implies agent-owned iteration.
7. Accidental model-runtime code paths must fail hard.

## CI checks

The following checks run in CI (`.github/workflows/milestone-deterministic.yml`):

- ✅ fail build if deterministic crates depend on model SDKs
- ✅ fail build if MCP tool registry contains forbidden tool names
- ✅ fail build if daemon method registry contains forbidden method names
- ✅ test that public tools map only to deterministic daemon methods

## Forbidden strings to grep for in new public surfaces

* `turn/start`
* `turn/steer`
* `review/start`
* `codex()`
* `codex-reply()`
* `continue_run`
* `resume_thread`
* `agent_step`
* `fix_end_to_end`

## Review rule

If a design choice makes it ambiguous whether the backend is still acting like an agent, reject that design and keep control with ChatGPT.
