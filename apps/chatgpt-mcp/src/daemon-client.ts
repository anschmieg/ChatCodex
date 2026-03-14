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

let _counter = 0;
function nextId(): string {
  _counter += 1;
  return `req_${_counter}`;
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
        `daemon HTTP error: ${resp.status} ${resp.statusText}`,
      );
    }

    const json = (await resp.json()) as JsonRpcResponse;

    if (json.error) {
      throw new Error(
        `daemon RPC error [${json.error.code}]: ${json.error.message}`,
      );
    }

    return json.result as T;
  }
}
