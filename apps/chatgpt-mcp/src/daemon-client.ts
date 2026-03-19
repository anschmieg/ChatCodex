/**
 * Internal JSON-RPC client for the deterministic Rust daemon.
 *
 * This module is the **only** place the MCP gateway talks to the daemon.
 */

export interface JsonRpcRequest {
  jsonrpc: "2.0";
  id: string;
  method: string;
  params: Record<string, unknown>;
}

export interface JsonRpcError {
  code: number;
  message: string;
  data?: unknown;
}

export interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: string;
  result?: unknown;
  error?: JsonRpcError;
}

let requestIdCounter = 0;
function nextId(): string {
  requestIdCounter += 1;
  return `req_${requestIdCounter}`;
}

/**
 * Get recovery hints based on error message patterns.
 */
function getRecoveryHints(errorMessage: string): string[] {
  const hints: string[] = [];

  if (errorMessage.includes("unknown run") || errorMessage.includes("not found")) {
    hints.push("Use list_runs to see available runs, or check the run_id parameter.");
  }

  if (errorMessage.includes("cannot be reopened") || errorMessage.includes("cannot be finalized")) {
    hints.push("Use get_run_state to check the current status.");
  }

  if (errorMessage.includes("already finalized")) {
    hints.push("Use reopen_run to continue work, or supersede_run to start a new approach.");
  }

  if (errorMessage.includes("already archived")) {
    hints.push("Use unarchive_run to restore it to the default list.");
  }

  if (errorMessage.includes("not archived")) {
    hints.push("Only archived runs can be unarchived. Use list_runs with includeArchived to see archived runs.");
  }

  if (errorMessage.includes("not snoozed")) {
    hints.push("Only snoozed runs can be unsnoozed. Use list_runs with includeSnoozed to see snoozed runs.");
  }

  if (errorMessage.includes("requires approval") || errorMessage.includes("approval")) {
    hints.push("Use approve_action to approve or deny the pending action.");
  }

  if (errorMessage.includes("view not found") || errorMessage.includes("view name cannot be empty")) {
    hints.push("Use list_queue_views to see available saved views.");
  }

  if (errorMessage.includes("already exists")) {
    hints.push("Choose a different name or use update_* to modify the existing item.");
  }

  return hints;
}

export class DaemonClient {
  private baseUrl: string;

  constructor(baseUrl?: string) {
    this.baseUrl =
      baseUrl ??
      process.env["DETERMINISTIC_DAEMON_URL"] ??
      "http://127.0.0.1:19280";
  }

  async healthz(): Promise<boolean> {
    const resp = await fetch(`${this.baseUrl}/healthz`);
    return resp.ok;
  }

  async call<T = unknown>(
    method: string,
    params: Record<string, unknown>,
  ): Promise<T> {
    const body: JsonRpcRequest = {
      jsonrpc: "2.0",
      id: nextId(),
      method,
      params,
    };

    const resp = await fetch(`${this.baseUrl}/rpc`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });

    if (!resp.ok) {
      throw new Error(
        `ChatCodex daemon unreachable (HTTP ${resp.status} ${resp.statusText}). ` +
        `Ensure the daemon is running at ${this.baseUrl}. ` +
        `Start the daemon with DETERMINISTIC_BIND=<host:port> and DETERMINISTIC_STORE_DIR=<path>.`,
      );
    }

    const json = (await resp.json()) as JsonRpcResponse;

    if (json.error) {
      const errorMessage = json.error.message;
      const hints = getRecoveryHints(errorMessage);
      const hintText = hints.length > 0 ? ` ${hints.join(" ")}` : "";

      throw new Error(
        `ChatCodex error in ${method}: ${errorMessage}${hintText}`,
      );
    }

    return json.result as T;
  }
}
