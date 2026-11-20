use config::Config;
use log::info;
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, DbErr, Statement,
};
use std::sync::Arc;
use tokio::time::Duration;

pub mod config;
pub mod logging;

/// Load environment variables from the `.env` file into the process environment.
///
/// Call this at the application entry point (e.g. `main()`) before
/// constructing [`Config`]. This is intentionally separate from `Config::new()`
/// so that test code can construct configs without `.env` side effects.
pub fn load_env_file() {
    dotenvy::dotenv().ok();
}

/// Session store pool size, carved out of `db_max_connections`.
pub const SESSION_POOL_MAX_CONNECTIONS: u32 = 3;

/// The schema holding the application's tables and types.
pub const SCHEMA: &str = "refactor_platform";

/// Main pool `(max, min)` after reserving the session pool; max stays at least 1.
fn main_pool_bounds(db_max_connections: u32, db_min_connections: u32) -> (u32, u32) {
    let max = db_max_connections
        .saturating_sub(SESSION_POOL_MAX_CONNECTIONS)
        .max(1);
    (max, db_min_connections.min(max))
}

pub async fn init_database(config: &Config) -> Result<DatabaseConnection, DbErr> {
    let (max_connections, min_connections) =
        main_pool_bounds(config.db_max_connections, config.db_min_connections);

    info!(
        "Database pool config: max_connections={}, min_connections={}, \
         session_pool_max_connections={}, connect_timeout={}s, acquire_timeout={}s, \
         idle_timeout={}s, max_lifetime={}s",
        max_connections,
        min_connections,
        SESSION_POOL_MAX_CONNECTIONS,
        config.db_connect_timeout_secs,
        config.db_acquire_timeout_secs,
        config.db_idle_timeout_secs,
        config.db_max_lifetime_secs,
    );

    let mut opt = ConnectOptions::new::<&str>(config.database_url());
    opt.max_connections(max_connections)
        .min_connections(min_connections)
        .connect_timeout(Duration::from_secs(config.db_connect_timeout_secs))
        .acquire_timeout(Duration::from_secs(config.db_acquire_timeout_secs))
        .idle_timeout(Duration::from_secs(config.db_idle_timeout_secs))
        .max_lifetime(Duration::from_secs(config.db_max_lifetime_secs))
        .sqlx_logging(true)
        .sqlx_logging_level(log::LevelFilter::Info);
    // No per-connection `search_path`: a transaction-mode pooler would not keep it. The
    // role's per-database default (see migration m20261119_000000) applies instead.

    let db = Database::connect(opt).await?;
    ensure_schema_on_search_path(&db).await?;

    Ok(db)
}

/// Fails unless the connection's `search_path` includes [`SCHEMA`], so a role without the
/// per-database default stops startup instead of failing enum casts on later writes.
async fn ensure_schema_on_search_path(db: &DatabaseConnection) -> Result<(), DbErr> {
    let schemas = db
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            "SELECT array_to_string(current_schemas(false), ',') AS schemas",
        ))
        .await?
        .map(|row| row.try_get::<String>("", "schemas"))
        .transpose()?
        .unwrap_or_default();

    check_search_path(&schemas)
}

/// Ok when `schemas`, the comma-separated list `current_schemas` reports, names [`SCHEMA`].
fn check_search_path(schemas: &str) -> Result<(), DbErr> {
    if schemas.split(',').any(|schema| schema == SCHEMA) {
        Ok(())
    } else {
        Err(DbErr::Custom(format!(
            "search_path does not include {SCHEMA}: the database role has no per-database \
             default. Run migrations as the backend's role, or connect as the role that ran them."
        )))
    }
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

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
