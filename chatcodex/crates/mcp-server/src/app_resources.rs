use rmcp::model::Meta;
use rmcp::model::RawResource;
use rmcp::model::Resource;
use rmcp::model::ResourceContents;
use serde_json::json;

pub const RUN_STATUS_RESOURCE_URI: &str = "ui://chatcodex/run-status.html";
pub const RUN_STATUS_MIME_TYPE: &str = "text/html;profile=mcp-app";

pub fn run_status_resource() -> Resource {
    let raw = RawResource {
        uri: RUN_STATUS_RESOURCE_URI.to_string(),
        name: "chatcodex-run-status".to_string(),
        title: Some("ChatCodex Run Status".to_string()),
        description: Some(
            "Shows the selected ChatCodex run and requests safe continuation leases.".to_string(),
        ),
        mime_type: Some(RUN_STATUS_MIME_TYPE.to_string()),
        size: None,
        icons: None,
        meta: Some(Meta(
            json!({
                "ui": {
                    "type": "resource",
                    "resourceUri": RUN_STATUS_RESOURCE_URI,
                    "visibility": ["model"],
                    "csp": {
                        "connectDomains": [],
                        "resourceDomains": []
                    }
                },
                "openai/widgetDescription": "Shows ChatCodex run status and safely requests one follow-up turn while work remains.",
                "openai/widgetPrefersBorder": false,
                "openai/widgetCSP": {
                    "connect_domains": [],
                    "resource_domains": []
                }
            })
            .as_object()
            .cloned()
            .unwrap_or_default(),
        )),
    };
    Resource::new(raw, None)
}

pub fn tool_meta(tool_name: &str) -> Option<Meta> {
    if !matches!(
        tool_name,
        "run_start" | "run_get" | "run_update" | "run_resume" | "run_cancel" | "run_followup_lease"
    ) {
        return None;
    }
    let mut meta = json!({
        "openai/widgetAccessible": true,
        "ui": {
            "widgetAccessible": true,
            "visibility": ["model", "component"]
        }
    });
    if tool_renders_run_status(tool_name) {
        if let Some(object) = meta.as_object_mut() {
            object.insert(
                "openai/outputTemplate".to_string(),
                json!(RUN_STATUS_RESOURCE_URI),
            );
            object.insert(
                "openai/toolInvocation/invoking".to_string(),
                json!("Updating ChatCodex run status"),
            );
            object.insert(
                "openai/toolInvocation/invoked".to_string(),
                json!("ChatCodex run status updated"),
            );
            if let Some(ui) = object
                .get_mut("ui")
                .and_then(serde_json::Value::as_object_mut)
            {
                ui.insert("resourceUri".to_string(), json!(RUN_STATUS_RESOURCE_URI));
            }
        }
    }
    meta.as_object().cloned().map(Meta)
}

fn tool_renders_run_status(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "run_start" | "run_get" | "run_update" | "run_resume" | "run_cancel"
    )
}

pub fn run_status_resource_contents() -> ResourceContents {
    ResourceContents::TextResourceContents {
        uri: RUN_STATUS_RESOURCE_URI.to_string(),
        mime_type: Some(RUN_STATUS_MIME_TYPE.to_string()),
        text: run_status_html().to_string(),
        meta: Some(Meta(
            json!({
                "ui": {
                    "type": "resource",
                    "resourceUri": RUN_STATUS_RESOURCE_URI,
                    "visibility": ["model"],
                    "csp": {
                        "connectDomains": [],
                        "resourceDomains": []
                    }
                },
                "openai/widgetDescription": "Shows ChatCodex run status and safely requests one follow-up turn while work remains.",
                "openai/widgetPrefersBorder": false,
                "openai/widgetCSP": {
                    "connect_domains": [],
                    "resource_domains": []
                }
            })
            .as_object()
            .cloned()
            .unwrap_or_default(),
        )),
    }
}

pub fn run_status_html() -> &'static str {
    r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>
:root {
  color-scheme: light dark;
  font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
  background: transparent;
}
body {
  margin: 0;
  padding: 12px;
  color: CanvasText;
}
.wrap {
  display: grid;
  gap: 8px;
  max-width: 560px;
}
.row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}
.label {
  font-size: 12px;
  color: color-mix(in srgb, CanvasText 62%, transparent);
}
.value {
  font-size: 13px;
  font-weight: 600;
  overflow-wrap: anywhere;
}
.bar {
  block-size: 6px;
  border-radius: 3px;
  background: color-mix(in srgb, CanvasText 12%, transparent);
  overflow: hidden;
}
.bar > i {
  display: block;
  block-size: 100%;
  inline-size: var(--progress, 0%);
  background: Highlight;
}
.note {
  min-block-size: 16px;
  font-size: 12px;
  color: color-mix(in srgb, CanvasText 70%, transparent);
}
</style>
</head>
<body>
<main class="wrap" aria-live="polite">
  <div class="row"><span class="label">Run</span><span id="run" class="value">-</span></div>
  <div class="row"><span class="label">State</span><span id="state" class="value">-</span></div>
  <div class="bar" aria-hidden="true"><i id="progress"></i></div>
  <div id="note" class="note"></div>
</main>
<script>
(() => {
  const terminal = new Set(["completed", "cancelled"]);
  const stopped = new Set(["paused", "blocked", "awaiting_approval"]);
  const openai = window.openai;
  const note = document.getElementById("note");

  function source() {
    return openai?.toolOutput || openai?.structuredContent || window.__CHATCODEX_RUN__ || {};
  }

  function runData() {
    const data = source();
    return data.run_metadata || data.run || data;
  }

  function percent(used, max) {
    if (!max || max <= 0) return 0;
    return Math.max(0, Math.min(100, Math.round((used / max) * 100)));
  }

  function draw(run) {
    const id = run.run_id || run.id || "";
    const phase = run.phase || "-";
    const status = run.status || "-";
    document.getElementById("run").textContent = id || "-";
    document.getElementById("state").textContent = `${phase} / ${status}`;
    const limits = run.limits || {};
    document.getElementById("progress").style.setProperty(
      "--progress",
      `${percent(limits.steps_used || 0, limits.max_steps || 0)}%`
    );
    if (!id) note.textContent = "No selected run.";
    else if (terminal.has(status)) note.textContent = "Terminal.";
    else if (stopped.has(status)) note.textContent = "Waiting.";
    else if (run.work_remaining) note.textContent = "Continuation lease pending.";
    else note.textContent = "No remaining work.";
  }

  function newNonce(runId) {
    const random = crypto.randomUUID?.() || Math.random().toString(36).slice(2);
    return `${runId}:${Date.now().toString(36)}:${random}`;
  }

  async function maybeFollowUp(run) {
    const runId = run.run_id || run.id;
    const status = run.status;
    if (!runId || !run.work_remaining || status !== "active") return;
    if (terminal.has(status) || stopped.has(status)) return;
    if (!openai?.callTool || !openai?.sendFollowUpMessage) return;

    const lease = await openai.callTool("run_followup_lease", {
      run_id: runId,
      requested_nonce: newNonce(runId)
    });
    const body = lease?.structuredContent || lease?.structured_content || lease || {};
    if (!body.granted || !body.nonce) {
      note.textContent = body.reason || "Continuation already leased.";
      return;
    }
    const delay = Math.max(0, Math.min(Number(body.delay_ms || 0), 30000));
    const expires = Number(body.expires_at_ms || 0);
    setTimeout(() => {
      if (expires && Date.now() >= expires) {
        note.textContent = "Continuation lease expired.";
        return;
      }
      window.openai.sendFollowUpMessage({ prompt: `Continue ChatCodex run ${runId}. Reload its authoritative state and continue until it reaches a terminal state.` });
    }, delay);
  }

  const run = runData();
  draw(run);
  maybeFollowUp(run).catch((error) => {
    note.textContent = String(error?.message || error || "Continuation unavailable.");
  });
})();
</script>
</body>
</html>"#
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn run_status_component_resource_has_required_mcp_app_metadata() {
        let resource = run_status_resource();
        assert_eq!(resource.uri, RUN_STATUS_RESOURCE_URI);
        assert_eq!(resource.mime_type.as_deref(), Some(RUN_STATUS_MIME_TYPE));
        let meta = resource.meta.clone().expect("resource meta");
        assert_eq!(meta.0["ui"]["csp"]["connectDomains"], serde_json::json!([]),);
        assert_eq!(
            meta.0["openai/widgetDescription"],
            "Shows ChatCodex run status and safely requests one follow-up turn while work remains.",
        );
    }

    #[test]
    fn run_status_component_uses_safe_followup_bridge_contract() {
        let html = run_status_html();
        assert!(html.contains("window.openai"));
        assert!(html.contains("run_followup_lease"));
        assert!(html.contains("sendFollowUpMessage({ prompt:"));
        assert!(html.contains("Continue ChatCodex run"));
        assert!(html.contains("run_id"));
        assert!(html.contains("paused"));
        assert!(html.contains("blocked"));
        assert!(html.contains("awaiting_approval"));
        assert!(html.contains("completed"));
        assert!(html.contains("cancelled"));
        assert!(!html.contains("objective"));
        assert!(!html.contains("acceptance_criteria"));
    }
}
