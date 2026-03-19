import {
  OpenIdProviderDiscoveryMetadataSchema,
  type OAuthMetadata,
} from "@modelcontextprotocol/sdk/shared/auth.js";
import type { OAuthTokenVerifier } from "@modelcontextprotocol/sdk/server/auth/provider.js";
import type { AuthInfo } from "@modelcontextprotocol/sdk/server/auth/types.js";
import { createRemoteJWKSet, jwtVerify, type JWTPayload } from "jose";
import type { ExternalOAuthAuthConfig } from "./config.js";

export interface OAuthRuntime {
  oauthMetadata: OAuthMetadata;
  verifier: OAuthTokenVerifier;
  requiredScopes: string[];
  scopesSupported: string[];
  resourceServerUrl: URL;
}

interface IntrospectionSuccessPayload {
  active?: boolean;
  client_id?: string;
  scope?: string;
  exp?: number;
  aud?: string | string[];
  resource?: string;
  sub?: string;
  [key: string]: unknown;
}

function toBasicAuth(clientId: string, clientSecret: string): string {
  return Buffer.from(`${clientId}:${clientSecret}`, "utf8").toString("base64");
}

function parseScopes(value: unknown): string[] {
  if (typeof value === "string") {
    return value
      .split(" ")
      .map((scope) => scope.trim())
      .filter((scope) => scope.length > 0);
  }

  if (Array.isArray(value)) {
    return value.filter((scope): scope is string => typeof scope === "string");
  }

  return [];
}

function normalizeAudience(value: unknown): string[] {
  if (typeof value === "string") {
    return [value];
  }

  if (Array.isArray(value)) {
    return value.filter((entry): entry is string => typeof entry === "string");
  }

  return [];
}

function maybeUrl(value: string | undefined): URL | undefined {
  if (!value) {
    return undefined;
  }

  try {
    return new URL(value);
  } catch {
    return undefined;
  }
}

function mapIntrospectionAuthInfo(
  token: string,
  payload: IntrospectionSuccessPayload,
): AuthInfo {
  return {
    token,
    clientId: payload.client_id ?? "unknown-client",
    scopes: parseScopes(payload.scope),
    expiresAt: payload.exp,
    resource: maybeUrl(payload.resource),
    extra: { subject: payload.sub },
  };
}

function mapJwtAuthInfo(
  token: string,
  payload: JWTPayload,
  clientIdClaim: unknown,
  resourceServerUrl: URL,
): AuthInfo {
  const audience = normalizeAudience(payload.aud);
  const matchingResource = audience.find((candidate) => candidate === resourceServerUrl.href);

  return {
    token,
    clientId:
      typeof clientIdClaim === "string"
        ? clientIdClaim
        : typeof payload.client_id === "string"
          ? payload.client_id
          : typeof payload.sub === "string"
            ? payload.sub
            : "unknown-client",
    scopes: parseScopes(payload.scope ?? payload.scp),
    expiresAt: payload.exp,
    resource: matchingResource ? new URL(matchingResource) : undefined,
    extra: { subject: payload.sub },
  };
}

function assertAudience(expectedAudience: string | undefined, actual: unknown): void {
  if (!expectedAudience) {
    return;
  }

  const audiences = normalizeAudience(actual);
  if (!audiences.includes(expectedAudience)) {
    throw new Error(
      `OAuth token audience mismatch. Expected "${expectedAudience}", got "${audiences.join(", ")}".`,
    );
  }
}

function buildOAuthMetadataFromConfig(config: ExternalOAuthAuthConfig): OAuthMetadata {
  if (!config.authorizationEndpoint || !config.tokenEndpoint) {
    throw new Error("OAuth metadata is incomplete. Missing authorization or token endpoint.");
  }

  return {
    issuer: config.issuerUrl.href,
    authorization_endpoint: config.authorizationEndpoint.href,
    token_endpoint: config.tokenEndpoint.href,
    registration_endpoint: config.registrationEndpoint?.href,
    revocation_endpoint: config.revocationEndpoint?.href,
    introspection_endpoint: config.introspectionEndpoint?.href,
    scopes_supported: config.scopesSupported,
    response_types_supported: ["code"],
    grant_types_supported: ["authorization_code", "refresh_token"],
    token_endpoint_auth_methods_supported: ["none", "client_secret_basic", "client_secret_post"],
    code_challenge_methods_supported: ["S256"],
    service_documentation: config.serviceDocumentationUrl?.href,
  };
}

async function fetchDiscoveryMetadata(
  discoveryUrl: URL,
  fetchImpl: typeof fetch,
): Promise<OAuthMetadata & { jwks_uri?: string }> {
  const response = await fetchImpl(discoveryUrl, {
    headers: { Accept: "application/json" },
  });

  if (!response.ok) {
    throw new Error(
      `Unable to load OAuth discovery metadata from ${discoveryUrl.href} (HTTP ${response.status}).`,
    );
  }

  const payload = OpenIdProviderDiscoveryMetadataSchema.parse(await response.json());
  return {
    issuer: payload.issuer,
    authorization_endpoint: payload.authorization_endpoint,
    token_endpoint: payload.token_endpoint,
    registration_endpoint: payload.registration_endpoint,
    scopes_supported: payload.scopes_supported,
    response_types_supported: payload.response_types_supported,
    response_modes_supported: payload.response_modes_supported,
    grant_types_supported: payload.grant_types_supported,
    token_endpoint_auth_methods_supported: payload.token_endpoint_auth_methods_supported,
    token_endpoint_auth_signing_alg_values_supported:
      payload.token_endpoint_auth_signing_alg_values_supported,
    service_documentation:
      typeof payload.service_documentation === "string"
        ? payload.service_documentation
        : undefined,
    code_challenge_methods_supported: payload.code_challenge_methods_supported,
    client_id_metadata_document_supported: payload.client_id_metadata_document_supported,
    jwks_uri: payload.jwks_uri,
  };
}

function createIntrospectionVerifier(
  config: ExternalOAuthAuthConfig,
  endpoint: URL,
  fetchImpl: typeof fetch,
): OAuthTokenVerifier {
  return {
    verifyAccessToken: async (token: string) => {
      const body = new URLSearchParams({ token });
      const headers: Record<string, string> = {
        Accept: "application/json",
        "Content-Type": "application/x-www-form-urlencoded",
      };

      if (config.introspectionClientId && config.introspectionClientSecret) {
        headers["Authorization"] = `Basic ${toBasicAuth(
          config.introspectionClientId,
          config.introspectionClientSecret,
        )}`;
      }

      const response = await fetchImpl(endpoint, {
        method: "POST",
        headers,
        body: body.toString(),
      });

      if (!response.ok) {
        throw new Error(
          `OAuth introspection failed at ${endpoint.href} (HTTP ${response.status}).`,
        );
      }

      const payload = (await response.json()) as IntrospectionSuccessPayload;
      if (!payload.active) {
        throw new Error("OAuth token is inactive.");
      }

      assertAudience(config.audience, payload.aud);
      return mapIntrospectionAuthInfo(token, payload);
    },
  };
}

function createJwtVerifier(
  config: ExternalOAuthAuthConfig,
  jwksUrl: URL,
): OAuthTokenVerifier {
  const jwks = createRemoteJWKSet(jwksUrl);

  return {
    verifyAccessToken: async (token: string) => {
      const { payload } = await jwtVerify(token, jwks, {
        issuer: config.issuerUrl.href,
        audience: config.audience,
        algorithms: config.jwtAlgorithms && config.jwtAlgorithms.length > 0
          ? config.jwtAlgorithms
          : undefined,
      });

      return mapJwtAuthInfo(token, payload, payload.azp, config.resourceServerUrl);
    },
  };
}

export async function initializeOAuthRuntime(
  config: ExternalOAuthAuthConfig,
  fetchImpl: typeof fetch = fetch,
): Promise<OAuthRuntime> {
  const discoveredMetadata = config.discoveryUrl
    ? await fetchDiscoveryMetadata(config.discoveryUrl, fetchImpl)
    : undefined;

  const oauthMetadata = discoveredMetadata ?? buildOAuthMetadataFromConfig(config);
  const introspectionEndpoint =
    config.introspectionEndpoint ??
    (oauthMetadata.introspection_endpoint
      ? new URL(oauthMetadata.introspection_endpoint)
      : undefined);
  const jwksUrl =
    config.jwksUrl ??
    (discoveredMetadata?.jwks_uri ? new URL(discoveredMetadata.jwks_uri) : undefined);

  const verifier =
    introspectionEndpoint
      ? createIntrospectionVerifier(config, introspectionEndpoint, fetchImpl)
      : jwksUrl
        ? createJwtVerifier(config, jwksUrl)
        : undefined;

  if (!verifier) {
    throw new Error(
      "OAuth token verification is not configured. Provide introspection, JWKS, or discovery metadata.",
    );
  }

  return {
    oauthMetadata,
    verifier,
    requiredScopes: config.requiredScopes,
    scopesSupported: config.scopesSupported,
    resourceServerUrl: config.resourceServerUrl,
  };
}
