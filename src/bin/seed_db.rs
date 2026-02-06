use log::{error, info};
use service::{config::Config, logging::Logger};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let config = Config::new();
    Logger::init_logger(&config as &Config);

    info!("Seeding database [{}]...", config.database_url());

    let db = match service::init_database(&config).await {
        Ok(db) => Arc::new(db),
        Err(e) => {
            error!("Failed to establish database connection: {e}");
            std::process::exit(1);
        }
    };

    let service_state = service::AppState::new(config, &db);

    entity_api::seed_database(service_state.db_conn_ref()).await;
}
