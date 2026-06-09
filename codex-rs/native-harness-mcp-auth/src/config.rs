//! Configuration loaded from environment variables.

use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use std::path::PathBuf;

/// Configuration for the OAuth 2.1 layer.
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Externally visible origin (e.g. `https://codex.nothing.pink`).
    pub public_base_url: String,
    /// Directory used for persistent state (token DB, keyring).
    pub data_dir: PathBuf,
    /// Cloudflare Access team domain (e.g. `https://team.cloudflareaccess.com`).
    pub cf_access_team: String,
    /// Expected `aud` claim for the Cloudflare Access JWT.
    pub cf_access_aud: String,
    /// Access-token lifetime.
    pub access_ttl: Duration,
    /// Refresh-token lifetime.
    pub refresh_ttl: Duration,
    /// Allow registering confidential clients with a static secret. Off by
    /// default: ChatGPT registers public PKCE clients only.
    pub allow_client_credentials: bool,
}

impl AuthConfig {
    /// Load configuration from the process environment. Required variables:
    ///
    /// * `CHATCODEX_PUBLIC_BASE_URL`
    /// * `CHATCODEX_CF_ACCESS_TEAM`
    /// * `CHATCODEX_CF_ACCESS_AUD`
    pub fn from_env() -> Result<Self> {
        let public_base_url = require_env("CHATCODEX_PUBLIC_BASE_URL")?;
        let cf_access_team = require_env("CHATCODEX_CF_ACCESS_TEAM")?;
        let cf_access_aud = require_env("CHATCODEX_CF_ACCESS_AUD")?;
        let data_dir = std::env::var_os("CHATCODEX_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/data"));
        let access_ttl = std::env::var("CHATCODEX_OAUTH_ACCESS_TTL_SECS")
            .ok()
            .and_then(|value| value.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(3600));
        let refresh_ttl = std::env::var("CHATCODEX_OAUTH_REFRESH_TTL_SECS")
            .ok()
            .and_then(|value| value.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(60 * 60 * 24 * 30));
        let allow_client_credentials = std::env::var("CHATCODEX_OAUTH_ALLOW_CONFIDENTIAL")
            .ok()
            .and_then(|value| match value.as_str() {
                "1" | "true" | "TRUE" | "yes" => Some(true),
                "0" | "false" | "FALSE" | "no" => Some(false),
                _ => None,
            })
            .unwrap_or(false);

        Ok(Self {
            public_base_url: trim_trailing_slash(&public_base_url),
            data_dir,
            cf_access_team: trim_trailing_slash(&cf_access_team),
            cf_access_aud,
            access_ttl,
            refresh_ttl,
            allow_client_credentials,
        })
    }

    /// Authorization endpoint URL exposed to MCP clients.
    pub fn authorization_endpoint(&self) -> String {
        format!("{}/oauth/authorize", self.public_base_url)
    }

    /// Token endpoint URL.
    pub fn token_endpoint(&self) -> String {
        format!("{}/oauth/token", self.public_base_url)
    }

    /// Dynamic client registration endpoint URL.
    pub fn registration_endpoint(&self) -> String {
        format!("{}/oauth/register", self.public_base_url)
    }

    /// JWKS URL.
    pub fn jwks_uri(&self) -> String {
        format!("{}/.well-known/jwks.json", self.public_base_url)
    }

    /// Resource indicator for issued access tokens.
    pub fn resource_indicator(&self) -> String {
        format!("{}/mcp", self.public_base_url)
    }

    /// Issuer claim for issued JWTs.
    pub fn issuer(&self) -> String {
        self.public_base_url.clone()
    }

    /// JWKS URL for Cloudflare Access signing keys.
    pub fn cf_access_certs_uri(&self) -> String {
        format!("{}/cdn-cgi/access/certs", self.cf_access_team)
    }
}

fn require_env(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("{name} must be set"))
}

fn trim_trailing_slash(value: &str) -> String {
    value.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_use_public_base_url() {
        let cfg = AuthConfig {
            public_base_url: "https://codex.example".to_string(),
            data_dir: PathBuf::from("/data"),
            cf_access_team: "https://team.cloudflareaccess.com".to_string(),
            cf_access_aud: "aud".to_string(),
            access_ttl: Duration::from_secs(1),
            refresh_ttl: Duration::from_secs(1),
            allow_client_credentials: false,
        };
        assert_eq!(
            cfg.authorization_endpoint(),
            "https://codex.example/oauth/authorize"
        );
        assert_eq!(cfg.jwks_uri(), "https://codex.example/.well-known/jwks.json");
        assert_eq!(
            cfg.cf_access_certs_uri(),
            "https://team.cloudflareaccess.com/cdn-cgi/access/certs"
        );
    }
}
