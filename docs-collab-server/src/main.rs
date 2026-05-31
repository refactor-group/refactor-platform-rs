//! Binary entrypoint. Wires `Config` to the axum router + storage + registry.

use docs_collab_server::Config;

#[tokio::main]
async fn main() {
    let _config = Config::new();
    todo!("server bootstrap in Phase 7")
}
