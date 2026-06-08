use codex_native_harness_mcp::NativeHarnessMcp;
use rmcp::ServiceExt;

fn stdio() -> (tokio::io::Stdin, tokio::io::Stdout) {
    (tokio::io::stdin(), tokio::io::stdout())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = NativeHarnessMcp::new()?;
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}
