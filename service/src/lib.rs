use config::Config;
use log::info;
use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr};
use std::sync::Arc;
use tokio::time::Duration;

pub mod config;
pub mod logging;

pub async fn init_database(config: &Config) -> Result<DatabaseConnection, DbErr> {
    info!(
        "Database pool config: max_connections={}, min_connections={}, \
         connect_timeout={}s, acquire_timeout={}s, idle_timeout={}s, max_lifetime={}s",
        config.db_max_connections,
        config.db_min_connections,
        config.db_connect_timeout_secs,
        config.db_acquire_timeout_secs,
        config.db_idle_timeout_secs,
        config.db_max_lifetime_secs,
    );

    let mut opt = ConnectOptions::new::<&str>(config.database_url());
    opt.max_connections(config.db_max_connections)
        .min_connections(config.db_min_connections)
        .connect_timeout(Duration::from_secs(config.db_connect_timeout_secs))
        .acquire_timeout(Duration::from_secs(config.db_acquire_timeout_secs))
        .idle_timeout(Duration::from_secs(config.db_idle_timeout_secs))
        .max_lifetime(Duration::from_secs(config.db_max_lifetime_secs))
        .sqlx_logging(true)
        .sqlx_logging_level(log::LevelFilter::Info)
        .set_schema_search_path("refactor_platform"); // Setting default PostgreSQL schema

    let db = Database::connect(opt).await?;

    Ok(db)
}

// Service-level state containing only infrastructure concerns
// Needs to implement Clone to be able to be passed into Router as State
#[derive(Clone)]
pub struct AppState {
    pub database_connection: Arc<DatabaseConnection>,
    pub config: Config,
}

impl AppState {
    pub fn new(app_config: Config, db: &Arc<DatabaseConnection>) -> Self {
        Self {
            database_connection: Arc::clone(db),
            config: app_config,
        }
    }

    pub fn db_conn_ref(&self) -> &DatabaseConnection {
        self.database_connection.as_ref()
    }

    pub fn set_db_conn(&mut self, db: DatabaseConnection) {
        self.database_connection = Arc::new(db);
    }
}
