//! In-memory SQLite for tests that must see what SQL actually selects or writes.
//!
//! Prefer the mock database. Reach for this only when correctness depends on the query
//! itself; see `docs/test-plans/sqlite_integration_testing.md` for when and why.

use std::future::Future;
use std::time::Duration;

use entity::coaching_session_images;
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, EntityTrait, Schema,
};

/// Ceiling on one SQLite test. Per-acquire timeouts still add up across many rows.
const TEST_TIME_LIMIT: Duration = Duration::from_secs(5 * 60);

/// Runs a SQLite test body, failing it once it passes `TEST_TIME_LIMIT`.
pub(crate) async fn within_time_limit<F: Future>(test: F) -> F::Output {
    tokio::time::timeout(TEST_TIME_LIMIT, test)
        .await
        .unwrap_or_else(|_| {
            panic!(
                "SQLite test ran past its {TEST_TIME_LIMIT:?} limit; a query likely escaped its transaction"
            )
        })
}

/// A fresh in-memory database holding the coaching session images table.
pub(crate) async fn database() -> DatabaseConnection {
    let db = empty_database().await;
    create_table(&db, coaching_session_images::Entity).await;
    db
}

/// A fresh in-memory database with no tables.
pub(crate) async fn empty_database() -> DatabaseConnection {
    let mut options = ConnectOptions::new("sqlite::memory:");
    // Every in-memory SQLite connection is its own database, so there must be exactly one.
    // A query that escapes an open transaction then waits for a second connection that never
    // frees up. The short timeout turns that wait into a prompt failure; every legitimate
    // acquire here is sequential and gets the connection back within microseconds.
    options
        .max_connections(1)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(1))
        .sqlx_logging(false);
    let db = Database::connect(options)
        .await
        .expect("in-memory SQLite opens");

    // Entities name the `refactor_platform` schema, which SQLite resolves as an attached
    // database. Foreign keys are off: only the tables under test exist, not their parents.
    for statement in [
        "ATTACH DATABASE ':memory:' AS refactor_platform",
        "PRAGMA foreign_keys = OFF",
    ] {
        db.execute_unprepared(statement)
            .await
            .expect("the database is prepared");
    }

    db
}

/// Creates `entity`'s table, without the indexes a migration would add.
pub(crate) async fn create_table<E: EntityTrait>(db: &DatabaseConnection, entity: E) {
    // Built from the entity so the table tracks its columns. SeaORM marks every primary key
    // auto-increment, which SQLite only accepts on integers, so that one keyword goes.
    let mut create =
        DbBackend::Sqlite.build(&Schema::new(DbBackend::Sqlite).create_table_from_entity(entity));
    create.sql = create.sql.replace(" AUTOINCREMENT", "");
    db.execute_raw(create).await.expect("the table is created");
}
