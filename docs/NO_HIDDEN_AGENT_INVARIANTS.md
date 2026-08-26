# No hidden-agent invariants

These invariants are mandatory.

## Invariants

1. No backend component may call an LLM.
2. No public MCP tool may perform autonomous coding work or continue a loop.
3. Every workspace source file mutation must originate from `apply_patch`.
4. Command execution must run through deterministic command tools and the server-side sandbox/policy layer.
5. Lifecycle persistence must be deterministic project/run state, not hidden planning logic.
6. The Rust daemon must not expose any method that implies agent-owned iteration.
7. Accidental model-runtime code paths must fail hard.

`run_resume` is permitted only as deterministic state selection for a
non-terminal persisted run. It does not execute work; ChatGPT must continue by
calling fine-grained tools.

## CI checks

The following checks run in CI (`.github/workflows/milestone-deterministic.yml`):

- ✅ fail build if deterministic crates depend on model SDKs
- ✅ fail build if MCP tool registry contains forbidden tool names
- ✅ fail build if daemon method registry contains forbidden method names
- ✅ test that public tools map only to deterministic daemon methods
- ✅ test persistent lifecycle state, transition rejection, lease safety, schema
  parity, and run metadata

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
