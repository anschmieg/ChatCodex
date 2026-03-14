//! Entry point for the deterministic daemon.

use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let store_dir = std::env::var("DETERMINISTIC_STORE_DIR")
        .unwrap_or_else(|_| "/tmp/deterministic-daemon".to_string());
    let bind_addr = std::env::var("DETERMINISTIC_BIND")
        .unwrap_or_else(|_| "127.0.0.1:19280".to_string());

    let store = deterministic_daemon::persistence::Store::open(std::path::Path::new(&store_dir))?;
    let state = Arc::new(deterministic_daemon::router::AppState { store });

    let app = deterministic_daemon::router::build_router(state);

    tracing::info!("deterministic daemon listening on {bind_addr}");
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
