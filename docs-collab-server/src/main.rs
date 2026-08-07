//! Binary entrypoint. Initializes logging and hands off to `serve`.

use tracing_subscriber::EnvFilter;

use docs_collab_server::{serve, Config};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    if let Err(e) = serve(Config::new()).await {
        tracing::error!(error = %e, "docs-collab-server exited with error");
        std::process::exit(1);
    }
}
