use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // NOTE: SeaORM wraps migrations in transactions, preventing CONCURRENT index creation
        // For production deployments, consider creating these indexes manually:
        // CREATE INDEX CONCURRENTLY coaching_sessions_relationship_date ON refactor_platform.coaching_sessions (coaching_relationship_id, date);

        // Create composite index for coaching_relationship_id + date (most common query pattern)
        manager
            .create_index(
                Index::create()
                    .name("coaching_sessions_relationship_date")
                    .table((
                        Alias::new("refactor_platform"),
                        Alias::new("coaching_sessions"),
                    ))
                    .col(Alias::new("coaching_relationship_id"))
                    .col(Alias::new("date"))
                    .to_owned(),
            )
            .await?;

        // Create individual indexes for sortable columns
        manager
            .create_index(
                Index::create()
                    .name("coaching_sessions_date")
                    .table((
                        Alias::new("refactor_platform"),
                        Alias::new("coaching_sessions"),
                    ))
                    .col(Alias::new("date"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("coaching_sessions_created_at")
                    .table((
                        Alias::new("refactor_platform"),
                        Alias::new("coaching_sessions"),
                    ))
                    .col(Alias::new("created_at"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("coaching_sessions_updated_at")
                    .table((
                        Alias::new("refactor_platform"),
                        Alias::new("coaching_sessions"),
                    ))
                    .col(Alias::new("updated_at"))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop indexes in reverse order
        manager
            .drop_index(
                Index::drop()
                    .name("coaching_sessions_updated_at")
                    .table((
                        Alias::new("refactor_platform"),
                        Alias::new("coaching_sessions"),
                    ))
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("coaching_sessions_created_at")
                    .table((
                        Alias::new("refactor_platform"),
                        Alias::new("coaching_sessions"),
                    ))
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("coaching_sessions_date")
                    .table((
                        Alias::new("refactor_platform"),
                        Alias::new("coaching_sessions"),
                    ))
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("coaching_sessions_relationship_date")
                    .table((
                        Alias::new("refactor_platform"),
                        Alias::new("coaching_sessions"),
                    ))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
