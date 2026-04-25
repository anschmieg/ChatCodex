# No hidden-agent invariants

These invariants are mandatory.

## Invariants

1. No backend component may call an LLM in deterministic mode.
2. In hybrid mode, worker LLM calls are bounded and return proposed patches only.
3. No public MCP tool may resume or continue an autonomous coding run.
4. Every file mutation must originate from `apply_patch`.
5. Every test execution must originate from `run_tests` or a tightly restricted `run_command`.
6. The TypeScript MCP gateway must not contain core planning logic.
7. The Rust daemon must not expose any method that implies agent-owned iteration.
8. Accidental model-runtime code paths must fail hard.
9. Hybrid workers never apply patches, run tests, commit, or mutate files directly.

## CI checks

The following checks run in CI (`.github/workflows/milestone-deterministic.yml`):

- ✅ fail build if deterministic crates depend on model SDKs
- ✅ fail build if MCP tool registry contains forbidden tool names
- ✅ fail build if daemon method registry contains forbidden method names
- ✅ fail build if hybrid mode is enabled without provider config
- ✅ fail build if hybrid worker tools are registered when hybrid is not configured
- ✅ test that public tools map only to deterministic daemon methods
- ✅ test that hybrid mode tools only appear when `CHATCODEX_HYBRID_ENABLED=true`

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
