//! Tests for command_policy.rs

use deterministic_core::command_policy::{CommandPolicy, CommandPolicyMode};
use std::collections::HashSet;

#[test]
fn default_policy_allows_known_frameworks() {
    let policy = CommandPolicy::default();
    assert!(policy.is_scope_allowed("cargo"));
    assert!(policy.is_scope_allowed("npm"));
    assert!(policy.is_scope_allowed("pytest"));
    assert!(policy.is_scope_allowed("make"));
    assert!(policy.is_scope_allowed("unit"));
    assert!(policy.is_scope_allowed("integration"));
}

#[test]
fn default_policy_rejects_unknown() {
    let policy = CommandPolicy::default();
    assert!(!policy.is_scope_allowed("bash"));
    assert!(!policy.is_scope_allowed("curl"));
    assert!(!policy.is_scope_allowed("rm"));
}

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

/// Test that workspace config file with no mode field defaults to Whitelist
/// (the mode field now has #[serde(default)]).
#[test]
fn workspace_config_loads_allowed_scopes() {
    let tmp_dir = tempfile::TempDir::new().unwrap();
    let config_path = tmp_dir.path().join(".chatcodex-command-policy.json");
    std::fs::write(
        &config_path,
        r#"{"allowed_scopes":["cargo","my_custom_framework"]}"#,
    )
    .unwrap();

    let policy = CommandPolicy::load(Some(tmp_dir.path().to_str().unwrap()));
    assert_eq!(policy.mode, CommandPolicyMode::Whitelist); // default
    assert!(
        policy.allowed_scopes.contains(&"my_custom_framework".to_string()),
        "workspace config should add 'my_custom_framework' to allowed_scopes; got {:?}",
        policy.allowed_scopes
    );
}

#[test]
fn workspace_config_loads_denied_scopes() {
    let tmp_dir = tempfile::TempDir::new().unwrap();
    let config_path = tmp_dir.path().join(".chatcodex-command-policy.json");
    std::fs::write(
        &config_path,
        r#"{"denied_scopes":["custom_dangerous"]}"#,
    )
    .unwrap();

    let policy = CommandPolicy::load(Some(tmp_dir.path().to_str().unwrap()));
    assert_eq!(policy.mode, CommandPolicyMode::Whitelist); // default
    assert!(
        policy.denied_scopes.contains(&"custom_dangerous".to_string()),
        "workspace config should add 'custom_dangerous' to denied_scopes; got {:?}",
        policy.denied_scopes
    );
}

#[test]
fn missing_workspace_config_uses_defaults() {
    let tmp_dir = tempfile::TempDir::new().unwrap();
    // No config file written — should use defaults
    let policy = CommandPolicy::load(Some(tmp_dir.path().to_str().unwrap()));
    assert_eq!(policy.mode, CommandPolicyMode::Whitelist);
    assert!(policy.is_scope_allowed("cargo"));
    assert!(!policy.is_scope_allowed("bash")); // not whitelisted by default
}

#[test]
fn workspace_config_explicit_mode() {
    let tmp_dir = tempfile::TempDir::new().unwrap();
    let config_path = tmp_dir.path().join(".chatcodex-command-policy.json");
    std::fs::write(
        &config_path,
        r#"{"mode":"allow_all","allowed_scopes":["bash"]}"#,
    )
    .unwrap();

    // Verify the file exists and has correct content
    let raw = std::fs::read_to_string(&config_path).unwrap();
    eprintln!("DEBUG: config file content = {}", raw);

    // Verify it parses to the right struct
    let parsed: CommandPolicy = serde_json::from_str(&raw).unwrap();
    eprintln!("DEBUG: parsed mode={:?} allowed={:?}", parsed.mode, parsed.allowed_scopes);

    let policy = CommandPolicy::load(Some(tmp_dir.path().to_str().unwrap()));
    eprintln!("DEBUG: load returned mode={:?} allowed={:?}", policy.mode, policy.allowed_scopes);
    assert_eq!(policy.mode, CommandPolicyMode::AllowAll);
    assert!(policy.is_scope_allowed("anything")); // allow_all
}