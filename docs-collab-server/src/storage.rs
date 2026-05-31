//! Persistence of Yjs document state.
//!
//! `Storage` is the abstract interface; `MemoryStorage` backs tests,
//! `PostgresStorage` backs runtime via `sqlx`.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("backend error: {0}")]
    Backend(String),
    #[error("not found")]
    NotFound,
}

/// Blob persistence for a single named document.
#[async_trait]
pub trait Storage: Send + Sync + 'static {
    /// Load the persisted state for `name`, or `None` if absent.
    async fn fetch(&self, name: &str) -> Result<Option<Vec<u8>>, StorageError>;

    /// Upsert the document's state bytes.
    async fn store(&self, name: &str, state: &[u8]) -> Result<(), StorageError>;

    /// Remove the row for `name`; not-found is not an error.
    async fn delete(&self, name: &str) -> Result<(), StorageError>;
}

/// In-memory storage backend for tests.
#[derive(Debug, Default)]
pub struct MemoryStorage {
    inner: Mutex<HashMap<String, Vec<u8>>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Storage for MemoryStorage {
    async fn fetch(&self, name: &str) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(self.inner.lock().unwrap().get(name).cloned())
    }

    async fn store(&self, name: &str, state: &[u8]) -> Result<(), StorageError> {
        self.inner
            .lock()
            .unwrap()
            .insert(name.to_string(), state.to_vec());
        Ok(())
    }

    async fn delete(&self, name: &str) -> Result<(), StorageError> {
        self.inner.lock().unwrap().remove(name);
        Ok(())
    }
}

/// PostgreSQL storage backend writing to `<schema>.collab_documents`.
pub struct PostgresStorage {
    pool: PgPool,
    schema: String,
}

impl PostgresStorage {
    /// Connect, ensure the schema and table exist, return a ready instance.
    pub async fn connect(database_url: &str, schema: &str) -> Result<Self, StorageError> {
        validate_schema_ident(schema)?;

        let pool = PgPoolOptions::new()
            .connect(database_url)
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;

        let create_schema = format!("CREATE SCHEMA IF NOT EXISTS {schema}");
        run_bootstrap_ddl(&pool, &create_schema).await?;

        let create_table = format!(
            "CREATE TABLE IF NOT EXISTS {schema}.collab_documents (\
                 name TEXT PRIMARY KEY, \
                 state BYTEA NOT NULL, \
                 updated_at TIMESTAMPTZ NOT NULL DEFAULT now()\
             )"
        );
        run_bootstrap_ddl(&pool, &create_table).await?;

        Ok(Self {
            pool,
            schema: schema.to_string(),
        })
    }
}

/// Run idempotent bootstrap DDL, absorbing the race when concurrent connects
/// both try to create the same schema or table. `IF NOT EXISTS` is not atomic
/// at the catalog level, so two sessions can still collide on the `pg_namespace`
/// or `pg_class` unique index; the lost-race SQLSTATEs (`23505`, `42P06`,
/// `42P07`) mean the object now exists, which is exactly what we want.
async fn run_bootstrap_ddl(pool: &PgPool, sql: &str) -> Result<(), StorageError> {
    match sqlx::query(sql).execute(pool).await {
        Ok(_) => Ok(()),
        Err(e) if is_concurrent_bootstrap_race(&e) => Ok(()),
        Err(e) => Err(StorageError::Backend(e.to_string())),
    }
}

fn is_concurrent_bootstrap_race(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .and_then(|d| d.code())
        .map(|c| matches!(c.as_ref(), "23505" | "42P06" | "42P07"))
        .unwrap_or(false)
}

/// Reject schema names that aren't `[A-Za-z_][A-Za-z0-9_]*`. DDL can't bind
/// identifiers, so this guards SQL injection through a misconfigured schema.
fn validate_schema_ident(schema: &str) -> Result<(), StorageError> {
    let mut chars = schema.chars();
    let first = chars
        .next()
        .ok_or_else(|| StorageError::Backend("schema name must not be empty".to_string()))?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(StorageError::Backend(format!(
            "invalid schema identifier: {schema}"
        )));
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '_') {
            return Err(StorageError::Backend(format!(
                "invalid schema identifier: {schema}"
            )));
        }
    }
    Ok(())
}

#[async_trait]
impl Storage for PostgresStorage {
    async fn fetch(&self, name: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let sql = format!(
            "SELECT state FROM {}.collab_documents WHERE name = $1",
            self.schema
        );
        let row = sqlx::query(&sql)
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        match row {
            Some(r) => {
                let bytes: Vec<u8> = r
                    .try_get("state")
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                Ok(Some(bytes))
            }
            None => Ok(None),
        }
    }

    async fn store(&self, name: &str, state: &[u8]) -> Result<(), StorageError> {
        let sql = format!(
            "INSERT INTO {}.collab_documents (name, state) VALUES ($1, $2) \
             ON CONFLICT (name) DO UPDATE SET state = EXCLUDED.state, updated_at = now()",
            self.schema
        );
        sqlx::query(&sql)
            .bind(name)
            .bind(state)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }

    async fn delete(&self, name: &str) -> Result<(), StorageError> {
        let sql = format!(
            "DELETE FROM {}.collab_documents WHERE name = $1",
            self.schema
        );
        sqlx::query(&sql)
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }
}
