/**
 * Entry point for the ChatGPT MCP gateway.
 *
 * This is a thin MCP server that:
 *  - Registers deterministic tools
 *  - Validates inputs with Zod
 *  - Forwards requests to the Rust daemon
 *  - Formats responses for ChatGPT
 *
 * It contains NO core planning logic and NO model calls.
 */

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { DaemonClient } from "./daemon-client.js";
import { registerTools } from "./tools.js";

async function main(): Promise<void> {
  const server = new McpServer({
    name: "chatgpt-deterministic-mcp",
    version: "0.0.1",
  });

  const client = new DaemonClient();

  registerTools(server, client);

  const transport = new StdioServerTransport();
  await server.connect(transport);

  // Log to stderr so MCP stdio transport is not polluted
  process.stderr.write("chatgpt-mcp: server started on stdio\n");
}

main().catch((err) => {
  process.stderr.write(`chatgpt-mcp: fatal: ${err}\n`);
  process.exit(1);
});
