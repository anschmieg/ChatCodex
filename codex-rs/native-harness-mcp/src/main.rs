use std::net::SocketAddr;

use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_native_harness_mcp::NativeHarnessMcp;
use codex_native_harness_mcp::http_router;
use rmcp::ServiceExt;

fn stdio() -> (tokio::io::Stdin, tokio::io::Stdout) {
    (tokio::io::stdin(), tokio::io::stdout())
}

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(run)
}

async fn run(arg0_paths: Arg0DispatchPaths) -> anyhow::Result<()> {
    if std::env::var("CHATCODEX_TRANSPORT").as_deref() == Ok("stdio") {
        return run_stdio(arg0_paths).await;
    }

    let bind_addr = std::env::var("CHATCODEX_BIND")
        .unwrap_or_else(|_| "0.0.0.0:3000".to_string())
        .parse::<SocketAddr>()?;
    let bearer_token = std::env::var("CHATCODEX_BEARER_TOKEN")
        .or_else(|_| std::env::var("MCP_AUTH_TOKEN"))
        .map_err(|_| {
            anyhow::anyhow!(
                "CHATCODEX_BEARER_TOKEN or MCP_AUTH_TOKEN must be set for HTTP transport"
            )
        })?;
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, http_router(bearer_token, arg0_paths).await?).await?;
    Ok(())
}

async fn run_stdio(arg0_paths: Arg0DispatchPaths) -> anyhow::Result<()> {
    let server = NativeHarnessMcp::new_with_arg0_paths(arg0_paths).await?;
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}
