/**
 * Entry point for the ChatGPT MCP gateway.
 *
 * Supports:
 *  - `stdio` transport for local MCP client process spawning
 *  - Streamable HTTP transport for remote MCP hosting
 *
 * It contains NO core planning logic and NO model calls.
 */

import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { createMcpServer } from "./mcp-server.js";
import { startHttpServer } from "./http-server.js";

async function main(): Promise<void> {
  const transportMode = (process.env["MCP_TRANSPORT"] ?? "stdio").toLowerCase();

  if (transportMode === "http") {
    await startHttpServer();
    return;
  }

  const server = createMcpServer();
  const transport = new StdioServerTransport();

  await server.connect(transport);
  // Log to stderr so MCP stdio transport is not polluted
  process.stderr.write("chatgpt-mcp: server started on stdio\n");
}

main().catch((err) => {
  process.stderr.write(`chatgpt-mcp: fatal: ${err}\n`);
  process.exit(1);
});
