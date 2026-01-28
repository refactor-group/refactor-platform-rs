use log::info;
use service::{config::Config, logging::Logger};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let config = Config::new();
    Logger::init_logger(&config as &Config);

    info!("Seeding database [{}]...", config.database_url());

    let db = Arc::new(service::init_database(config.database_url()).await.unwrap());

    let service_state = service::AppState::new(config, &db);

    entity_api::seed_database(service_state.db_conn_ref()).await;
}
