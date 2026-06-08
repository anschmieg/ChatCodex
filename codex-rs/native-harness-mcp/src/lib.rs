//! MCP transport adapter for the deterministic Codex harness.

#![deny(clippy::print_stdout, clippy::print_stderr)]

use std::borrow::Cow;
use std::sync::Arc;

use codex_core::harness_mcp::HarnessToolSpec;
use codex_core::harness_mcp::native_tool_catalog;
use rmcp::ErrorData as McpError;
use rmcp::handler::server::ServerHandler;
use rmcp::model::CallToolRequestParams;
use rmcp::model::CallToolResult;
use rmcp::model::JsonObject;
use rmcp::model::ListToolsResult;
use rmcp::model::PaginatedRequestParams;
use rmcp::model::ServerCapabilities;
use rmcp::model::ServerInfo;
use rmcp::model::Tool;

#[derive(Clone)]
pub struct NativeHarnessMcp {
    tools: Arc<Vec<Tool>>,
}

impl NativeHarnessMcp {
    pub fn new() -> anyhow::Result<Self> {
        let tools = native_tool_catalog()?
            .into_iter()
            .map(tool_from_native_spec)
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            tools: Arc::new(tools),
        })
    }

    pub fn tools(&self) -> &[Tool] {
        self.tools.as_slice()
    }
}

fn tool_from_native_spec(native: HarnessToolSpec) -> anyhow::Result<Tool> {
    let definition = native
        .definition
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("native tool {} is not an object", native.name))?;
    let description = definition
        .get("description")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let input_schema = definition
        .get("parameters")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("native tool {} has no parameters", native.name))?;
    let input_schema: JsonObject = serde_json::from_value(input_schema)?;
    let output_schema = definition
        .get("output_schema")
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .map(Arc::new);

    Ok(Tool {
        name: Cow::Owned(native.name),
        title: None,
        description: Some(Cow::Owned(description)),
        input_schema: Arc::new(input_schema),
        output_schema,
        annotations: None,
        execution: None,
        icons: None,
        meta: None,
    })
}

impl ServerHandler for NativeHarnessMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..ServerInfo::default()
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let tools = Arc::clone(&self.tools);
        async move {
            Ok(ListToolsResult {
                tools: (*tools).clone(),
                next_cursor: None,
                meta: None,
            })
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        Err(McpError::internal_error(
            format!(
                "native dispatch is not implemented yet for {}",
                request.name
            ),
            None,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::NativeHarnessMcp;

    #[test]
    fn mcp_catalog_preserves_native_names_and_input_schemas() {
        let server = NativeHarnessMcp::new().expect("native MCP server");
        let names = server
            .tools()
            .iter()
            .map(|tool| tool.name.as_ref())
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

        let apply_patch = server
            .tools()
            .iter()
            .find(|tool| tool.name == "apply_patch")
            .expect("apply_patch");
        assert_eq!(
            apply_patch.input_schema["required"],
            serde_json::json!(["input"])
        );
    }
}
