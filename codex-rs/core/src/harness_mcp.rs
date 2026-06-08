//! Model-free access to the deterministic Codex tool harness.

use crate::tools::spec::ToolsConfig;
use crate::tools::spec::build_specs_with_discoverable_tools;
use serde::Serialize;
use serde_json::Value;

/// A native Codex tool definition ready to map onto an MCP tool.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HarnessToolSpec {
    pub name: String,
    pub definition: Value,
    pub supports_parallel_tool_calls: bool,
}

/// Build the deterministic model-facing catalog without constructing a model
/// client or starting a Codex turn.
pub fn native_tool_catalog() -> Result<Vec<HarnessToolSpec>, serde_json::Error> {
    let (specs, _) =
        build_specs_with_discoverable_tools(&ToolsConfig::native_harness(), None, None, None, &[])
            .build();

    specs
        .into_iter()
        .map(|configured| {
            let name = configured.spec.name().to_string();
            let definition = serde_json::to_value(configured.spec)?;
            Ok(HarnessToolSpec {
                name,
                definition,
                supports_parallel_tool_calls: configured.supports_parallel_tool_calls,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::native_tool_catalog;
    use serde_json::Value;

    #[test]
    fn native_catalog_uses_codex_tool_specs_without_agent_tools() {
        let catalog = native_tool_catalog().expect("native tool catalog");
        let names = catalog
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            [
                "exec_command",
                "write_stdin",
                "update_plan",
                "apply_patch",
                "view_image",
            ]
        );

        let apply_patch = catalog
            .iter()
            .find(|tool| tool.name == "apply_patch")
            .expect("apply_patch tool");
        assert_eq!(
            apply_patch.definition["type"],
            Value::String("function".into())
        );
        assert_eq!(
            apply_patch.definition["parameters"]["required"],
            serde_json::json!(["input"])
        );

        for forbidden in [
            "codex",
            "codex-reply",
            "spawn_agent",
            "send_input",
            "resume_agent",
            "wait",
            "close_agent",
        ] {
            assert!(
                !names.contains(&forbidden),
                "{forbidden} must not be exposed"
            );
        }
    }
}
