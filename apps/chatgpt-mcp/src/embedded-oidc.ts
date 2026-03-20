import { mkdirSync } from "node:fs";
import { dirname } from "node:path";
import Database from "better-sqlite3";
import type { Request, Response, Router } from "express";
import express from "express";
import { createRemoteJWKSet, jwtVerify } from "jose";
import { Provider } from "oidc-provider";
import type { OAuthMetadata } from "@modelcontextprotocol/sdk/shared/auth.js";
import type { OAuthTokenVerifier } from "@modelcontextprotocol/sdk/server/auth/provider.js";
import type { AuthInfo } from "@modelcontextprotocol/sdk/server/auth/types.js";
import type {
  EmbeddedOidcAuthConfig,
  EmbeddedOidcCloudflareAccessLoginConfig,
} from "./config.js";

export interface EmbeddedOidcRuntime {
  router: Router;
  oauthMetadata: OAuthMetadata;
  verifier: OAuthTokenVerifier;
  requiredScopes: string[];
  scopesSupported: string[];
  resourceServerUrl: URL;
  close(): Promise<void>;
}

interface SqliteRecord {
  payload: string;
  expires_at: number | null;
}

const CLOUDFLARE_ACCESS_JWT_HEADER = "cf-access-jwt-assertion";
const CLOUDFLARE_CERTS_PATH = "/cdn-cgi/access/certs";
const CLOUDLFARE_CLAIM_SUBJECT = "sub";
const CIMD_ACKNOWLEDGEMENT = "draft-01";
const REGISTRATION_ACCESS_TOKEN_TTL_SECONDS = 60 * 60 * 24 * 30;
const AUTHORIZATION_CODE_TTL_SECONDS = 60 * 5;
const ACCESS_TOKEN_TTL_SECONDS = 60 * 15;
const REFRESH_TOKEN_TTL_SECONDS = 60 * 60 * 24 * 30;

function epochTime(): number {
  return Math.floor(Date.now() / 1000);
}

function withOptionalTrailingSlash(url: URL): [string, string] {
  const withSlash = url.href;
  const withoutSlash = withSlash.endsWith("/") ? withSlash.slice(0, -1) : withSlash;
  return [withSlash, withoutSlash];
}

function normalizeResource(value: unknown): URL | undefined {
  if (typeof value === "string") {
    return new URL(value);
  }

  if (Array.isArray(value)) {
    const first = value.find((entry): entry is string => typeof entry === "string");
    return first ? new URL(first) : undefined;
  }

  return undefined;
}

function parseScopes(value: unknown): string[] {
  if (typeof value !== "string") {
    return [];
  }

  return value
    .split(" ")
    .map((scope) => scope.trim())
    .filter((scope) => scope.length > 0);
}

function buildAuthInfo(token: string, accessToken: Record<string, unknown>): AuthInfo {
  return {
    token,
    clientId: typeof accessToken.clientId === "string" ? accessToken.clientId : "unknown-client",
    scopes: parseScopes(accessToken.scope),
    expiresAt: typeof accessToken.exp === "number" ? accessToken.exp : undefined,
    resource: normalizeResource(accessToken.aud),
    extra: {
      subject:
        typeof accessToken.accountId === "string"
          ? accessToken.accountId
          : typeof accessToken[CLOUDLFARE_CLAIM_SUBJECT] === "string"
            ? accessToken[CLOUDLFARE_CLAIM_SUBJECT]
            : undefined,
    },
  };
}

function initializeSchema(db: Database.Database): void {
  db.exec(`
    CREATE TABLE IF NOT EXISTS oidc_store (
      model TEXT NOT NULL,
      id TEXT NOT NULL,
      payload TEXT NOT NULL,
      expires_at INTEGER,
      grant_id TEXT,
      user_code TEXT,
      uid TEXT,
      PRIMARY KEY (model, id)
    );
    CREATE INDEX IF NOT EXISTS oidc_store_expires_at_idx ON oidc_store (expires_at);
    CREATE INDEX IF NOT EXISTS oidc_store_grant_id_idx ON oidc_store (grant_id);
    CREATE INDEX IF NOT EXISTS oidc_store_uid_idx ON oidc_store (uid);
    CREATE INDEX IF NOT EXISTS oidc_store_user_code_idx ON oidc_store (user_code);
  `);
  db.prepare("DELETE FROM oidc_store WHERE expires_at IS NOT NULL AND expires_at <= ?").run(epochTime());
}

class SqliteAdapter {
  private readonly model: string;

  private readonly db: Database.Database;

  public constructor(model: string, db: Database.Database) {
    this.model = model;
    this.db = db;
  }

  public async destroy(id: string): Promise<void> {
    this.db
      .prepare("DELETE FROM oidc_store WHERE model = ? AND id = ?")
      .run(this.model, id);
  }

  public async consume(id: string): Promise<void> {
    const stored = this.db
      .prepare("SELECT payload FROM oidc_store WHERE model = ? AND id = ?")
      .get(this.model, id) as Pick<SqliteRecord, "payload"> | undefined;

    if (!stored) {
      return;
    }

    const payload = JSON.parse(stored.payload) as Record<string, unknown>;
    payload.consumed = epochTime();

    this.db
      .prepare("UPDATE oidc_store SET payload = ? WHERE model = ? AND id = ?")
      .run(JSON.stringify(payload), this.model, id);
  }

  public async find(id: string): Promise<Record<string, unknown> | undefined> {
    const row = this.db
      .prepare(
        "SELECT payload, expires_at FROM oidc_store WHERE model = ? AND id = ?",
      )
      .get(this.model, id) as SqliteRecord | undefined;

    return this.deserialize(row);
  }

  public async findByUid(uid: string): Promise<Record<string, unknown> | undefined> {
    const row = this.db
      .prepare(
        "SELECT payload, expires_at FROM oidc_store WHERE model = ? AND uid = ? ORDER BY expires_at DESC LIMIT 1",
      )
      .get(this.model, uid) as SqliteRecord | undefined;

    return this.deserialize(row);
  }

  public async findByUserCode(userCode: string): Promise<Record<string, unknown> | undefined> {
    const row = this.db
      .prepare(
        "SELECT payload, expires_at FROM oidc_store WHERE model = ? AND user_code = ? ORDER BY expires_at DESC LIMIT 1",
      )
      .get(this.model, userCode) as SqliteRecord | undefined;

    return this.deserialize(row);
  }

  public async revokeByGrantId(grantId: string): Promise<void> {
    this.db
      .prepare("DELETE FROM oidc_store WHERE grant_id = ?")
      .run(grantId);
  }

  public async upsert(
    id: string,
    payload: Record<string, unknown>,
    expiresIn: number,
  ): Promise<void> {
    const expiresAt = epochTime() + expiresIn;
    const grantId = typeof payload.grantId === "string" ? payload.grantId : null;
    const userCode = typeof payload.userCode === "string" ? payload.userCode : null;
    const uid = typeof payload.uid === "string" ? payload.uid : null;

    this.db
      .prepare(`
        INSERT INTO oidc_store (model, id, payload, expires_at, grant_id, user_code, uid)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(model, id) DO UPDATE SET
          payload = excluded.payload,
          expires_at = excluded.expires_at,
          grant_id = excluded.grant_id,
          user_code = excluded.user_code,
          uid = excluded.uid
      `)
      .run(this.model, id, JSON.stringify(payload), expiresAt, grantId, userCode, uid);
  }

  private deserialize(row: SqliteRecord | undefined): Record<string, unknown> | undefined {
    if (!row) {
      return undefined;
    }

    if (row.expires_at !== null && row.expires_at <= epochTime()) {
      return undefined;
    }

    return JSON.parse(row.payload) as Record<string, unknown>;
  }
}

async function verifyCloudflareAccessIdentity(
  req: Request,
  config: EmbeddedOidcCloudflareAccessLoginConfig,
): Promise<{ accountId: string; email: string }> {
  const token = req.header(CLOUDFLARE_ACCESS_JWT_HEADER);
  if (!token) {
    throw new Error(
      "Missing Cloudflare Access identity. Protect the OIDC interaction routes with a Cloudflare Access application.",
    );
  }

  const jwks = createRemoteJWKSet(new URL(CLOUDFLARE_CERTS_PATH, config.teamDomain));
  const { payload } = await jwtVerify(token, jwks, {
    issuer: withOptionalTrailingSlash(config.teamDomain),
    audience: config.audience,
  });
  const emailClaim = payload[config.emailClaim];
  if (typeof emailClaim !== "string" || emailClaim.length === 0) {
    throw new Error(`Cloudflare Access token missing "${config.emailClaim}" claim.`);
  }

  return {
    accountId: emailClaim.toLowerCase(),
    email: emailClaim.toLowerCase(),
  };
}

async function finishConsent(provider: Provider, req: Request, res: Response): Promise<void> {
  const interaction = await provider.interactionDetails(req, res);
  const accountId = interaction.session?.accountId;

  if (!accountId) {
    throw new Error("OIDC consent prompt is missing a logged-in account.");
  }

  let grant = interaction.grantId
    ? await provider.Grant.find(interaction.grantId)
    : undefined;

  if (!grant) {
    grant = new provider.Grant({
      accountId,
      clientId: String(interaction.params.client_id),
    });
  }

  const details = interaction.prompt.details as {
    missingOIDCScope?: string[];
    missingOIDCClaims?: string[];
    missingResourceScopes?: Record<string, string[]>;
    missingResourceIndicators?: string[];
  };

  if (details.missingOIDCScope && details.missingOIDCScope.length > 0) {
    grant.addOIDCScope(details.missingOIDCScope.join(" "));
  }

  if (details.missingOIDCClaims && details.missingOIDCClaims.length > 0) {
    grant.addOIDCClaims(details.missingOIDCClaims);
  }

  if (details.missingResourceScopes) {
    for (const [resource, scopes] of Object.entries(details.missingResourceScopes)) {
      grant.addResourceScope(resource, scopes.join(" "));
    }
  }

  if (details.missingResourceIndicators) {
    for (const resource of details.missingResourceIndicators) {
      grant.addResourceScope(resource, "");
    }
  }

  const grantId = await grant.save();
  await provider.interactionFinished(
    req,
    res,
    { consent: { grantId } },
    { mergeWithLastSubmission: true },
  );
}

function createOidcRouter(
  provider: Provider,
  config: EmbeddedOidcAuthConfig,
): Router {
  const router = express.Router();

  router.get("/interaction/:uid", async (req, res, next) => {
    try {
      const interaction = await provider.interactionDetails(req, res);
      if (interaction.prompt.name === "login") {
        const identity = await verifyCloudflareAccessIdentity(req, config.login);
        await provider.interactionFinished(
          req,
          res,
          {
            login: {
              accountId: identity.accountId,
              remember: false,
              ts: epochTime(),
            },
          },
          { mergeWithLastSubmission: false },
        );
        return;
      }

      if (interaction.prompt.name === "consent") {
        await finishConsent(provider, req, res);
        return;
      }

      throw new Error(`Unsupported OIDC interaction prompt: ${interaction.prompt.name}`);
    } catch (error) {
      next(error);
    }
  });

  router.use(provider.callback());
  return router;
}

function createAccount(accountId: string) {
  const email = accountId.toLowerCase();
  return {
    accountId,
    async claims() {
      const preferredUsername = email.includes("@") ? email.split("@", 1)[0] : email;
      return {
        sub: accountId,
        email,
        email_verified: true,
        preferred_username: preferredUsername,
        name: preferredUsername,
      };
    },
  };
}

function createProvider(config: EmbeddedOidcAuthConfig, db: Database.Database): Provider {
  const issuerPath = config.issuerUrl.pathname.replace(/\/$/, "");
  const interactionBasePath = `${issuerPath}/interaction`;
  const features = {
    devInteractions: {
      enabled: false,
    },
    resourceIndicators: {
      enabled: true,
      defaultResource() {
        return config.resourceServerUrl.href;
      },
      useGrantedResource() {
        return true;
      },
      async getResourceServerInfo(
        _ctx: unknown,
        resourceIndicator: string,
        _client: unknown,
      ) {
        if (resourceIndicator !== config.resourceServerUrl.href) {
          throw new Error("unknown resource indicator");
        }

        return {
          audience: config.resourceServerUrl.href,
          scope: config.scopesSupported.join(" "),
        };
      },
    },
    clientIdMetadataDocument: {
      ack: CIMD_ACKNOWLEDGEMENT,
      enabled: true,
      async allowFetch(_ctx: unknown, clientId: string) {
        const hostname = new URL(clientId).hostname.toLowerCase();
        return config.cimdAllowedHosts.some(
          (allowedHost) =>
            hostname === allowedHost || hostname.endsWith(`.${allowedHost}`),
        );
      },
    },
    registration: {
      enabled: true,
      initialAccessToken: false,
    },
    registrationManagement: {
      enabled: true,
    },
    revocation: {
      enabled: true,
    },
  } as unknown;

  const provider = new Provider(config.issuerUrl.href, {
    adapter: (modelName: string) => new SqliteAdapter(modelName, db),
    cookies: { keys: config.cookieKeys },
    discovery: {
      service_documentation: config.serviceDocumentationUrl?.href,
    },
    features: features as never,
    findAccount(_ctx: unknown, sub: string) {
      return createAccount(sub) as never;
    },
    interactions: {
      url(_ctx: unknown, interaction: { uid: string }) {
        return `${interactionBasePath}/${interaction.uid}`;
      },
    },
    jwks: config.jwks,
    loadExistingGrant: async (ctx: {
      oidc: {
        client?: { clientId: string };
        provider: Provider;
        result?: { consent?: { grantId?: string } };
        session?: {
          accountId?: string;
          grantIdFor(clientId: string): string | undefined;
        };
      };
    }) => {
      const clientId = ctx.oidc.client?.clientId;
      if (!clientId) {
        return undefined;
      }

      const grantId =
        ctx.oidc.result?.consent?.grantId ?? ctx.oidc.session?.grantIdFor(clientId);

      if (grantId) {
        return ctx.oidc.provider.Grant.find(grantId);
      }

      const accountId = ctx.oidc.session?.accountId;
      if (!accountId) {
        return undefined;
      }

      const grant = new ctx.oidc.provider.Grant({
        accountId,
        clientId,
      });
      grant.addOIDCScope("openid offline_access");
      grant.addResourceScope(config.resourceServerUrl.href, config.scopesSupported.join(" "));
      await grant.save();
      return grant;
    },
    pkce: {
      required: () => true,
    },
    responseTypes: ["code"],
    routes: {
      authorization: "/authorize",
      jwks: "/jwks",
      registration: "/register",
      revocation: "/revoke",
      token: "/token",
    },
    ttl: {
      AccessToken: ACCESS_TOKEN_TTL_SECONDS,
      AuthorizationCode: AUTHORIZATION_CODE_TTL_SECONDS,
      RefreshToken: REFRESH_TOKEN_TTL_SECONDS,
      RegistrationAccessToken: REGISTRATION_ACCESS_TOKEN_TTL_SECONDS,
    },
  });

  provider.proxy = true;
  return provider;
}

function createOAuthMetadata(config: EmbeddedOidcAuthConfig): OAuthMetadata {
  return {
    issuer: config.issuerUrl.href,
    authorization_endpoint: config.authorizationEndpoint.href,
    token_endpoint: config.tokenEndpoint.href,
    registration_endpoint: config.registrationEndpoint.href,
    revocation_endpoint: config.revocationEndpoint.href,
    jwks_uri: config.jwksUrl.href,
    response_types_supported: ["code"],
    grant_types_supported: ["authorization_code", "refresh_token"],
    code_challenge_methods_supported: ["S256"],
    token_endpoint_auth_methods_supported: ["none", "client_secret_post", "client_secret_basic"],
    scopes_supported: config.scopesSupported,
    service_documentation: config.serviceDocumentationUrl?.href,
    client_id_metadata_document_supported: true,
  };
}

function createVerifier(provider: Provider): OAuthTokenVerifier {
  return {
    verifyAccessToken: async (token: string) => {
      const accessToken = await provider.AccessToken.find(token);
      if (!accessToken) {
        throw new Error("OAuth access token is invalid or expired.");
      }

      return buildAuthInfo(token, accessToken as unknown as Record<string, unknown>);
    },
  };
}

export async function initializeEmbeddedOidcRuntime(
  config: EmbeddedOidcAuthConfig,
): Promise<EmbeddedOidcRuntime> {
  mkdirSync(dirname(config.storagePath), { recursive: true });
  const db = new Database(config.storagePath);
  initializeSchema(db);

  const provider = createProvider(config, db);
  const router = createOidcRouter(provider, config);

  return {
    router,
    oauthMetadata: createOAuthMetadata(config),
    verifier: createVerifier(provider),
    requiredScopes: config.requiredScopes,
    scopesSupported: config.scopesSupported,
    resourceServerUrl: config.resourceServerUrl,
    close: async () => {
      db.close();
    },
  };
}
