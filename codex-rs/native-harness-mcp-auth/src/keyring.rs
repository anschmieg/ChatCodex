//! RS256 signing keys, generated on first use, persisted by `Store`.
//! Public JWKs are exposed at `/.well-known/jwks.json`.

use std::sync::Arc;

use anyhow::Context;
use anyhow::Result;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use pkcs8::DecodePrivateKey;
use rsa::BigUint;
use rsa::RsaPrivateKey;
use rsa::RsaPublicKey;
use rsa::pkcs1v15::Signature;
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs1v15::VerifyingKey;
use rsa::signature::SignatureEncoding;
use rsa::signature::Signer;
use rsa::signature::Verifier;
use sha2::Sha256;

use crate::storage::Store;

/// The active signing key, with a copy of the public JWK for serialization.
#[derive(Debug, Clone)]
pub struct SigningKeyEntry {
    pub kid: String,
    pub alg: String,
    pub private_pem: String,
    pub public_jwk: serde_json::Value,
}

impl SigningKeyEntry {
    /// Mint a JWT for the supplied base64url-encoded payload, signed with the
    /// private key.
    pub fn sign_jwt(&self, payload_b64: &str) -> Result<String> {
        let private = RsaPrivateKey::from_pkcs8_pem(&self.private_pem)
            .context("loading PKCS#8 private key")?;
        let signing_key = SigningKey::<Sha256>::new(private);
        let header_b64 = URL_SAFE_NO_PAD.encode(
            serde_json::json!({
                "alg": "RS256",
                "typ": "JWT",
                "kid": self.kid,
            })
            .to_string()
            .as_bytes(),
        );
        let signing_input = format!("{header_b64}.{payload_b64}");
        let signature: Signature = signing_key.sign(signing_input.as_bytes());
        let signature_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());
        Ok(format!("{signing_input}.{signature_b64}"))
    }
}

/// Verify a JWT against the supplied key and return the parsed claims if valid.
pub fn verify_jwt(
    token: &str,
    issuer: &str,
    audience: &str,
    key: &SigningKeyEntry,
    now_ts: i64,
) -> Result<Option<serde_json::Value>> {
    let mut parts = token.split('.');
    let Some(header_b64) = parts.next() else {
        return Ok(None);
    };
    let Some(payload_b64) = parts.next() else {
        return Ok(None);
    };
    let Some(signature_b64) = parts.next() else {
        return Ok(None);
    };
    if parts.next().is_some() {
        return Ok(None);
    }
    let header_bytes = URL_SAFE_NO_PAD
        .decode(header_b64)
        .map_err(|error| anyhow::anyhow!("invalid JWT header base64: {error}"))?;
    let header: serde_json::Value = serde_json::from_slice(&header_bytes)
        .map_err(|error| anyhow::anyhow!("invalid JWT header JSON: {error}"))?;
    let kid = header
        .get("kid")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("JWT header missing kid"))?;
    if kid != key.kid {
        return Ok(None);
    }
    let alg = header
        .get("alg")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if alg != "RS256" {
        return Ok(None);
    }
    let payload_bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|error| anyhow::anyhow!("invalid JWT payload base64: {error}"))?;
    let claims: serde_json::Value = serde_json::from_slice(&payload_bytes)
        .map_err(|error| anyhow::anyhow!("invalid JWT payload JSON: {error}"))?;
    let exp = claims
        .get("exp")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("JWT missing exp claim"))?;
    if exp <= now_ts {
        return Ok(None);
    }
    let iss = claims
        .get("iss")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("JWT missing iss claim"))?;
    if iss != issuer {
        return Ok(None);
    }
    let aud_ok = match claims.get("aud") {
        Some(serde_json::Value::String(s)) => s == audience,
        Some(serde_json::Value::Array(arr)) => arr.iter().any(|v| v.as_str() == Some(audience)),
        _ => false,
    };
    if !aud_ok {
        return Ok(None);
    }
    // For verification we need a public key. If the SigningKeyEntry has a
    // private PEM (ChatCodex-issued tokens), derive the public key from it.
    // Otherwise, reconstruct the public key from the JWK fields (Cloudflare
    // Access tokens, which only have a public key).
    let public = if !key.private_pem.is_empty() {
        let private = RsaPrivateKey::from_pkcs8_pem(&key.private_pem)
            .context("loading PKCS#8 private key for verify")?;
        RsaPublicKey::from(&private)
    } else {
        let jwk = &key.public_jwk;
        let n_b64 = jwk.get("n").and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("JWK missing n"))?;
        let e_b64 = jwk.get("e").and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("JWK missing e"))?;
        let n_bytes = URL_SAFE_NO_PAD.decode(n_b64)
            .context("decoding JWK n")?;
        let e_bytes = URL_SAFE_NO_PAD.decode(e_b64)
            .context("decoding JWK e")?;
        let n = BigUint::from_bytes_be(&n_bytes);
        let e = BigUint::from_bytes_be(&e_bytes);
        RsaPublicKey::new(n, e)
            .context("invalid JWK RSA key")?
    };
    let signature = URL_SAFE_NO_PAD
        .decode(signature_b64)
        .map_err(|error| anyhow::anyhow!("invalid JWT signature base64: {error}"))?;
    let verifying_key = VerifyingKey::<Sha256>::new(public);
    let signature = Signature::try_from(signature.as_slice())
        .map_err(|error| anyhow::anyhow!("invalid JWT signature: {error}"))?;
    let signing_input = format!("{header_b64}.{payload_b64}");
    verifying_key
        .verify(signing_input.as_bytes(), &signature)
        .map_err(|error| anyhow::anyhow!("JWT signature verification failed: {error}"))?;
    Ok(Some(claims))
}

/// Cache wrapping the persistent signing-key store and configured issuer/audience.
#[derive(Clone)]
pub struct Keyring {
    inner: Arc<KeyringInner>,
}

struct KeyringInner {
    active: std::sync::Mutex<Option<SigningKeyEntry>>,
    issuer: String,
    audience: String,
}

impl Keyring {
    /// Load or create the active signing key.
    pub fn load_or_create(store: Store, issuer: String, audience: String) -> Result<Self> {
        let entry = store.load_or_create_signing_key()?;
        Ok(Self {
            inner: Arc::new(KeyringInner {
                active: std::sync::Mutex::new(Some(entry)),
                issuer,
                audience,
            }),
        })
    }

    #[allow(clippy::expect_used)]
    pub async fn active(&self) -> SigningKeyEntry {
        self.inner
            .active
            .lock()
            .expect("keyring mutex poisoned")
            .clone()
            .expect("keyring entry should be initialized")
    }

    pub fn issuer(&self) -> &str {
        &self.inner.issuer
    }

    pub fn audience(&self) -> &str {
        &self.inner.audience
    }

    /// Public JWKs for the JWKS endpoint.
    pub async fn jwks(&self) -> serde_json::Value {
        let entry = self.active().await;
        serde_json::json!({ "keys": [entry.public_jwk] })
    }

    /// Sign claims (already JSON-serialized and base64url-encoded).
    pub async fn sign(&self, payload_b64: &str) -> Result<String> {
        let entry = self.active().await;
        entry.sign_jwt(payload_b64)
    }

    /// Verify a token against the current active key and configured issuer/audience.
    #[allow(clippy::expect_used)]
    pub fn verify(&self, token: &str, now_ts: i64) -> Result<Option<serde_json::Value>> {
        let active = self
            .inner
            .active
            .lock()
            .expect("keyring mutex poisoned")
            .clone()
            .expect("keyring entry should be initialized");
        verify_jwt(token, &self.inner.issuer, &self.inner.audience, &active, now_ts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn keyring_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let keyring = Keyring::load_or_create(
            store,
            "https://example".to_string(),
            "https://example/mcp".to_string(),
        )
        .unwrap();
        let now_ts = crate::storage::now();
        let claims = serde_json::json!({
            "iss": "https://example",
            "aud": "https://example/mcp",
            "exp": now_ts + 60,
            "sub": "user-1",
            "client_id": "client",
            "scope": "mcp:tools",
            "jti": "jti-1",
        });
        let payload_b64 = URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());
        let token = keyring.sign(&payload_b64).await.unwrap();
        let verified = keyring.verify(&token, now_ts).unwrap().expect("verified");
        assert_eq!(verified["sub"], "user-1");
        assert_eq!(verified["jti"], "jti-1");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn keyring_rejects_wrong_audience() {
        let store = Store::open_in_memory().unwrap();
        let keyring = Keyring::load_or_create(
            store,
            "https://example".to_string(),
            "https://example/mcp".to_string(),
        )
        .unwrap();
        let now_ts = crate::storage::now();
        let claims = serde_json::json!({
            "iss": "https://example",
            "aud": "https://other/mcp",
            "exp": now_ts + 60,
            "sub": "user-1",
        });
        let payload_b64 = URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());
        let token = keyring.sign(&payload_b64).await.unwrap();
        let result = keyring.verify(&token, now_ts).unwrap();
        assert!(result.is_none());
    }
}
