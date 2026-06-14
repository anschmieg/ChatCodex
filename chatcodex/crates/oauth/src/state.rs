//! Process-wide auth state shared across handlers.

use std::sync::Arc;

use crate::cf_access::CfAccessVerifier;
use crate::config::AuthConfig;
use crate::keyring::Keyring;
use crate::storage::Store;

#[derive(Clone)]
pub struct AuthState {
    inner: Arc<AuthStateInner>,
}

pub struct AuthStateInner {
    pub config: AuthConfig,
    pub store: Store,
    pub keyring: Keyring,
    pub cf: CfAccessVerifier,
}

impl AuthState {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::new(crate::config::AuthConfig::from_env()?)
    }

    pub fn new(config: AuthConfig) -> anyhow::Result<Self> {
        let store = Store::open(&config.data_dir)?;
        let resource = config.resource_indicator();
        let keyring = Keyring::load_or_create(store.clone(), config.issuer(), resource)?;
        let cf = CfAccessVerifier::new(config.cf_access_certs_uri(), config.cf_access_aud.clone());
        Ok(Self {
            inner: Arc::new(AuthStateInner {
                config,
                store,
                keyring,
                cf,
            }),
        })
    }

    pub fn new_for_test(
        config: AuthConfig,
        store: Store,
        keyring: Keyring,
        cf: CfAccessVerifier,
    ) -> Self {
        Self {
            inner: Arc::new(AuthStateInner {
                config,
                store,
                keyring,
                cf,
            }),
        }
    }

    pub fn config(&self) -> &AuthConfig {
        &self.inner.config
    }

    pub fn store(&self) -> &Store {
        &self.inner.store
    }

    pub fn keyring(&self) -> &Keyring {
        &self.inner.keyring
    }

    pub fn cf(&self) -> &CfAccessVerifier {
        &self.inner.cf
    }
}
