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
    let bind_addr =
        std::env::var("DETERMINISTIC_BIND").unwrap_or_else(|_| "127.0.0.1:19280".to_string());
    let workspace_root = std::env::var("DETERMINISTIC_WORKSPACE_ROOT").ok();

    let store = deterministic_daemon::persistence::Store::open(std::path::Path::new(&store_dir))?;
    let hybrid_config = deterministic_core::HybridConfig::load_from_env();
    let state = Arc::new(deterministic_daemon::router::AppState {
        store,
        hybrid_config: hybrid_config.clone(),
        workspace_root,
    });

    if hybrid_config.is_enabled() {
        tracing::info!(
            "hybrid mode enabled; provider={} model={}",
            hybrid_config
                .get_default_profile()
                .map(|p| p.base_url.as_str())
                .unwrap_or("N/A"),
            hybrid_config
                .get_default_profile()
                .map(|p| p.model.as_str())
                .unwrap_or("N/A")
        );
    } else {
        tracing::info!("hybrid mode disabled (default)");
    }

    let app = deterministic_daemon::router::build_router(state);
    tracing::info!("deterministic daemon listening on {bind_addr}");
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
