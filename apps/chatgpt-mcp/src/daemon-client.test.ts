import { afterEach, describe, it } from "node:test";
import assert from "node:assert/strict";
import { DaemonClient } from "./daemon-client.js";

const originalFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = originalFetch;
});

describe("DaemonClient", () => {
  it("maps JSON-RPC method/params and returns result", async () => {
    let capturedUrl: string | undefined;
    let capturedBody: unknown;

    globalThis.fetch = (async (input, init) => {
      capturedUrl = String(input);
      capturedBody = JSON.parse(String(init?.body ?? "{}"));
      return new Response(
        JSON.stringify({
          jsonrpc: "2.0",
          id: "req_1",
          result: { ok: true, runId: "r-1" },
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      );
    }) as typeof fetch;

    const client = new DaemonClient("http://127.0.0.1:19999");
    const result = await client.call<{ ok: boolean; runId: string }>(
      "run.prepare",
      { workspaceId: "/tmp/ws", userGoal: "goal" },
    );

    assert.equal(capturedUrl, "http://127.0.0.1:19999/rpc");
    assert.deepEqual(result, { ok: true, runId: "r-1" });
    assert.equal((capturedBody as { method: string }).method, "run.prepare");
    assert.deepEqual((capturedBody as { params: unknown }).params, {
      workspaceId: "/tmp/ws",
      userGoal: "goal",
    });
  });

  it("surfaces daemon transport failures with startup guidance", async () => {
    globalThis.fetch = (async () => new Response("down", { status: 503, statusText: "Service Unavailable" })) as typeof fetch;
    const client = new DaemonClient("http://127.0.0.1:19999");

    await assert.rejects(
      () => client.call("run.prepare", { workspaceId: "/tmp/ws", userGoal: "goal" }),
      (error: unknown) => {
        const message = String((error as Error).message);
        assert.match(message, /daemon unreachable/i);
        assert.match(message, /deterministic-daemon --port <port> --data-dir <path>/);
        return true;
      },
    );
  });

  it("adds recovery hints for known daemon error categories", async () => {
    globalThis.fetch = (async () =>
      new Response(
        JSON.stringify({
          jsonrpc: "2.0",
          id: "req_2",
          error: { code: -32000, message: "unknown run: r-missing" },
        }),
        { status: 200, headers: { "Content-Type": "application/json" } },
      )) as typeof fetch;

    const client = new DaemonClient("http://127.0.0.1:19999");
    await assert.rejects(
      () => client.call("run.get", { runId: "r-missing" }),
      (error: unknown) => {
        const message = String((error as Error).message);
        assert.match(message, /ChatCodex error in run\.get: unknown run: r-missing/);
        assert.match(message, /Use list_runs to see available runs/i);
        return true;
      },
    );
  });
});
