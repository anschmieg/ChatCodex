//! Restricted, whitelisted command execution for `run_command`.
//!
//! This module enforces that only explicitly allowed commands may be
//! executed.  File writes always go through `apply_patch`; test execution
//! through `run_tests` — `run_command` is for build/lint/format/codegen
//! and other utility operations.

use deterministic_protocol::CommandRunParams;

// ---------------------------------------------------------------------------
// Command whitelist
// ---------------------------------------------------------------------------

/// The set of allowed base-command names.
///
/// Each entry is a binary name (no path).  The backend resolves it via
/// `PATH` lookup at execution time.  Arguments are passed separately and
/// are NOT subject to per-arg whitelisting (the base command is what
/// matters for the policy).
const ALLOWED_COMMANDS: &[&str] = &[
    // Rust toolchain
    "cargo",
    "rustc",
    "rustfmt",
    // JavaScript / TypeScript
    "node",
    "npm",
    "npx",
    "tsc",
    "eslint",
    "prettier",
    // Python
    "python3",
    "python",
    "pip3",
    "flake8",
    "black",
    "mypy",
    // Make / build
    "make",
    "cmake",
    "ninja",
    // Lint / format
    "clippy-driver",
    "shellcheck",
    "shfmt",
    // General utilities
    "which",
    "true",
    "false",
    "echo",
    "cat",
    "head",
    "tail",
    "wc",
    "sort",
    "uniq",
    "grep",
    "find",
    "diff",
    "comm",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "pwd",
    "env",
    "printenv",
    "date",
];

/// Patterns that are NEVER allowed in any command or argument.
const FORBIDDEN_SUBSTRINGS: &[&str] = &[
    // Shell injection
    ";",
    "|",
    "`",
    "$(",
    "${",
    // Redirection
    ">",
    "<",
    ">>",
    // Escalation
    "sudo",
    "su ",
    // Network
    "curl",
    "wget",
    "nc ",
    "telnet",
    "ssh",
    // File mutation (must go through apply_patch)
    "sed",
    "awk",
    "tee",
    "dd",
    // Dangerous globbing
    "rm -rf /",
    "rm -rf /*",
    "chmod",
    "chown",
];

/// Maximum allowed command timeout in seconds.
const MAX_TIMEOUT_SECS: u64 = 300;

/// Default timeout in seconds.
const DEFAULT_TIMEOUT_SECS: u64 = 60;

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Outcome of validating a command against the whitelist.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationOutcome {
    /// Command is allowed.
    Allowed {
        /// The base command name that was matched on the whitelist.
        matched_command: String,
    },
    /// Command is denied with a reason.
    Denied {
        reason: String,
    },
}

/// Validate command parameters against the whitelist and security rules.
pub fn validate_command(params: &CommandRunParams) -> ValidationOutcome {
    let cmd = params.command.trim();

    if cmd.is_empty() {
        return ValidationOutcome::Denied {
            reason: "command must not be empty".to_string(),
        };
    }

    // Check for forbidden substrings in both command and all args.
    let all_parts = std::iter::once(cmd)
        .chain(params.args.iter().map(|a| a.as_str()));

    for part in all_parts {
        for forbidden in FORBIDDEN_SUBSTRINGS {
            if part.contains(forbidden) {
                return ValidationOutcome::Denied {
                    reason: format!(
                        "command or argument contains forbidden pattern: {forbidden:?}"
                    ),
                };
            }
        }
    }

    // Extract the base command name (first whitespace-delimited token).
    let base = cmd.split_whitespace().next().unwrap_or("");

    // Check if the base command is on the whitelist.
    for allowed in ALLOWED_COMMANDS {
        if base == *allowed {
            return ValidationOutcome::Allowed {
                matched_command: allowed.to_string(),
            };
        }
    }

    // Check if it's a relative or absolute path to an allowed command.
    // e.g., "./node_modules/.bin/tsc" — extract the filename.
    if let Some(file_name) = std::path::Path::new(base).file_name() {
        let name = file_name.to_string_lossy();
        if ALLOWED_COMMANDS.contains(&name.as_ref()) {
            return ValidationOutcome::Allowed {
                matched_command: name.to_string(),
            };
        }
    }

    ValidationOutcome::Denied {
        reason: format!(
            "command {:?} is not on the allowed whitelist. \
             Allowed commands: {}",
            cmd,
            ALLOWED_COMMANDS.join(", "),
        ),
    }
}

/// Get the effective timeout for a command.
pub fn effective_timeout(params: &CommandRunParams) -> u64 {
    params
        .timeout_secs
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .min(MAX_TIMEOUT_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use deterministic_protocol::CommandRunParams;

    fn make_params(command: &str, args: &[&str]) -> CommandRunParams {
        CommandRunParams {
            run_id: "r_test".to_string(),
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            workdir: None,
            timeout_secs: None,
        }
    }

    #[test]
    fn allows_cargo_check() {
        let result = validate_command(&make_params("cargo", &["check"]));
        assert_eq!(
            result,
            ValidationOutcome::Allowed {
                matched_command: "cargo".to_string()
            }
        );
    }

    #[test]
    fn allows_cargo_fmt() {
        let result = validate_command(&make_params("cargo", &["fmt", "--check"]));
        assert!(matches!(result, ValidationOutcome::Allowed { .. }));
    }

    #[test]
    fn allows_npm_run_lint() {
        let result = validate_command(&make_params("npm", &["run", "lint"]));
        assert!(matches!(result, ValidationOutcome::Allowed { .. }));
    }

    #[test]
    fn allows_python3() {
        let result = validate_command(&make_params("python3", &["-m", "pytest"]));
        assert!(matches!(result, ValidationOutcome::Allowed { .. }));
    }

    #[test]
    fn allows_make() {
        let result = validate_command(&make_params("make", &["check"]));
        assert!(matches!(result, ValidationOutcome::Allowed { .. }));
    }

    #[test]
    fn denies_empty_command() {
        let result = validate_command(&make_params("", &[]));
        assert_eq!(
            result,
            ValidationOutcome::Denied {
                reason: "command must not be empty".to_string()
            }
        );
    }

    #[test]
    fn denies_shell_injection_semicolon() {
        let result = validate_command(&make_params("cargo", &["check; rm -rf /"]));
        assert!(matches!(result, ValidationOutcome::Denied { .. }));
    }

    #[test]
    fn denies_shell_injection_pipe() {
        let result = validate_command(&make_params("cargo", &["check | cat /etc/passwd"]));
        assert!(matches!(result, ValidationOutcome::Denied { .. }));
    }

    #[test]
    fn denies_shell_injection_backtick() {
        let result = validate_command(&make_params("echo", &["`id`"]));
        assert!(matches!(result, ValidationOutcome::Denied { .. }));
    }

    #[test]
    fn denies_curl() {
        let result = validate_command(&make_params("curl", &["http://evil"]));
        assert!(matches!(result, ValidationOutcome::Denied { .. }));
    }

    #[test]
    fn denies_wget() {
        let result = validate_command(&make_params("wget", &["http://evil"]));
        assert!(matches!(result, ValidationOutcome::Denied { .. }));
    }

    #[test]
    fn denies_sudo() {
        let result = validate_command(&make_params("sudo", &["rm", "-rf", "/"]));
        assert!(matches!(result, ValidationOutcome::Denied { .. }));
    }

    #[test]
    fn denies_sed() {
        let result = validate_command(&make_params("sed", &["-i", "s/foo/bar/g", "file.rs"]));
        assert!(matches!(result, ValidationOutcome::Denied { .. }));
    }

    #[test]
    fn denies_chmod() {
        let result = validate_command(&make_params("chmod", &["+x", "script.sh"]));
        assert!(matches!(result, ValidationOutcome::Denied { .. }));
    }

    #[test]
    fn allows_path_to_allowed_command() {
        let result = validate_command(&make_params("/usr/bin/cargo", &["check"]));
        assert!(matches!(result, ValidationOutcome::Allowed { .. }));
    }

    #[test]
    fn denies_unknown_command() {
        let result = validate_command(&make_params("evil_script", &[]));
        assert!(matches!(result, ValidationOutcome::Denied { .. }));
    }

    #[test]
    fn timeout_defaults_to_60() {
        let params = make_params("cargo", &["check"]);
        assert_eq!(effective_timeout(&params), 60);
    }

    #[test]
    fn timeout_caps_at_300() {
        let mut params = make_params("cargo", &["check"]);
        params.timeout_secs = Some(999);
        assert_eq!(effective_timeout(&params), 300);
    }

    #[test]
    fn timeout_respects_custom_value() {
        let mut params = make_params("cargo", &["check"]);
        params.timeout_secs = Some(120);
        assert_eq!(effective_timeout(&params), 120);
    }
}
