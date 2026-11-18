//! Transaction-scoped Postgres advisory locks, in the two-key form.
//!
//! Each kind of lock has its own first key (namespace): 1 for relationship pairs in
//! `coaching_relationship`, 2 for organizations' audit chains in `audit_event`.

use sea_orm::{ConnectionTrait, DbBackend, Statement};

use super::error::Error;

/// Takes the advisory lock `(namespace, key)`, held until the transaction ends.
///
/// Does nothing on SQLite, the test tier, which has no advisory locks and runs on one
/// connection.
pub(crate) async fn advisory_xact_lock(
    db: &impl ConnectionTrait,
    namespace: i32,
    key: i32,
) -> Result<(), Error> {
    if db.get_database_backend() != DbBackend::Postgres {
        return Ok(());
    }
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT pg_advisory_xact_lock($1, $2)",
        [namespace.into(), key.into()],
    ))
    .await?;
    Ok(())
}

/// `value` folded into a 32-bit lock key, with both halves contributing.
pub(crate) fn fold(value: u64) -> i32 {
    (value ^ (value >> 32)) as i32
}
