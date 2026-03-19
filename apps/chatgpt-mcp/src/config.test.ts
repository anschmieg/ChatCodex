import { describe, it } from "node:test";
import * as assert from "node:assert/strict";
import { loadServerConfig } from "./config.js";

describe("loadServerConfig", () => {
  it("defaults to unauthenticated mode when no auth env is provided", () => {
    const config = loadServerConfig({});
    assert.equal(config.auth.mode, "none");
    assert.equal(config.host, "0.0.0.0");
    assert.equal(config.port, 3000);
  });

  it("loads static bearer auth when MCP_AUTH_TOKEN is set", () => {
    const config = loadServerConfig({
      MCP_AUTH_TOKEN: "test-token",
    });

    assert.deepEqual(config.auth, {
      mode: "static-token",
      token: "test-token",
    });
  });

  it("loads OAuth config from explicit endpoint settings", () => {
    const config = loadServerConfig({
      PUBLIC_BASE_URL: "https://chatcodex.example.com",
      OAUTH_ISSUER_URL: "https://auth.example.com",
      OAUTH_AUTHORIZATION_ENDPOINT: "https://auth.example.com/oauth/authorize",
      OAUTH_TOKEN_ENDPOINT: "https://auth.example.com/oauth/token",
      OAUTH_JWKS_URL: "https://auth.example.com/.well-known/jwks.json",
      OAUTH_SCOPES_SUPPORTED: "mcp:tools,profile",
      OAUTH_REQUIRED_SCOPES: "mcp:tools",
    });

    assert.equal(config.auth.mode, "oauth");
    if (config.auth.mode !== "oauth") {
      throw new Error("expected oauth config");
    }

    assert.equal(config.auth.resourceServerUrl.href, "https://chatcodex.example.com/mcp");
    assert.deepEqual(config.auth.scopesSupported, ["mcp:tools", "profile"]);
    assert.deepEqual(config.auth.requiredScopes, ["mcp:tools"]);
    assert.equal(config.auth.jwksUrl?.href, "https://auth.example.com/.well-known/jwks.json");
  });

  it("rejects OAuth mode without remote resource URL context", () => {
    assert.throws(
      () =>
        loadServerConfig({
          OAUTH_ISSUER_URL: "https://auth.example.com",
          OAUTH_AUTHORIZATION_ENDPOINT: "https://auth.example.com/oauth/authorize",
          OAUTH_TOKEN_ENDPOINT: "https://auth.example.com/oauth/token",
          OAUTH_JWKS_URL: "https://auth.example.com/.well-known/jwks.json",
        }),
      /OAUTH_RESOURCE_SERVER_URL or PUBLIC_BASE_URL is required/,
    );
  });

  it("loads Cloudflare Access OAuth config from team domain and client id", () => {
    const config = loadServerConfig({
      CHATCODEX_AUTH_MODE: "oauth",
      CHATCODEX_OAUTH_PROVIDER: "cloudflare-access",
      PUBLIC_BASE_URL: "https://chatcodex.example.com",
      CLOUDFLARE_ACCESS_TEAM_DOMAIN: "acme.cloudflareaccess.com",
      CLOUDFLARE_ACCESS_CLIENT_ID: "cf-client-id",
    });

    assert.equal(config.auth.mode, "oauth");
    if (config.auth.mode !== "oauth") {
      throw new Error("expected oauth config");
    }

    assert.equal(config.auth.provider, "cloudflare-access");
    assert.equal(
      config.auth.issuerUrl.href,
      "https://acme.cloudflareaccess.com/cdn-cgi/access/sso/oidc/cf-client-id",
    );
    assert.equal(
      config.auth.discoveryUrl?.href,
      "https://acme.cloudflareaccess.com/cdn-cgi/access/sso/oidc/cf-client-id/.well-known/openid-configuration",
    );
    assert.equal(
      config.auth.jwksUrl?.href,
      "https://acme.cloudflareaccess.com/cdn-cgi/access/sso/oidc/cf-client-id/jwks",
    );
    assert.deepEqual(config.auth.requiredScopes, []);
  });

  it("rejects Cloudflare Access config without client id", () => {
    assert.throws(
      () =>
        loadServerConfig({
          CHATCODEX_AUTH_MODE: "oauth",
          CHATCODEX_OAUTH_PROVIDER: "cloudflare-access",
          PUBLIC_BASE_URL: "https://chatcodex.example.com",
          CLOUDFLARE_ACCESS_TEAM_DOMAIN: "https://acme.cloudflareaccess.com",
        }),
      /CLOUDFLARE_ACCESS_CLIENT_ID is required/,
    );
  });
});
