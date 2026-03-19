import { randomUUID } from "node:crypto";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import { createMcpExpressApp } from "@modelcontextprotocol/sdk/server/express.js";
import {
  getOAuthProtectedResourceMetadataUrl,
  mcpAuthMetadataRouter,
} from "@modelcontextprotocol/sdk/server/auth/router.js";
import { requireBearerAuth } from "@modelcontextprotocol/sdk/server/auth/middleware/bearerAuth.js";
import { isInitializeRequest } from "@modelcontextprotocol/sdk/types.js";
import express, { type Request, type Response, type NextFunction } from "express";
import { createMcpServer } from "./mcp-server.js";
import { DaemonClient } from "./daemon-client.js";
import { loadServerConfig } from "./config.js";
import { initializeOAuthRuntime } from "./oauth.js";

const MAX_BODY_SIZE = "1mb";

function sendJson(
  res: Response,
  statusCode: number,
  body: unknown,
): void {
  res.status(statusCode).json(body);
}

function sendText(
  res: Response,
  statusCode: number,
  body: string,
  contentType = "text/plain; charset=utf-8",
): void {
  res.status(statusCode).type(contentType).send(body);
}

async function handleHealthz(res: Response, daemonClient: DaemonClient): Promise<void> {
  const daemonOk = await daemonClient.healthz().catch(() => false);
  if (daemonOk) {
    sendJson(res, 200, { status: "ok" });
    return;
  }

  sendJson(res, 503, {
    status: "degraded",
    daemon: "unreachable",
  });
}

interface SessionContext {
  server: ReturnType<typeof createMcpServer>;
  transport: StreamableHTTPServerTransport;
}

function getSessionId(req: Request): string | undefined {
  const value = req.header("mcp-session-id");
  return value && value.trim().length > 0 ? value.trim() : undefined;
}

function createStaticTokenMiddleware(token: string) {
  return (req: Request, res: Response, next: NextFunction): void => {
    if (req.header("authorization") === `Bearer ${token}`) {
      next();
      return;
    }

    res.status(401).set("WWW-Authenticate", 'Bearer realm="chatcodex-mcp"').json({
      error: "unauthorized",
      message: "Provide a valid Bearer token.",
    });
  };
}

function createSessionContext(
  sessions: Map<string, SessionContext>,
): SessionContext {
  const server = createMcpServer();
  const transport = new StreamableHTTPServerTransport({
    sessionIdGenerator: () => randomUUID(),
    onsessioninitialized: (sessionId) => {
      sessions.set(sessionId, { server, transport });
    },
  });

  transport.onclose = () => {
    const sessionId = transport.sessionId;
    if (sessionId) {
      sessions.delete(sessionId);
    }
    void server.close();
  };

  transport.onerror = (error) => {
    process.stderr.write(`chatgpt-mcp: transport error: ${error}\n`);
  };

  return { server, transport };
}

async function handleMcpRequest(
  req: Request,
  res: Response,
  sessions: Map<string, SessionContext>,
): Promise<void> {
  const sessionId = getSessionId(req);
  let context = sessionId ? sessions.get(sessionId) : undefined;

  if (!context) {
    if (sessionId) {
      sendJson(res, 404, {
        jsonrpc: "2.0",
        error: {
          code: -32001,
          message: `Unknown MCP session: ${sessionId}`,
        },
        id: null,
      });
      return;
    }

    if (req.method !== "POST" || !isInitializeRequest(req.body)) {
      sendJson(res, 400, {
        jsonrpc: "2.0",
        error: {
          code: -32000,
          message: "Bad Request: expected initialize request without session ID.",
        },
        id: null,
      });
      return;
    }

    context = createSessionContext(sessions);
    await context.server.connect(context.transport);
  }

  await context.transport.handleRequest(req, res, req.body);
}

export async function startHttpServer(): Promise<void> {
  const config = loadServerConfig();
  const daemonClient = new DaemonClient(config.daemonUrl);
  const app = createMcpExpressApp({
    host: config.host,
    allowedHosts: config.allowedHosts.length > 0 ? config.allowedHosts : undefined,
  });
  const sessions = new Map<string, SessionContext>();

  app.use(express.json({ limit: MAX_BODY_SIZE }));
  app.use(express.urlencoded({ extended: false, limit: MAX_BODY_SIZE }));

  if (config.auth.mode === "oauth") {
    const oauth = await initializeOAuthRuntime(config.auth);
    app.use(
      mcpAuthMetadataRouter({
        oauthMetadata: oauth.oauthMetadata,
        resourceServerUrl: oauth.resourceServerUrl,
        serviceDocumentationUrl: config.auth.serviceDocumentationUrl,
        scopesSupported: oauth.scopesSupported,
        resourceName: "ChatCodex MCP",
      }),
    );

    app.all(
      config.mcpPath,
      requireBearerAuth({
        verifier: oauth.verifier,
        requiredScopes: oauth.requiredScopes,
        resourceMetadataUrl: getOAuthProtectedResourceMetadataUrl(oauth.resourceServerUrl),
      }),
      async (req, res, next) => {
        try {
          await handleMcpRequest(req, res, sessions);
        } catch (error) {
          next(error);
        }
      },
    );
  } else if (config.auth.mode === "static-token") {
    app.all(config.mcpPath, createStaticTokenMiddleware(config.auth.token), async (req, res, next) => {
      try {
        await handleMcpRequest(req, res, sessions);
      } catch (error) {
        next(error);
      }
    });
  } else {
    app.all(config.mcpPath, async (req, res, next) => {
      try {
        await handleMcpRequest(req, res, sessions);
      } catch (error) {
        next(error);
      }
    });
  }

  app.get(config.healthzPath, async (_req, res, next) => {
    try {
      await handleHealthz(res, daemonClient);
    } catch (error) {
      next(error);
    }
  });

  app.get("/", (_req, res) => {
    sendJson(res, 200, {
      service: "chatcodex-mcp",
      transport: "streamable-http",
      authMode: config.auth.mode,
      mcpPath: config.mcpPath,
      healthz: config.healthzPath,
      publicBaseUrl: config.publicBaseUrl?.href,
    });
  });

  app.use((req, res) => {
    sendText(res, 404, `Not Found: ${req.path}`);
  });

  app.use((error: unknown, _req: Request, res: Response, _next: NextFunction) => {
    const message = error instanceof Error ? error.message : "internal server error";
    sendJson(res, 500, {
      error: "internal_error",
      message,
    });
  });

  await new Promise<void>((resolve, reject) => {
    const server = app.listen(config.port, config.host, () => resolve());
    server.once("error", reject);
  });

  process.stderr.write(
    `chatgpt-mcp: HTTP server listening on http://${config.host}:${config.port}${config.mcpPath}\n`,
  );
}
