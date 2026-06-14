//! Cloudflare Access identity verification.
//!
//! After the operator has authenticated through their team, Cloudflare
//! forwards the request with a `CF_Authorization` JWT cookie. The JWT is
//! signed by the team's published key set. We fetch the certs once, cache
//! them, and verify the signature + standard claims before letting the
//! consent handler proceed.

use std::sync::Arc;
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use anyhow::Context;
use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rsa::BigUint;
use rsa::RsaPublicKey;
use rsa::traits::PublicKeyParts;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::keyring::SigningKeyEntry;
use crate::keyring::verify_jwt;

/// Minimum set of claims we expect from a Cloudflare Access JWT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfAccessClaims {
    pub aud: Value,
    pub email: Option<String>,
    pub sub: String,
    pub iss: String,
    pub iat: i64,
    pub exp: i64,
}

/// Cached JWKs from the team's `cdn-cgi/access/certs` endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
struct CfCerts {
    keys: Vec<CfJwk>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CfJwk {
    kid: String,
    kty: String,
    #[serde(default, rename = "use")]
    usage: Option<String>,
    alg: String,
    n: String,
    e: String,
}

/// Cloudflare Access verifier that lazily fetches and caches the team's
/// signing keys.
#[derive(Clone)]
pub struct CfAccessVerifier {
    inner: Arc<CfAccessInner>,
    stub: Option<Arc<StubInner>>,
}

struct StubInner {
    expected_aud: String,
    subject: String,
    email: Option<String>,
}

struct CfAccessInner {
    certs_uri: String,
    team_origin: String,
    expected_aud: String,
    cache: RwLock<Option<CachedCerts>>,
    http: reqwest::Client,
}

#[derive(Debug, Clone)]
struct CachedCerts {
    fetched_at: i64,
    keys: Vec<CfJwk>,
}

impl CfAccessVerifier {
    /// Build a verifier that will fetch certs from `certs_uri` and accept
    /// JWTs whose `aud` claim equals `expected_aud`.
    /// Build a stub verifier for tests. Any non-empty cookie value is
    /// accepted; the supplied `subject` and `email` are returned as claims.
    #[allow(clippy::expect_used)]
    pub fn new_stub(expected_aud: String, subject: String, email: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client");
        let team_origin = "https://stub.team".to_string();
        let inner = StubInner {
            expected_aud,
            subject,
            email,
        };
        let inner = std::sync::Arc::new(inner);
        Self {
            inner: Arc::new(CfAccessInner {
                certs_uri: format!("{team_origin}/cdn-cgi/access/certs"),
                team_origin,
                expected_aud: inner.expected_aud.clone(),
                cache: RwLock::new(Some(CachedCerts {
                    fetched_at: now(),
                    keys: Vec::new(),
                })),
                http,
            }),
            stub: Some(inner),
        }
    }

    #[allow(clippy::expect_used)]
    pub fn new(certs_uri: String, expected_aud: String) -> Self {
        let team_origin = certs_uri
            .trim_end_matches("/cdn-cgi/access/certs")
            .trim_end_matches('/')
            .to_string();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest client");
        Self {
            inner: Arc::new(CfAccessInner {
                certs_uri,
                team_origin,
                expected_aud,
                cache: RwLock::new(None),
                http,
            }),
            stub: None,
        }
    }

    /// Fetch the current set of certs, refreshing the cache if older than
    /// 10 minutes.
    pub async fn refresh_certs(&self) -> Result<()> {
        let now = now();
        {
            let cache = self.inner.cache.read().await;
            if let Some(cached) = cache.as_ref()
                && now - cached.fetched_at < 600
            {
                return Ok(());
            }
        }
        let response = self
            .inner
            .http
            .get(&self.inner.certs_uri)
            .send()
            .await
            .with_context(|| format!("fetching CF Access certs from {}", self.inner.certs_uri))?
            .error_for_status()
            .context("CF Access certs endpoint returned error")?;
        let certs: CfCerts = response
            .json()
            .await
            .context("decoding CF Access certs JSON")?;
        let mut cache = self.inner.cache.write().await;
        *cache = Some(CachedCerts {
            fetched_at: now,
            keys: certs.keys,
        });
        Ok(())
    }

    /// Verify a CF_Authorization JWT and return the claims on success.
    pub async fn verify(&self, token: &str) -> Result<Option<CfAccessClaims>> {
        if let Some(stub) = &self.stub {
            if token.is_empty() {
                return Ok(None);
            }
            return Ok(Some(CfAccessClaims {
                aud: serde_json::Value::String(stub.expected_aud.clone()),
                email: stub.email.clone(),
                sub: stub.subject.clone(),
                iss: "https://stub.team".to_string(),
                iat: now(),
                exp: now() + 3600,
            }));
        }
        self.refresh_certs().await?;
        let cache = self.inner.cache.read().await;
        let Some(cached) = cache.as_ref() else {
            return Ok(None);
        };
        let Some(kid) = peek_kid(token) else {
            return Ok(None);
        };
        let Some(jwk) = cached.keys.iter().find(|key| key.kid == kid) else {
            return Ok(None);
        };
        if jwk.kty != "RSA" {
            return Ok(None);
        }
        let key = jwk_to_entry(jwk)?;
        let Some(claims) = verify_jwt(
            token,
            &self.inner.team_origin,
            &self.inner.expected_aud,
            &key,
            now(),
        )?
        else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_value(claims)?))
    }

    /// Begin the Cloudflare Access login flow for the supplied `next` URL.
    /// The team login endpoint handles the IdP choice; the response sets
    /// the `CF_Authorization` cookie and 302s back to `next`.
    pub fn login_url(&self, next: &str) -> String {
        format!(
            "{}/cdn-cgi/access/login?next={}",
            self.inner.team_origin,
            urlencode(next)
        )
    }
}

fn jwk_to_entry(jwk: &CfJwk) -> Result<SigningKeyEntry> {
    let n_bytes = URL_SAFE_NO_PAD.decode(&jwk.n).context("decoding JWK n")?;
    let e_bytes = URL_SAFE_NO_PAD.decode(&jwk.e).context("decoding JWK e")?;
    let n = BigUint::from_bytes_be(&n_bytes);
    let e = BigUint::from_bytes_be(&e_bytes);
    let public =
        RsaPublicKey::new(n, e).map_err(|error| anyhow::anyhow!("invalid JWK RSA key: {error}"))?;
    let _ = public.n();
    Ok(SigningKeyEntry {
        kid: jwk.kid.clone(),
        alg: jwk.alg.clone(),
        private_pem: String::new(),
        public_jwk: serde_json::to_value(jwk)?,
    })
}

fn urlencode(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (*byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn peek_kid(token: &str) -> Option<String> {
    let mut parts = token.split('.');
    let header_b64 = parts.next()?;
    let bytes = URL_SAFE_NO_PAD.decode(header_b64).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value.get("kid").and_then(Value::as_str).map(str::to_string)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
