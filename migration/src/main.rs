use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{RuntimeErr, TransactionTrait};
use sea_orm_migration::MigratorTraitSelf;

use migration::Migrator;

/// Postgres's "unsafe use of new value": an enum value added in this transaction was used.
const UNSAFE_NEW_ENUM_VALUE: &str = "55P04";

/// Runs `up` against an existing database as one transaction, so a failed run applies
/// nothing; every other subcommand behaves exactly as the stock CLI.
struct AllOrNothing;

#[async_trait::async_trait]
impl MigratorTraitSelf for AllOrNothing {
    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        Migrator.migrations()
    }

    async fn up<'c, C>(&self, db: C, steps: Option<u32>) -> Result<(), DbErr>
    where
        C: IntoSchemaManagerConnection<'c>,
    {
        if let Some(name) = self
            .migrations()
            .iter()
            .find(|m| m.use_transaction() == Some(false))
            .map(|m| m.name().to_owned())
        {
            return Err(DbErr::Migration(format!(
                "Migration '{name}' opts out of transactions, so `up` cannot apply it atomically"
            )));
        }

        // A fresh database has no data to protect, and its full history cannot run as one
        // transaction: Postgres rejects using an enum value added earlier in the same one.
        let db = db.into_database_executor();
        if self.get_applied_migrations_read_only(&db).await?.is_empty() {
            return Migrator.up(db, steps).await;
        }

        let txn = db.begin().await?;
        Migrator.up(&txn, steps).await.map_err(explain)?;
        txn.commit().await
    }
}

/// Adds the way out to the one failure a single transaction causes on its own.
fn explain(err: DbErr) -> DbErr {
    let unsafe_new_enum_value = matches!(
        &err,
        DbErr::Exec(RuntimeErr::SqlxError(e)) | DbErr::Query(RuntimeErr::SqlxError(e))
            if e.as_database_error().and_then(|e| e.code()).as_deref() == Some(UNSAFE_NEW_ENUM_VALUE)
    );
    if !unsafe_new_enum_value {
        return err;
    }
    DbErr::Migration(format!(
        "{err}\n\
         Nothing was applied. A pending migration writes an enum value that an earlier pending \
         migration adds, and Postgres allows that only once the value is committed. Apply the \
         pending migrations through the one that adds the value with `migrationctl up -n <count>`, \
         then run `up` again. See \"Recovering from a stuck deploy\" in docs/cicd/README.md."
    ))
}

#[tokio::main]
async fn main() {
    cli::run_cli(AllOrNothing).await;
}
