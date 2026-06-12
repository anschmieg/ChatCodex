//! SQLite-backed persistence for OAuth 2.1 clients, codes, tokens, and the
//! resource server's signing keys.
//!
//! All operations are synchronous and intended to be invoked behind
//! `tokio::task::spawn_blocking` by callers. The store handles its own
//! migration on first open.

use std::path::Path;
use std::sync::Mutex;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pkcs8::EncodePrivateKey;
use pkcs8::EncodePublicKey;
use rand::RngCore;
use rsa::RsaPrivateKey;
use rsa::RsaPublicKey;
use rsa::traits::PublicKeyParts;
use rusqlite::Connection;
use rusqlite::OptionalExtension;
use rusqlite::params;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;

use crate::keyring;
use sha2::Sha256;

/// Unix-seconds timestamp for "now".
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// SHA-256 of an opaque token, hex-encoded.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

/// Generate a fresh opaque token (32 random bytes, base64url-encoded).
pub fn new_opaque_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// An OAuth client registered through RFC 7591 dynamic registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRecord {
    pub client_id: String,
    pub client_secret_hash: Option<String>,
    pub client_name: Option<String>,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub token_endpoint_auth_method: String,
    pub created_at: i64,
    pub disabled_at: Option<i64>,
}

/// A pending authorization code (single-use).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthCodeRecord {
    pub code_hash: String,
    pub client_id: String,
    pub subject: String,
    pub redirect_uri: String,
    pub scope: String,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub expires_at: i64,
    pub consumed_at: Option<i64>,
}

/// A refresh token row (opaque token, hashed).
#[derive(Debug, Clone)]
pub struct RefreshTokenRecord {
    pub token_hash: String,
    pub client_id: String,
    pub subject: String,
    pub scope: String,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
    pub replaced_by: Option<String>,
}

/// A JTI revocation record for an access token.
#[derive(Debug, Clone)]
pub struct AccessTokenJtiRecord {
    pub jti: String,
    pub client_id: String,
    pub subject: String,
    pub expires_at: i64,
    pub revoked_at: Option<i64>,
}

/// Persisted signing key, only the public material is exposed to callers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigningKeyRecord {
    pub kid: String,
    pub alg: String,
    pub private_pem: String,
    pub public_jwk: serde_json::Value,
    pub created_at: i64,
    pub is_active: bool,
}

/// All persistent state for the auth layer.
#[derive(Clone)]
pub struct Store {
    conn: std::sync::Arc<Mutex<Connection>>,
}

impl Store {
    /// Open (or create) the auth database at `dir/oauth.db`.
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir).context("cannot create auth store directory")?;
        let db_path = dir.join("oauth.db");
        let conn = Connection::open(&db_path).with_context(|| {
            format!("cannot open OAuth SQLite database at {}", db_path.display())
        })?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        Self::migrate(&conn)?;
        // Checkpoint the WAL so data is in the main DB file, ensuring
        // persistence across container restarts even if the WAL is lost.
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(Self {
            conn: std::sync::Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory database for tests.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("cannot open in-memory SQLite")?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: std::sync::Arc::new(Mutex::new(conn)),
        })
    }

    fn migrate(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS clients (
                client_id TEXT PRIMARY KEY,
                client_secret_hash TEXT,
                client_name TEXT,
                redirect_uris TEXT NOT NULL,
                grant_types TEXT NOT NULL,
                token_endpoint_auth_method TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                disabled_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS authorization_codes (
                code_hash TEXT PRIMARY KEY,
                client_id TEXT NOT NULL,
                subject TEXT NOT NULL,
                redirect_uri TEXT NOT NULL,
                scope TEXT NOT NULL,
                code_challenge TEXT,
                code_challenge_method TEXT,
                expires_at INTEGER NOT NULL,
                consumed_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS refresh_tokens (
                token_hash TEXT PRIMARY KEY,
                client_id TEXT NOT NULL,
                subject TEXT NOT NULL,
                scope TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                revoked_at INTEGER,
                replaced_by TEXT
            );
            CREATE TABLE IF NOT EXISTS access_token_jti (
                jti TEXT PRIMARY KEY,
                client_id TEXT NOT NULL,
                subject TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                revoked_at INTEGER
            );
            CREATE TABLE IF NOT EXISTS signing_keys (
                kid TEXT PRIMARY KEY,
                alg TEXT NOT NULL,
                private_pem TEXT NOT NULL,
                public_jwk TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                is_active INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS refresh_tokens_client_subject_idx
                ON refresh_tokens(client_id, subject);
            CREATE INDEX IF NOT EXISTS access_token_jti_expires_idx
                ON access_token_jti(expires_at);
            CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY);
            INSERT OR IGNORE INTO schema_version (version) VALUES (1);
            ",
        )?;
        Ok(())
    }

    fn with_conn<R>(&self, op: impl FnOnce(&mut Connection) -> Result<R>) -> Result<R> {
        let mut guard = self
            .conn
            .lock()
            .map_err(|error| anyhow::anyhow!("auth store lock poisoned: {error}"))?;
        op(&mut guard)
    }

    /// Insert a newly-registered client.
    pub fn insert_client(&self, record: &ClientRecord) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO clients
                    (client_id, client_secret_hash, client_name, redirect_uris, grant_types,
                     token_endpoint_auth_method, created_at, disabled_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    record.client_id,
                    record.client_secret_hash,
                    record.client_name,
                    serde_json::to_string(&record.redirect_uris)?,
                    serde_json::to_string(&record.grant_types)?,
                    record.token_endpoint_auth_method,
                    record.created_at,
                    record.disabled_at,
                ],
            )?;
            Ok(())
        })
    }

    /// Look up a client by its public id.
    pub fn get_client(&self, client_id: &str) -> Result<Option<ClientRecord>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT client_id, client_secret_hash, client_name, redirect_uris,
                            grant_types, token_endpoint_auth_method, created_at, disabled_at
                     FROM clients WHERE client_id = ?1",
                    params![client_id],
                    Self::row_to_client,
                )
                .optional()?;
            Ok(row)
        })
    }

    fn row_to_client(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClientRecord> {
        let redirect_uris: String = row.get(3)?;
        let grant_types: String = row.get(4)?;
        Ok(ClientRecord {
            client_id: row.get(0)?,
            client_secret_hash: row.get(1)?,
            client_name: row.get(2)?,
            redirect_uris: serde_json::from_str(&redirect_uris).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
            })?,
            grant_types: serde_json::from_str(&grant_types).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
            })?,
            token_endpoint_auth_method: row.get(5)?,
            created_at: row.get(6)?,
            disabled_at: row.get(7)?,
        })
    }

    /// Insert a fresh authorization code.
    pub fn insert_auth_code(&self, record: &AuthCodeRecord) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO authorization_codes
                    (code_hash, client_id, subject, redirect_uri, scope, code_challenge,
                     code_challenge_method, expires_at, consumed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    record.code_hash,
                    record.client_id,
                    record.subject,
                    record.redirect_uri,
                    record.scope,
                    record.code_challenge,
                    record.code_challenge_method,
                    record.expires_at,
                    record.consumed_at,
                ],
            )?;
            Ok(())
        })
    }

    /// Atomically consume an authorization code if it exists, is unused, and
    /// has not expired. Returns the record that was consumed.
    pub fn consume_auth_code(&self, code_hash: &str, now_ts: i64) -> Result<Option<AuthCodeRecord>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT code_hash, client_id, subject, redirect_uri, scope, code_challenge,
                        code_challenge_method, expires_at, consumed_at
                 FROM authorization_codes WHERE code_hash = ?1",
            )?;
            let record: Option<AuthCodeRecord> = stmt
                .query_row(params![code_hash], |row| {
                    Ok(AuthCodeRecord {
                        code_hash: row.get(0)?,
                        client_id: row.get(1)?,
                        subject: row.get(2)?,
                        redirect_uri: row.get(3)?,
                        scope: row.get(4)?,
                        code_challenge: row.get(5)?,
                        code_challenge_method: row.get(6)?,
                        expires_at: row.get(7)?,
                        consumed_at: row.get(8)?,
                    })
                })
                .optional()?;

            let Some(record) = record else {
                return Ok(None);
            };
            if record.consumed_at.is_some() || record.expires_at <= now_ts {
                return Ok(None);
            }
            conn.execute(
                "UPDATE authorization_codes SET consumed_at = ?1 WHERE code_hash = ?2",
                params![now_ts, code_hash],
            )?;
            Ok(Some(record))
        })
    }

    /// Insert a refresh token.
    pub fn insert_refresh_token(&self, record: &RefreshTokenRecord) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO refresh_tokens
                    (token_hash, client_id, subject, scope, expires_at, revoked_at, replaced_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    record.token_hash,
                    record.client_id,
                    record.subject,
                    record.scope,
                    record.expires_at,
                    record.revoked_at,
                    record.replaced_by,
                ],
            )?;
            Ok(())
        })
    }

    /// Look up a refresh token by its hash.
    pub fn get_refresh_token(&self, token_hash: &str) -> Result<Option<RefreshTokenRecord>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT token_hash, client_id, subject, scope, expires_at, revoked_at,
                            replaced_by
                     FROM refresh_tokens WHERE token_hash = ?1",
                    params![token_hash],
                    |row| {
                        Ok(RefreshTokenRecord {
                            token_hash: row.get(0)?,
                            client_id: row.get(1)?,
                            subject: row.get(2)?,
                            scope: row.get(3)?,
                            expires_at: row.get(4)?,
                            revoked_at: row.get(5)?,
                            replaced_by: row.get(6)?,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
    }

    /// Revoke a refresh token.
    pub fn revoke_refresh_token(&self, token_hash: &str, now_ts: i64) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE refresh_tokens SET revoked_at = COALESCE(revoked_at, ?1)
                 WHERE token_hash = ?2",
                params![now_ts, token_hash],
            )?;
            Ok(())
        })
    }

    /// Insert a JTI for an access token.
    pub fn insert_access_jti(&self, record: &AccessTokenJtiRecord) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO access_token_jti
                    (jti, client_id, subject, expires_at, revoked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    record.jti,
                    record.client_id,
                    record.subject,
                    record.expires_at,
                    record.revoked_at,
                ],
            )?;
            Ok(())
        })
    }

    /// Check whether a JTI is active (not revoked, not expired).
    pub fn jti_is_active(&self, jti: &str, now_ts: i64) -> Result<bool> {
        self.with_conn(|conn| {
            let row: Option<(Option<i64>, i64)> = conn
                .query_row(
                    "SELECT revoked_at, expires_at FROM access_token_jti WHERE jti = ?1",
                    params![jti],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            Ok(matches!(row, Some((None, expires)) if expires > now_ts))
        })
    }

    /// Revoke a JTI.
    pub fn revoke_jti(&self, jti: &str, now_ts: i64) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute(
                "UPDATE access_token_jti SET revoked_at = COALESCE(revoked_at, ?1)
                 WHERE jti = ?2",
                params![now_ts, jti],
            )?;
            Ok(())
        })
    }

    /// Look up a JTI for introspection.
    pub fn get_jti(&self, jti: &str) -> Result<Option<AccessTokenJtiRecord>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT jti, client_id, subject, expires_at, revoked_at
                     FROM access_token_jti WHERE jti = ?1",
                    params![jti],
                    |row| {
                        Ok(AccessTokenJtiRecord {
                            jti: row.get(0)?,
                            client_id: row.get(1)?,
                            subject: row.get(2)?,
                            expires_at: row.get(3)?,
                            revoked_at: row.get(4)?,
                        })
                    },
                )
                .optional()?;
            Ok(row)
        })
    }

    fn load_active_signing_key(&self) -> Result<Option<SigningKeyRecord>> {
        self.with_conn(|conn| {
            let row: Option<(String, String, String, String, i64, i64)> = conn
                .query_row(
                    "SELECT kid, alg, private_pem, public_jwk, created_at, is_active
                     FROM signing_keys WHERE is_active = 1 LIMIT 1",
                    [],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    },
                )
                .ok();
            Ok(row.and_then(
                |(kid, alg, private_pem, public_jwk, created_at, is_active)| {
                    let public: serde_json::Value = serde_json::from_str(&public_jwk).ok()?;
                    Some(SigningKeyRecord {
                        kid,
                        alg,
                        private_pem,
                        public_jwk: public,
                        created_at,
                        is_active: is_active != 0,
                    })
                },
            ))
        })
    }

    fn insert_signing_key(&self, record: &SigningKeyRecord) -> Result<()> {
        self.with_conn(|conn| {
            conn.execute("UPDATE signing_keys SET is_active = 0", [])?;
            conn.execute(
                "INSERT INTO signing_keys (kid, alg, private_pem, public_jwk, created_at, is_active)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    record.kid,
                    record.alg,
                    record.private_pem,
                    serde_json::to_string(&record.public_jwk)?,
                    record.created_at,
                    if record.is_active { 1 } else { 0 },
                ],
            )?;
            Ok(())
        })
    }

    /// Load the active signing key, generating and persisting a new one if
    /// none exists.
    pub fn load_or_create_signing_key(&self) -> Result<keyring::SigningKeyEntry> {
        if let Some(record) = self.load_active_signing_key()? {
            return Ok(keyring::SigningKeyEntry {
                kid: record.kid,
                alg: record.alg,
                private_pem: record.private_pem,
                public_jwk: record.public_jwk,
            });
        }
        use rand_core::RngCore as _;
        let mut rng = rand_core::OsRng;
        let private = RsaPrivateKey::new(&mut rng, 2048)
            .map_err(|error| anyhow::anyhow!("failed to generate RSA key: {error}"))?;
        let public = RsaPublicKey::from(&private);
        let private_pem = private
            .to_pkcs8_pem(pkcs8::LineEnding::LF)
            .map_err(|error| anyhow::anyhow!("failed to encode PKCS#8: {error}"))?
            .to_string();
        let _ = public
            .to_public_key_der()
            .map_err(|error| anyhow::anyhow!("failed to encode SPKI: {error}"))?;
        let n_b64 = URL_SAFE_NO_PAD.encode(public.n().to_bytes_be());
        let e_b64 = URL_SAFE_NO_PAD.encode(public.e().to_bytes_be());
        let mut seed = [0u8; 8];
        rng.fill_bytes(&mut seed);
        let kid = format!(
            "chatcodex-{}",
            URL_SAFE_NO_PAD.encode(seed)
        );
        let public_jwk = serde_json::json!({
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": kid,
            "n": n_b64,
            "e": e_b64,
        });
        let record = SigningKeyRecord {
            kid: kid.clone(),
            alg: "RS256".to_string(),
            private_pem,
            public_jwk: public_jwk.clone(),
            created_at: now(),
            is_active: true,
        };
        self.insert_signing_key(&record)?;
        Ok(keyring::SigningKeyEntry {
            kid,
            alg: "RS256".to_string(),
            private_pem: record.private_pem,
            public_jwk,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open_in_memory().expect("store")
    }

    #[test]
    fn auth_code_single_use() {
        let store = store();
        let code = "opaque-code".to_string();
        let record = AuthCodeRecord {
            code_hash: hash_token(&code),
            client_id: "client-1".to_string(),
            subject: "user".to_string(),
            redirect_uri: "https://chatgpt.com/cb".to_string(),
            scope: "mcp:tools".to_string(),
            code_challenge: None,
            code_challenge_method: None,
            expires_at: now() + 60,
            consumed_at: None,
        };
        store.insert_auth_code(&record).expect("insert");
        let first = store
            .consume_auth_code(&record.code_hash, now())
            .expect("consume")
            .expect("present");
        assert_eq!(first.subject, "user");
        let second = store
            .consume_auth_code(&record.code_hash, now())
            .expect("consume-2");
        assert!(second.is_none(), "replay must return None");
    }

    #[test]
    fn expired_auth_code_rejected() {
        let store = store();
        let record = AuthCodeRecord {
            code_hash: hash_token("expired"),
            client_id: "client-1".to_string(),
            subject: "user".to_string(),
            redirect_uri: "https://chatgpt.com/cb".to_string(),
            scope: "mcp:tools".to_string(),
            code_challenge: None,
            code_challenge_method: None,
            expires_at: now() - 5,
            consumed_at: None,
        };
        store.insert_auth_code(&record).expect("insert");
        let result = store
            .consume_auth_code(&record.code_hash, now())
            .expect("consume");
        assert!(result.is_none());
    }

    #[test]
    fn jti_lifecycle() {
        let store = store();
        let record = AccessTokenJtiRecord {
            jti: "jti-1".to_string(),
            client_id: "client".to_string(),
            subject: "user".to_string(),
            expires_at: now() + 60,
            revoked_at: None,
        };
        store.insert_access_jti(&record).expect("insert");
        assert!(store.jti_is_active(&record.jti, now()).expect("active"));
        store.revoke_jti(&record.jti, now()).expect("revoke");
        assert!(!store.jti_is_active(&record.jti, now()).expect("inactive"));
    }

    #[test]
    fn signing_key_is_persistent() {
        let store = store();
        let entry = store.load_or_create_signing_key().expect("key");
        let again = store.load_or_create_signing_key().expect("again");
        assert_eq!(entry.kid, again.kid);
    }
}
