import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { describe, it } from "node:test";
import * as assert from "node:assert/strict";
import { exportJWK, generateKeyPair } from "jose";
import { initializeEmbeddedOidcRuntime } from "./embedded-oidc.js";
import type { EmbeddedOidcAuthConfig } from "./config.js";

async function createTestConfig(): Promise<EmbeddedOidcAuthConfig> {
  const { privateKey } = await generateKeyPair("RS256", { extractable: true });
  const jwk = await exportJWK(privateKey);
  jwk.alg = "RS256";
  jwk.use = "sig";
  jwk.kid = "test-signing-key";

  return {
    mode: "oauth",
    provider: "embedded-oidc",
    registrationMode: "both",
    issuerUrl: new URL("https://codex.nothing.pink/oauth"),
    resourceServerUrl: new URL("https://codex.nothing.pink/mcp"),
    authorizationEndpoint: new URL("https://codex.nothing.pink/oauth/authorize"),
    tokenEndpoint: new URL("https://codex.nothing.pink/oauth/token"),
    registrationEndpoint: new URL("https://codex.nothing.pink/oauth/register"),
    revocationEndpoint: new URL("https://codex.nothing.pink/oauth/revoke"),
    jwksUrl: new URL("https://codex.nothing.pink/oauth/jwks"),
    scopesSupported: ["mcp:tools"],
    requiredScopes: ["mcp:tools"],
    storagePath: join(tmpdir(), `chatcodex-oidc-${randomUUID()}.sqlite`),
    cookieKeys: ["cookie-key-1"],
    jwks: { keys: [jwk] },
    cimdAllowedHosts: ["chat.openai.com", "chatgpt.com", "openai.com"],
    login: {
      provider: "cloudflare-access",
      teamDomain: new URL("https://nothingpink.cloudflareaccess.com"),
      audience: "self-hosted-app-aud",
      emailClaim: "email",
    },
  };
}

describe("initializeEmbeddedOidcRuntime", () => {
  it("exposes registration and CIMD metadata for ChatGPT clients", async () => {
    const runtime = await initializeEmbeddedOidcRuntime(await createTestConfig());

    assert.equal(runtime.oauthMetadata.issuer, "https://codex.nothing.pink/oauth");
    assert.equal(
      runtime.oauthMetadata.registration_endpoint,
      "https://codex.nothing.pink/oauth/register",
    );
    assert.equal(runtime.oauthMetadata.client_id_metadata_document_supported, true);

    await runtime.close();
  });

  it("can run in CIMD-only mode without advertising dynamic registration", async () => {
    const config = await createTestConfig();
    config.registrationMode = "cimd";
    config.registrationEndpoint = undefined;

    const runtime = await initializeEmbeddedOidcRuntime(config);

    assert.equal(runtime.oauthMetadata.client_id_metadata_document_supported, true);
    assert.equal(runtime.oauthMetadata.registration_endpoint, undefined);

    await runtime.close();
  });
});
