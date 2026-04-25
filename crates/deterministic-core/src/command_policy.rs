//! Test command policy enforcement.
//! The harness runs test commands deterministically based on scope labels
//! (cargo, npm, pytest, make) and explicitly configured targets.  This module
//! adds a `CommandPolicy` layer that can whitelist or deny specific commands
//! before they reach the approval policy layer.
//!
//! # Configuration
//!
//! Policy is loaded from, in order of precedence:
//! 1. `TESTS_COMMAND_MODE` env var: `whitelist` | `deny` | `allow_all`
//! 2. `TESTS_ALLOWED_SCOPES` env var (whitelist mode): comma-separated list
//! 3. `TESTS_DENIED_SCOPES` env var (deny mode): comma-separated list
//! 4. Workspace `.chatcodex-command-policy.json` file (same structure as env)
//!
//! Defaults when nothing is set:
//! - Mode: `whitelist`
//! - Allowed scopes: `cargo,pytest,npm,make,unit,integration,all`

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

/// Policy operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandPolicyMode {
    /// Only explicitly allowed commands pass through.
    #[serde(rename = "whitelist")]
    Whitelist,
    /// Block listed commands; all others pass.
    #[serde(rename = "deny")]
    Deny,
    /// No enforcement — allow all resolved commands.
    #[serde(rename = "allow_all")]
    AllowAll,
}

impl Default for CommandPolicyMode {
    fn default() -> Self {
        CommandPolicyMode::Whitelist
    }
}

/// Test command policy configuration.
///
/// Controls which scopes are permitted at the daemon level before
/// the approval policy layer is consulted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandPolicy {
    /// Operating mode.
    #[serde(default)]
    pub mode: CommandPolicyMode,
    /// Scopes permitted in whitelist mode.
    #[serde(default)]
    pub allowed_scopes: HashSet<String>,
    /// Scopes blocked in deny mode.
    #[serde(default)]
    pub denied_scopes: HashSet<String>,
}

impl Default for CommandPolicy {
    fn default() -> Self {
        let allowed = ["cargo", "npm", "pytest", "make", "unit", "integration", "all"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        Self {
            mode: CommandPolicyMode::Whitelist,
            allowed_scopes: allowed,
            denied_scopes: HashSet::new(),
        }
    }
}

impl CommandPolicy {
    /// Load policy from environment variables and workspace config file.
    ///
    /// Precedence (highest first):
    /// 1. Env vars (TESTS_COMMAND_MODE, TESTS_ALLOWED_SCOPES, TESTS_DENIED_SCOPES)
    /// 2. Workspace `.chatcodex-command-policy.json`
    /// 3. Defaults
    pub fn load(workspace_root: Option<&str>) -> Self {
        // Start with defaults
        let mut policy = Self::default();

        // Check env vars first (highest precedence)
        if let Ok(mode) = std::env::var("TESTS_COMMAND_MODE") {
            policy.mode = match mode.trim().to_lowercase().as_str() {
                "whitelist" => CommandPolicyMode::Whitelist,
                "deny" => CommandPolicyMode::Deny,
                "allow_all" => CommandPolicyMode::AllowAll,
                _ => CommandPolicyMode::Whitelist,
            };
        }

        if let Ok(allowed) = std::env::var("TESTS_ALLOWED_SCOPES") {
            policy.allowed_scopes = allowed
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
        }

        if let Ok(denied) = std::env::var("TESTS_DENIED_SCOPES") {
            policy.denied_scopes = denied
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
        }

        // Workspace config file can override
        if let Some(root) = workspace_root {
            let config_path = Path::new(root).join(".chatcodex-command-policy.json");
            if config_path.is_file() {
                if let Ok(workspace_policy) = Self::load_from_file(&config_path) {
                    // Workspace file overrides only if it provides values
                    policy.mode = workspace_policy.mode;
                    if !workspace_policy.allowed_scopes.is_empty() {
                        policy.allowed_scopes = workspace_policy.allowed_scopes;
                    }
                    if !workspace_policy.denied_scopes.is_empty() {
                        policy.denied_scopes = workspace_policy.denied_scopes;
                    }
                }
            }
        }

        policy
    }

    /// Load policy from a JSON file.
    fn load_from_file(path: &Path) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let p: CommandPolicy = serde_json::from_str(&content)?;
        Ok(p)
    }

    /// Check whether a scope is permitted under this policy.
    pub fn is_scope_allowed(&self, scope: &str) -> bool {
        match self.mode {
            CommandPolicyMode::AllowAll => true,
            CommandPolicyMode::Whitelist => self.allowed_scopes.contains(&scope.to_lowercase()),
            CommandPolicyMode::Deny => !self.denied_scopes.contains(&scope.to_lowercase()),
        }
    }

    /// Validate a scope against this policy.
    /// Returns `Ok(())` if the scope is allowed, or an error describing the violation.
    pub fn validate_scope(&self, scope: &str) -> Result<(), CommandPolicyViolation> {
        if !self.is_scope_allowed(scope) {
            match self.mode {
                CommandPolicyMode::Whitelist => Err(CommandPolicyViolation::ScopeNotWhitelisted {
                    scope: scope.to_string(),
                    allowed: self.allowed_scopes.clone(),
                }),
                CommandPolicyMode::Deny => Err(CommandPolicyViolation::ScopeDenied {
                    scope: scope.to_string(),
                    denied: self.denied_scopes.clone(),
                }),
                CommandPolicyMode::AllowAll => Ok(()),
            }
        } else {
            Ok(())
        }
    }
}

/// A policy violation when attempting to run a test command.
#[derive(Debug, Clone)]
pub enum CommandPolicyViolation {
    ScopeNotWhitelisted {
        scope: String,
        allowed: HashSet<String>,
    },
    ScopeDenied {
        scope: String,
        denied: HashSet<String>,
    },
}

impl std::fmt::Display for CommandPolicyViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ScopeNotWhitelisted { scope, allowed } => {
                write!(
                    f,
                    "test scope '{}' is not whitelisted. Allowed scopes: {}",
                    scope,
                    allowed.iter().cloned().collect::<Vec<_>>().join(", ")
                )
            }
            Self::ScopeDenied { scope, denied } => {
                write!(
                    f,
                    "test scope '{}' is explicitly denied. Denied scopes: {}",
                    scope,
                    denied.iter().cloned().collect::<Vec<_>>().join(", ")
                )
            }
        }
    }
}

impl std::error::Error for CommandPolicyViolation {}

// Tests live in tests/command_policy.rs (integration test compiled as separate binary,
// allowing use of unsafe std::env fns under the workspace deny(unsafe_code) policy).
#[cfg(test)]
mod tests {
    // No unsafe env tests here (conflicts with deny(unsafe_code)).
    // Use tests/command_policy.rs for env var tests.

    use super::*;
    use std::collections::HashSet;

    #[test]
    fn whitelist_mode_rejects_unlisted() {
        let policy = CommandPolicy {
            mode: CommandPolicyMode::Whitelist,
            allowed_scopes: ["cargo", "pytest"].iter().map(|s| s.to_string()).collect(),
            denied_scopes: HashSet::new(),
        };
        assert!(policy.is_scope_allowed("cargo"));
        assert!(policy.is_scope_allowed("pytest"));
        assert!(!policy.is_scope_allowed("npm"));
        assert!(!policy.is_scope_allowed("make"));
    }

    #[test]
    fn deny_mode_blocks_listed() {
        let policy = CommandPolicy {
            mode: CommandPolicyMode::Deny,
            allowed_scopes: HashSet::new(),
            denied_scopes: ["bash", "sh"].iter().map(|s| s.to_string()).collect(),
        };
        assert!(!policy.is_scope_allowed("bash"));
        assert!(!policy.is_scope_allowed("sh"));
        assert!(policy.is_scope_allowed("cargo")); // not denied
    }

    #[test]
    fn allow_all_mode_permits_everything() {
        let policy = CommandPolicy {
            mode: CommandPolicyMode::AllowAll,
            allowed_scopes: HashSet::new(),
            denied_scopes: HashSet::new(),
        };
        assert!(policy.is_scope_allowed("cargo"));
        assert!(policy.is_scope_allowed("bash"));
        assert!(policy.is_scope_allowed("curl"));
    }

    #[test]
    fn validate_scope_returns_ok_for_allowed() {
        let policy = CommandPolicy::default();
        assert!(policy.validate_scope("cargo").is_ok());
        assert!(policy.validate_scope("npm").is_ok());
    }

    #[test]
    fn validate_scope_returns_error_for_denied() {
        let policy = CommandPolicy::default();
        let result = policy.validate_scope("bash");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not whitelisted"));
    }

    #[test]
    fn validate_scope_deny_mode_describes_denied() {
        let policy = CommandPolicy {
            mode: CommandPolicyMode::Deny,
            allowed_scopes: HashSet::new(),
            denied_scopes: ["bash"].iter().map(|s| s.to_string()).collect(),
        };
        let result = policy.validate_scope("bash");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("explicitly denied"));
    }

    #[test]
    fn empty_scope_is_rejected() {
        let policy = CommandPolicy::default();
        let result = policy.validate_scope("");
        assert!(result.is_err());
    }

    #[test]
    fn case_insensitive_scope_check() {
        let policy = CommandPolicy::default();
        assert!(policy.is_scope_allowed("CARGO"));
        assert!(policy.is_scope_allowed("PyTest"));
        assert!(policy.is_scope_allowed("NPM"));
    }

    #[test]
    fn serde_round_trip() {
        let policy = CommandPolicy {
            mode: CommandPolicyMode::Whitelist,
            allowed_scopes: ["cargo", "pytest"].iter().map(|s| s.to_string()).collect(),
            denied_scopes: HashSet::new(),
        };
        let json = serde_json::to_string(&policy).unwrap();
        let restored: CommandPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.mode, CommandPolicyMode::Whitelist);
        assert!(restored.allowed_scopes.contains("cargo"));
    }
}