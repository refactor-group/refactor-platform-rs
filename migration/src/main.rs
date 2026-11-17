use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::TransactionTrait;
use sea_orm_migration::MigratorTraitSelf;

use migration::Migrator;

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
        Migrator.up(&txn, steps).await?;
        txn.commit().await
    }
}

#[tokio::main]
async fn main() {
    cli::run_cli(AllOrNothing).await;
}
