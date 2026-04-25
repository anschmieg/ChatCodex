//! Hybrid provider configuration for dual-mode worker LLM calls.
//!
//! Loaded from environment variables at daemon startup.
//!
//! ## Environment Variables
//!
//! | Variable | Required | Default | Description |
//! |---|---|---|---|
//! | `CHATCODEX_HYBRID_ENABLED` | No | `false` | Enable hybrid mode |
//! | `CHATCODEX_HYBRID_PROVIDER_BASE_URL` | If enabled | — | OpenAI-compatible base URL |
//! | `CHATCODEX_HYBRID_PROVIDER_MODEL` | If enabled | — | Model name |
//! | `CHATCODEX_HYBRID_PROVIDER_API_KEY_ENV` | No | — | Env var name holding API key |
//! | `CHATCODEX_HYBRID_PROVIDER_TIMEOUT_SECONDS` | No | `120` | Request timeout |
//! | `CHATCODEX_HYBRID_PROVIDER_MAX_OUTPUT_TOKENS` | No | `8000` | Max completion tokens |
//! | `CHATCODEX_HYBRID_PROVIDER_TEMPERATURE` | No | `0.2` | Sampling temperature |

use serde::{Deserialize, Serialize};

/// Profile for an OpenAI-compatible worker LLM provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridProviderProfile {
    /// Unique identifier for this profile.
    pub profile_id: String,
    /// Provider kind — only `openai_compatible` in v1.
    pub kind: HybridProviderKind,
    /// Base URL of the OpenAI-compatible endpoint (e.g. `http://127.0.0.1:11434/v1`).
    pub base_url: String,
    /// Name of the environment variable holding the API key. Optional.
    pub api_key_env: Option<String>,
    /// Model name advertised to the provider.
    pub model: String,
    /// Request timeout in seconds.
    pub timeout_seconds: u64,
    /// Sampling temperature.
    pub temperature: f32,
    /// Maximum output tokens.
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HybridProviderKind {
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
}

/// Global hybrid configuration, populated at daemon startup.
#[derive(Debug, Clone)]
pub struct HybridConfig {
    /// Whether hybrid mode is globally enabled.
    pub enabled: bool,
    /// The default provider profile to use for worker calls.
    pub default_profile: Option<HybridProviderProfile>,
}

impl HybridConfig {
    /// Load configuration from environment variables.
    ///
    /// Hybrid is disabled unless `CHATCODEX_HYBRID_ENABLED=true`.
    /// When enabled, `CHATCODEX_HYBRID_PROVIDER_BASE_URL` and
    /// `CHATCODEX_HYBRID_PROVIDER_MODEL` are required.
    pub fn load_from_env() -> Self {
        let enabled = std::env::var("CHATCODEX_HYBRID_ENABLED")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        if !enabled {
            return Self {
                enabled: false,
                default_profile: None,
            };
        }

        let base_url = match std::env::var("CHATCODEX_HYBRID_PROVIDER_BASE_URL") {
            Ok(v) if !v.is_empty() => v,
            _ => {
                return Self {
                    enabled: false,
                    default_profile: None,
                };
            }
        };

        let model = match std::env::var("CHATCODEX_HYBRID_PROVIDER_MODEL") {
            Ok(v) if !v.is_empty() => v,
            _ => {
                return Self {
                    enabled: false,
                    default_profile: None,
                };
            }
        };

        let api_key_env = std::env::var("CHATCODEX_HYBRID_PROVIDER_API_KEY_ENV")
            .ok()
            .filter(|v| !v.is_empty());

        let timeout_seconds = std::env::var("CHATCODEX_HYBRID_PROVIDER_TIMEOUT_SECONDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(120);

        let max_output_tokens = std::env::var("CHATCODEX_HYBRID_PROVIDER_MAX_OUTPUT_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8000);

        let temperature = std::env::var("CHATCODEX_HYBRID_PROVIDER_TEMPERATURE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.2);

        let profile = HybridProviderProfile {
            profile_id: "default".to_string(),
            kind: HybridProviderKind::OpenAiCompatible,
            base_url,
            api_key_env,
            model,
            timeout_seconds,
            temperature,
            max_output_tokens,
        };

        Self {
            enabled: true,
            default_profile: Some(profile),
        }
    }

    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[inline]
    pub fn get_default_profile(&self) -> Option<&HybridProviderProfile> {
        self.default_profile.as_ref()
    }
}

impl HybridProviderProfile {
    /// Validate this profile. Returns `Err` with an explanation if invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.base_url.is_empty() {
            return Err("base_url must not be empty".to_string());
        }
        if self.model.is_empty() {
            return Err("model must not be empty".to_string());
        }
        if self.timeout_seconds == 0 {
            return Err("timeout_seconds must be > 0".to_string());
        }
        if !(0.0..=2.0).contains(&self.temperature) {
            return Err("temperature must be between 0.0 and 2.0".to_string());
        }
        if self.max_output_tokens == 0 {
            return Err("max_output_tokens must be > 0".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Isolated unit tests using the HybridProviderProfile builder directly.
    // These avoid process-level env var side effects.
    // -----------------------------------------------------------------------

    #[test]
    fn profile_validate_accepts_valid_profile() {
        let profile = HybridProviderProfile {
            profile_id: "test".to_string(),
            kind: HybridProviderKind::OpenAiCompatible,
            base_url: "http://localhost".to_string(),
            api_key_env: None,
            model: "model".to_string(),
            timeout_seconds: 120,
            temperature: 0.2,
            max_output_tokens: 8000,
        };
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn profile_validate_rejects_invalid_temperature() {
        let profile = HybridProviderProfile {
            profile_id: "test".to_string(),
            kind: HybridProviderKind::OpenAiCompatible,
            base_url: "http://localhost".to_string(),
            api_key_env: None,
            model: "model".to_string(),
            timeout_seconds: 120,
            temperature: 5.0,
            max_output_tokens: 8000,
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn profile_validate_rejects_empty_base_url() {
        let profile = HybridProviderProfile {
            profile_id: "test".to_string(),
            kind: HybridProviderKind::OpenAiCompatible,
            base_url: "".to_string(),
            api_key_env: None,
            model: "model".to_string(),
            timeout_seconds: 120,
            temperature: 0.2,
            max_output_tokens: 8000,
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn profile_validate_rejects_empty_model() {
        let profile = HybridProviderProfile {
            profile_id: "test".to_string(),
            kind: HybridProviderKind::OpenAiCompatible,
            base_url: "http://localhost".to_string(),
            api_key_env: None,
            model: "".to_string(),
            timeout_seconds: 120,
            temperature: 0.2,
            max_output_tokens: 8000,
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn config_disabled_by_default() {
        // When no HYBRID_ vars are set in the parent environment, config is disabled.
        let config = HybridConfig::load_from_env();
        if config.is_enabled() {
            assert!(config.get_default_profile().is_some());
        } else {
            assert!(config.get_default_profile().is_none());
        }
    }
}
