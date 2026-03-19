import { createServer, IncomingMessage, ServerResponse } from "node:http";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import { createMcpServer } from "./mcp-server.js";
import { DaemonClient } from "./daemon-client.js";

const DEFAULT_PORT = 3000;
const DEFAULT_HOST = "0.0.0.0";
const MAX_BODY_BYTES = 1024 * 1024;

function sendJson(
  res: ServerResponse,
  statusCode: number,
  body: unknown,
): void {
  const payload = JSON.stringify(body);
  res.writeHead(statusCode, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(payload),
  });
  res.end(payload);
}

function sendText(
  res: ServerResponse,
  statusCode: number,
  body: string,
  contentType = "text/plain; charset=utf-8",
): void {
  res.writeHead(statusCode, {
    "content-type": contentType,
    "content-length": Buffer.byteLength(body),
  });
  res.end(body);
}

function getPath(req: IncomingMessage): string {
  const rawUrl = req.url ?? "/";
  return new URL(rawUrl, "http://localhost").pathname;
}

function requireBearerToken(req: IncomingMessage, res: ServerResponse): boolean {
  const expectedToken = process.env["MCP_AUTH_TOKEN"];
  if (!expectedToken) {
    return true;
  }

  const authHeader = req.headers["authorization"];
  if (authHeader === `Bearer ${expectedToken}`) {
    return true;
  }

  res.writeHead(401, {
    "content-type": "application/json",
    "www-authenticate": 'Bearer realm="chatcodex-mcp"',
  });
  res.end(
    JSON.stringify({
      error: "unauthorized",
      message: "Provide a valid Bearer token.",
    }),
  );
  return false;
}

async function parseJsonBody(req: IncomingMessage): Promise<unknown | undefined> {
  if (req.method !== "POST") {
    return undefined;
  }

  const chunks: Buffer[] = [];
  let total = 0;

  for await (const chunk of req) {
    const bufferChunk = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    total += bufferChunk.length;
    if (total > MAX_BODY_BYTES) {
      throw new Error("request body too large");
    }
    chunks.push(bufferChunk);
  }

  if (chunks.length === 0) {
    return undefined;
  }

  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

async function handleHealthz(res: ServerResponse): Promise<void> {
  const daemonOk = await new DaemonClient().healthz().catch(() => false);
  if (daemonOk) {
    sendJson(res, 200, { status: "ok" });
    return;
  }

  sendJson(res, 503, {
    status: "degraded",
    daemon: "unreachable",
  });
}

async function handleMcpRequest(
  req: IncomingMessage,
  res: ServerResponse,
): Promise<void> {
  if (!requireBearerToken(req, res)) {
    return;
  }

  if (req.method !== "POST") {
    sendJson(res, 405, {
      jsonrpc: "2.0",
      error: {
        code: -32000,
        message: "Method not allowed.",
      },
      id: null,
    });
    return;
  }

  const parsedBody = await parseJsonBody(req);
  const server = createMcpServer();
  const transport = new StreamableHTTPServerTransport({
    sessionIdGenerator: undefined,
  });

  try {
    await server.connect(transport);
    await transport.handleRequest(req, res, parsedBody);
  } finally {
    res.on("close", () => {
      void transport.close();
      void server.close();
    });
  }
}

export async function startHttpServer(): Promise<void> {
  const port = Number.parseInt(process.env["PORT"] ?? `${DEFAULT_PORT}`, 10);
  const host = process.env["HOST"] ?? DEFAULT_HOST;

  const server = createServer(async (req, res) => {
    try {
      const path = getPath(req);

      if (path === "/healthz") {
        await handleHealthz(res);
        return;
      }

      if (path === "/") {
        sendJson(res, 200, {
          service: "chatcodex-mcp",
          transport: "streamable-http",
          mcpPath: "/mcp",
          healthz: "/healthz",
        });
        return;
      }

      if (path === "/mcp") {
        await handleMcpRequest(req, res);
        return;
      }

      sendText(res, 404, "Not Found");
    } catch (error) {
      const message = error instanceof Error ? error.message : "internal server error";
      sendJson(res, 500, {
        error: "internal_error",
        message,
      });
    }
  });

  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, host, () => resolve());
  });

  process.stderr.write(
    `chatgpt-mcp: HTTP server listening on http://${host}:${port}/mcp\n`,
  );
}
