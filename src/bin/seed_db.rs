use log::info;
use service::{config::Config, logging::Logger, AppState};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let config = Config::new();
    Logger::init_logger(&config as &Config);

    info!("Seeding database [{}]...", config.database_url());

    let db = Arc::new(service::init_database(config.database_url()).await.unwrap());
    let sse_manager = Arc::new(sse::Manager::new());

    let app_state = AppState::new(config, &db, sse_manager);

    entity_api::seed_database(app_state.db_conn_ref()).await;
}
