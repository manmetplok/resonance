//! resonance-mcp — MCP stdio server binary.
//!
//! stdout carries ONLY newline-delimited JSON-RPC (the MCP transport);
//! all diagnostics go to stderr (spec MUST, ba doc #266 §5). Register
//! with Claude Code:
//!
//! ```sh
//! claude mcp add --transport stdio resonance -- /path/to/resonance-mcp
//! ```

use rmcp::{transport::stdio, ServiceExt};
use resonance_mcp::{client, ControlClient, ResonanceMcp};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // stdout purity: logs to stderr, no ANSI (Claude Code captures it),
    // and panics too — the default panic hook already writes to stderr,
    // but make the process exit visibly instead of unwinding into a
    // half-alive transport.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let path = client::socket_path();
    tracing::info!("control socket: {}", path.display());
    // Do NOT require the app to be running at startup: tools return an
    // actionable error until it is, and connect lazily per call.
    let control = ControlClient::new(path);

    let service = ResonanceMcp::new(control)
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("failed to serve MCP over stdio: {e:?}"))?;

    // Runs until the client closes the session (stdin EOF).
    service.waiting().await?;
    Ok(())
}
