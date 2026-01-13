use config::Config;
use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr};
use std::sync::Arc;
use tokio::time::Duration;

pub mod config;
pub mod logging;

pub async fn init_database(database_url: &str) -> Result<DatabaseConnection, DbErr> {
    let mut opt = ConnectOptions::new::<&str>(database_url);
    opt.max_connections(100)
        .min_connections(5)
        .connect_timeout(Duration::from_secs(8))
        .acquire_timeout(Duration::from_secs(8))
        .idle_timeout(Duration::from_secs(8))
        .max_lifetime(Duration::from_secs(8))
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
