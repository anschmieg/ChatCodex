import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { DaemonClient } from "./daemon-client.js";
import { registerTools } from "./tools.js";

export function createMcpServer(): McpServer {
  const server = new McpServer({
    name: "chatgpt-deterministic-mcp",
    version: "0.0.1",
  });

  registerTools(server, new DaemonClient());
  return server;
}
