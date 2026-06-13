use std::net::SocketAddr;

use codex_arg0::Arg0DispatchPaths;
use codex_arg0::arg0_dispatch_or_else;
use codex_native_harness_mcp::NativeHarnessMcp;
use codex_native_harness_mcp::http_router;
use rmcp::ServiceExt;
use tracing_subscriber::EnvFilter;

fn stdio() -> (tokio::io::Stdin, tokio::io::Stdout) {
    (tokio::io::stdin(), tokio::io::stdout())
}

fn main() -> anyhow::Result<()> {
    arg0_dispatch_or_else(run)
}

async fn run(arg0_paths: Arg0DispatchPaths) -> anyhow::Result<()> {
    // Initialize structured logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();

    if std::env::var("CHATCODEX_TRANSPORT").as_deref() == Ok("stdio") {
        return run_stdio(arg0_paths).await;
    }

    let bind_addr = std::env::var("CHATCODEX_BIND")
        .unwrap_or_else(|_| "0.0.0.0:3000".to_string())
        .parse::<SocketAddr>()?;
    // Validate OAuth config eagerly so the operator gets a clear error if
    // CHATCODEX_PUBLIC_BASE_URL / CHATCODEX_CF_ACCESS_TEAM /
    // CHATCODEX_CF_ACCESS_AUD are missing or malformed.
    codex_native_harness_mcp_auth::AuthConfig::from_env()?;
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;

    tracing::info!(bind_addr = %bind_addr, "starting ChatCodex MCP server");

    axum::serve(listener, http_router(arg0_paths).await?)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("server stopped");
    Ok(())
}

async fn run_stdio(arg0_paths: Arg0DispatchPaths) -> anyhow::Result<()> {
    let server = NativeHarnessMcp::new_with_arg0_paths(arg0_paths).await?;
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received, starting graceful shutdown");
}
