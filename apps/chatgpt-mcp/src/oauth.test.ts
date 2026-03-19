import { describe, it } from "node:test";
import * as assert from "node:assert/strict";
import { initializeOAuthRuntime } from "./oauth.js";
import type { OAuthAuthConfig } from "./config.js";

describe("initializeOAuthRuntime", () => {
  const baseConfig: OAuthAuthConfig = {
    mode: "oauth",
    issuerUrl: new URL("https://auth.example.com"),
    resourceServerUrl: new URL("https://chatcodex.example.com/mcp"),
    authorizationEndpoint: new URL("https://auth.example.com/oauth/authorize"),
    tokenEndpoint: new URL("https://auth.example.com/oauth/token"),
    introspectionEndpoint: new URL("https://auth.example.com/oauth/introspect"),
    scopesSupported: ["mcp:tools"],
    requiredScopes: ["mcp:tools"],
  };

  it("uses explicit metadata and introspection to build the verifier", async () => {
    const runtime = await initializeOAuthRuntime(baseConfig, async (_input, init) => {
      assert.equal(init?.method, "POST");
      return new Response(
        JSON.stringify({
          active: true,
          client_id: "chatgpt-client",
          scope: "mcp:tools",
          exp: 2000000000,
        }),
        {
          status: 200,
          headers: { "Content-Type": "application/json" },
        },
      );
    });

    assert.equal(runtime.oauthMetadata.issuer, "https://auth.example.com/");
    const authInfo = await runtime.verifier.verifyAccessToken("opaque-token");
    assert.equal(authInfo.clientId, "chatgpt-client");
    assert.deepEqual(authInfo.scopes, ["mcp:tools"]);
  });

  it("rejects inactive introspection responses", async () => {
    const runtime = await initializeOAuthRuntime(baseConfig, async () => {
      return new Response(JSON.stringify({ active: false }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    });

    await assert.rejects(
      runtime.verifier.verifyAccessToken("opaque-token"),
      /inactive/,
    );
  });
});
