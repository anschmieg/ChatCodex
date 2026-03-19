export interface ServerRuntimeConfig {
  host: string;
  port: number;
  allowedHosts: string[];
  publicBaseUrl?: URL;
  mcpPath: string;
  healthzPath: string;
  daemonUrl?: string;
  auth: AuthConfig;
}

export type AuthConfig = NoAuthConfig | StaticTokenAuthConfig | OAuthAuthConfig;

export interface NoAuthConfig {
  mode: "none";
}

export interface StaticTokenAuthConfig {
  mode: "static-token";
  token: string;
}

interface BaseOAuthAuthConfig {
  mode: "oauth";
  issuerUrl: URL;
  resourceServerUrl: URL;
  serviceDocumentationUrl?: URL;
  scopesSupported: string[];
  requiredScopes: string[];
}

export interface ExternalOAuthAuthConfig extends BaseOAuthAuthConfig {
  provider?: "generic" | "cloudflare-access";
  discoveryUrl?: URL;
  authorizationEndpoint?: URL;
  tokenEndpoint?: URL;
  registrationEndpoint?: URL;
  revocationEndpoint?: URL;
  introspectionEndpoint?: URL;
  jwksUrl?: URL;
  audience?: string;
  introspectionClientId?: string;
  introspectionClientSecret?: string;
  jwtAlgorithms?: string[];
}

export interface EmbeddedOidcCloudflareAccessLoginConfig {
  provider: "cloudflare-access";
  teamDomain: URL;
  audience: string;
  emailClaim: string;
}

export interface EmbeddedOidcAuthConfig extends BaseOAuthAuthConfig {
  provider: "embedded-oidc";
  authorizationEndpoint: URL;
  tokenEndpoint: URL;
  registrationEndpoint: URL;
  revocationEndpoint: URL;
  jwksUrl: URL;
  storagePath: string;
  cookieKeys: string[];
  jwks: { keys: JsonWebKey[] };
  cimdAllowedHosts: string[];
  login: EmbeddedOidcCloudflareAccessLoginConfig;
}

export type OAuthAuthConfig = ExternalOAuthAuthConfig | EmbeddedOidcAuthConfig;

type Env = NodeJS.ProcessEnv;

const DEFAULT_HOST = "0.0.0.0";
const DEFAULT_PORT = 3000;
const DEFAULT_MCP_PATH = "/mcp";
const DEFAULT_HEALTHZ_PATH = "/healthz";
const DEFAULT_SCOPE = "mcp:tools";
const CLOUDFLARE_ACCESS_PROVIDER = "cloudflare-access";
const EMBEDDED_OIDC_PROVIDER = "embedded-oidc";
const DEFAULT_EMBEDDED_OIDC_PATH = "/oauth";
const DEFAULT_OIDC_STORAGE_PATH = ".data/chatcodex-oidc.sqlite";
const DEFAULT_CIMD_ALLOWED_HOSTS = ["chat.openai.com", "chatgpt.com", "openai.com"];

function readTrimmed(env: Env, key: string): string | undefined {
  const raw = env[key];
  if (raw === undefined) {
    return undefined;
  }

  const trimmed = raw.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

function readUrl(env: Env, key: string): URL | undefined {
  const value = readTrimmed(env, key);
  return value ? new URL(value) : undefined;
}

function readCsv(env: Env, key: string, fallback: string[] = []): string[] {
  const value = readTrimmed(env, key);
  if (!value) {
    return fallback;
  }

  return value
    .split(",")
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
}

function readPositiveInteger(env: Env, key: string, fallback: number): number {
  const value = readTrimmed(env, key);
  if (!value) {
    return fallback;
  }

  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`${key} must be a positive integer, got "${value}".`);
  }

  return parsed;
}

function inferAuthMode(env: Env): "none" | "static-token" | "oauth" {
  const explicit = readTrimmed(env, "CHATCODEX_AUTH_MODE");
  if (explicit === "none" || explicit === "static-token" || explicit === "oauth") {
    return explicit;
  }

  if (
    readTrimmed(env, "CHATCODEX_OAUTH_PROVIDER") === EMBEDDED_OIDC_PROVIDER ||
    readTrimmed(env, "OAUTH_ISSUER_URL") ||
    readTrimmed(env, "OAUTH_DISCOVERY_URL") ||
    readTrimmed(env, "OAUTH_AUTHORIZATION_ENDPOINT") ||
    readTrimmed(env, "OAUTH_TOKEN_ENDPOINT")
  ) {
    return "oauth";
  }

  if (readTrimmed(env, "MCP_AUTH_TOKEN")) {
    return "static-token";
  }

  return "none";
}

function normalizeCloudflareTeamDomain(raw: string): URL {
  const withScheme = raw.startsWith("http://") || raw.startsWith("https://")
    ? raw
    : `https://${raw}`;
  const url = new URL(withScheme);

  if (url.protocol !== "https:") {
    throw new Error("CLOUDFLARE_ACCESS_TEAM_DOMAIN must use https.");
  }

  if (url.pathname !== "/" || url.search || url.hash) {
    throw new Error(
      "CLOUDFLARE_ACCESS_TEAM_DOMAIN must be an origin only, for example https://example.cloudflareaccess.com.",
    );
  }

  return url;
}

function requireUrl(value: URL | undefined, key: string): URL {
  if (!value) {
    throw new Error(`${key} is required.`);
  }

  return value;
}

function requireString(value: string | undefined, key: string): string {
  if (!value) {
    throw new Error(`${key} is required.`);
  }

  return value;
}

function readJson<T>(env: Env, key: string): T | undefined {
  const value = readTrimmed(env, key);
  if (!value) {
    return undefined;
  }

  try {
    return JSON.parse(value) as T;
  } catch (error) {
    const message = error instanceof Error ? error.message : "invalid JSON";
    throw new Error(`${key} must contain valid JSON: ${message}`);
  }
}

function maybeLoadEmbeddedOidcConfig(
  env: Env,
  publicBaseUrl: URL | undefined,
): EmbeddedOidcAuthConfig | undefined {
  if (readTrimmed(env, "CHATCODEX_OAUTH_PROVIDER") !== EMBEDDED_OIDC_PROVIDER) {
    return undefined;
  }

  if (!publicBaseUrl) {
    throw new Error("Embedded OIDC provider mode requires PUBLIC_BASE_URL.");
  }

  const issuerPath = readTrimmed(env, "OIDC_PROVIDER_ISSUER_PATH") ?? DEFAULT_EMBEDDED_OIDC_PATH;
  const issuerUrl = new URL(issuerPath, publicBaseUrl);
  const resourceServerUrl =
    readUrl(env, "OAUTH_RESOURCE_SERVER_URL") ?? new URL(DEFAULT_MCP_PATH, publicBaseUrl);
  const jwks = readJson<{ keys: JsonWebKey[] }>(env, "OIDC_PROVIDER_JWKS_JSON");
  const cookieKeys = readCsv(env, "OIDC_PROVIDER_COOKIE_KEYS");
  const teamDomain = normalizeCloudflareTeamDomain(
    requireString(
      readTrimmed(env, "CLOUDFLARE_ACCESS_TEAM_DOMAIN"),
      "CLOUDFLARE_ACCESS_TEAM_DOMAIN",
    ),
  );

  if (!jwks || !Array.isArray(jwks.keys) || jwks.keys.length === 0) {
    throw new Error("OIDC_PROVIDER_JWKS_JSON must be a JWK set with at least one key.");
  }

  if (cookieKeys.length === 0) {
    throw new Error("OIDC_PROVIDER_COOKIE_KEYS must include at least one signing key.");
  }

  const scopesSupported = readCsv(env, "OAUTH_SCOPES_SUPPORTED", [DEFAULT_SCOPE]);
  const requiredScopes = readCsv(env, "OAUTH_REQUIRED_SCOPES", [...scopesSupported]);

  return {
    mode: "oauth",
    provider: "embedded-oidc",
    issuerUrl,
    resourceServerUrl,
    authorizationEndpoint: new URL("authorize", `${issuerUrl.href}/`),
    tokenEndpoint: new URL("token", `${issuerUrl.href}/`),
    registrationEndpoint: new URL("register", `${issuerUrl.href}/`),
    revocationEndpoint: new URL("revoke", `${issuerUrl.href}/`),
    jwksUrl: new URL("jwks", `${issuerUrl.href}/`),
    storagePath: readTrimmed(env, "OIDC_PROVIDER_STORAGE_PATH") ?? DEFAULT_OIDC_STORAGE_PATH,
    cookieKeys,
    jwks,
    cimdAllowedHosts: readCsv(env, "OIDC_CIMD_ALLOWED_HOSTS", DEFAULT_CIMD_ALLOWED_HOSTS),
    scopesSupported,
    requiredScopes,
    login: {
      provider: "cloudflare-access",
      teamDomain,
      audience: requireString(
        readTrimmed(env, "OIDC_LOGIN_CLOUDFLARE_ACCESS_AUDIENCE"),
        "OIDC_LOGIN_CLOUDFLARE_ACCESS_AUDIENCE",
      ),
      emailClaim: readTrimmed(env, "OIDC_LOGIN_EMAIL_CLAIM") ?? "email",
    },
  };
}

function maybeLoadCloudflareAccessConfig(
  env: Env,
  publicBaseUrl: URL | undefined,
): ExternalOAuthAuthConfig | undefined {
  const provider = readTrimmed(env, "CHATCODEX_OAUTH_PROVIDER");
  const hasCloudflareHints =
    provider === CLOUDFLARE_ACCESS_PROVIDER ||
    readTrimmed(env, "CLOUDFLARE_ACCESS_TEAM_DOMAIN") !== undefined ||
    readTrimmed(env, "CLOUDFLARE_ACCESS_CLIENT_ID") !== undefined;

  if (!hasCloudflareHints) {
    return undefined;
  }

  const teamDomain = normalizeCloudflareTeamDomain(
    requireString(
      readTrimmed(env, "CLOUDFLARE_ACCESS_TEAM_DOMAIN"),
      "CLOUDFLARE_ACCESS_TEAM_DOMAIN",
    ),
  );
  const clientId = requireString(
    readTrimmed(env, "CLOUDFLARE_ACCESS_CLIENT_ID"),
    "CLOUDFLARE_ACCESS_CLIENT_ID",
  );
  const issuerUrl = new URL(`/cdn-cgi/access/sso/oidc/${clientId}`, teamDomain);

  const resourceServerUrl =
    readUrl(env, "OAUTH_RESOURCE_SERVER_URL") ??
    (publicBaseUrl ? new URL(DEFAULT_MCP_PATH, publicBaseUrl) : undefined);

  if (!resourceServerUrl) {
    throw new Error(
      "CLOUDFLARE Access OAuth requires OAUTH_RESOURCE_SERVER_URL or PUBLIC_BASE_URL.",
    );
  }

  const scopesSupported = readCsv(env, "OAUTH_SCOPES_SUPPORTED");
  const requiredScopes = readCsv(env, "OAUTH_REQUIRED_SCOPES");

  return {
    mode: "oauth",
    provider: "cloudflare-access",
    issuerUrl,
    resourceServerUrl,
    discoveryUrl: new URL(".well-known/openid-configuration", `${issuerUrl.href}/`),
    authorizationEndpoint: new URL("authorization", `${issuerUrl.href}/`),
    tokenEndpoint: new URL("token", `${issuerUrl.href}/`),
    jwksUrl: new URL("jwks", `${issuerUrl.href}/`),
    serviceDocumentationUrl: new URL(
      "https://developers.cloudflare.com/cloudflare-one/access-controls/ai-controls/saas-mcp/",
    ),
    scopesSupported,
    requiredScopes,
    audience: readTrimmed(env, "CLOUDFLARE_ACCESS_AUDIENCE") ?? readTrimmed(env, "OAUTH_AUDIENCE"),
    jwtAlgorithms: readCsv(env, "OAUTH_JWT_ALGORITHMS", ["RS256"]),
  };
}

function loadOAuthConfig(env: Env, publicBaseUrl: URL | undefined): OAuthAuthConfig {
  const embeddedOidcConfig = maybeLoadEmbeddedOidcConfig(env, publicBaseUrl);
  if (embeddedOidcConfig) {
    return embeddedOidcConfig;
  }

  const cloudflareAccessConfig = maybeLoadCloudflareAccessConfig(env, publicBaseUrl);
  if (cloudflareAccessConfig) {
    return cloudflareAccessConfig;
  }

  const issuerUrl = requireUrl(readUrl(env, "OAUTH_ISSUER_URL"), "OAUTH_ISSUER_URL");
  const resourceServerUrl =
    readUrl(env, "OAUTH_RESOURCE_SERVER_URL") ??
    (publicBaseUrl ? new URL(DEFAULT_MCP_PATH, publicBaseUrl) : undefined);

  if (!resourceServerUrl) {
    throw new Error(
      "OAUTH_RESOURCE_SERVER_URL or PUBLIC_BASE_URL is required for OAuth mode.",
    );
  }

  const discoveryUrl = readUrl(env, "OAUTH_DISCOVERY_URL");
  const authorizationEndpoint = readUrl(env, "OAUTH_AUTHORIZATION_ENDPOINT");
  const tokenEndpoint = readUrl(env, "OAUTH_TOKEN_ENDPOINT");
  const introspectionEndpoint = readUrl(env, "OAUTH_INTROSPECTION_ENDPOINT");
  const jwksUrl = readUrl(env, "OAUTH_JWKS_URL");

  if (!discoveryUrl && (!authorizationEndpoint || !tokenEndpoint)) {
    throw new Error(
      "OAuth mode requires OAUTH_DISCOVERY_URL, or both OAUTH_AUTHORIZATION_ENDPOINT and OAUTH_TOKEN_ENDPOINT.",
    );
  }

  if (!introspectionEndpoint && !jwksUrl && !discoveryUrl) {
    throw new Error(
      "OAuth mode requires token verification via OAUTH_INTROSPECTION_ENDPOINT, OAUTH_JWKS_URL, or OAUTH_DISCOVERY_URL.",
    );
  }

  const scopesSupported = readCsv(env, "OAUTH_SCOPES_SUPPORTED", [DEFAULT_SCOPE]);
  const requiredScopes = readCsv(env, "OAUTH_REQUIRED_SCOPES", [...scopesSupported]);

  return {
    mode: "oauth",
    provider: "generic",
    issuerUrl,
    resourceServerUrl,
    discoveryUrl,
    authorizationEndpoint,
    tokenEndpoint,
    registrationEndpoint: readUrl(env, "OAUTH_REGISTRATION_ENDPOINT"),
    revocationEndpoint: readUrl(env, "OAUTH_REVOCATION_ENDPOINT"),
    introspectionEndpoint,
    jwksUrl,
    serviceDocumentationUrl: readUrl(env, "OAUTH_SERVICE_DOCUMENTATION_URL"),
    scopesSupported,
    requiredScopes,
    audience: readTrimmed(env, "OAUTH_AUDIENCE"),
    introspectionClientId: readTrimmed(env, "OAUTH_INTROSPECTION_CLIENT_ID"),
    introspectionClientSecret: readTrimmed(env, "OAUTH_INTROSPECTION_CLIENT_SECRET"),
    jwtAlgorithms: readCsv(env, "OAUTH_JWT_ALGORITHMS"),
  };
}

export function loadServerConfig(env: Env = process.env): ServerRuntimeConfig {
  const host = readTrimmed(env, "HOST") ?? DEFAULT_HOST;
  const port = readPositiveInteger(env, "PORT", DEFAULT_PORT);
  const allowedHosts = readCsv(env, "MCP_ALLOWED_HOSTS");
  const publicBaseUrl = readUrl(env, "PUBLIC_BASE_URL");
  const authMode = inferAuthMode(env);

  const auth: AuthConfig =
    authMode === "oauth"
      ? loadOAuthConfig(env, publicBaseUrl)
      : authMode === "static-token"
        ? {
            mode: "static-token",
            token: requireString(readTrimmed(env, "MCP_AUTH_TOKEN"), "MCP_AUTH_TOKEN"),
          }
        : { mode: "none" };

  return {
    host,
    port,
    allowedHosts,
    publicBaseUrl,
    mcpPath: DEFAULT_MCP_PATH,
    healthzPath: DEFAULT_HEALTHZ_PATH,
    daemonUrl: readTrimmed(env, "DETERMINISTIC_DAEMON_URL"),
    auth,
  };
}
