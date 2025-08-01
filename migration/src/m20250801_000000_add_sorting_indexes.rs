use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create indexes for actions table
        manager
            .create_index(
                Index::create()
                    .name("actions_due_by")
                    .table((Alias::new("refactor_platform"), Alias::new("actions")))
                    .col(Alias::new("due_by"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("actions_created_at")
                    .table((Alias::new("refactor_platform"), Alias::new("actions")))
                    .col(Alias::new("created_at"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("actions_updated_at")
                    .table((Alias::new("refactor_platform"), Alias::new("actions")))
                    .col(Alias::new("updated_at"))
                    .to_owned(),
            )
            .await?;

        // Create indexes for agreements table
        // Note: Indexing on body field is omitted as it's likely to contain long text
        // Consider using a GIN index for full-text search if needed in the future
        manager
            .create_index(
                Index::create()
                    .name("agreements_created_at")
                    .table((Alias::new("refactor_platform"), Alias::new("agreements")))
                    .col(Alias::new("created_at"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("agreements_updated_at")
                    .table((Alias::new("refactor_platform"), Alias::new("agreements")))
                    .col(Alias::new("updated_at"))
                    .to_owned(),
            )
            .await?;

        // Create indexes for overarching_goals table
        manager
            .create_index(
                Index::create()
                    .name("overarching_goals_title")
                    .table((
                        Alias::new("refactor_platform"),
                        Alias::new("overarching_goals"),
                    ))
                    .col(Alias::new("title"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("overarching_goals_created_at")
                    .table((
                        Alias::new("refactor_platform"),
                        Alias::new("overarching_goals"),
                    ))
                    .col(Alias::new("created_at"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("overarching_goals_updated_at")
                    .table((
                        Alias::new("refactor_platform"),
                        Alias::new("overarching_goals"),
                    ))
                    .col(Alias::new("updated_at"))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop indexes for overarching_goals table
        manager
            .drop_index(
                Index::drop()
                    .name("overarching_goals_updated_at")
                    .table((
                        Alias::new("refactor_platform"),
                        Alias::new("overarching_goals"),
                    ))
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("overarching_goals_created_at")
                    .table((
                        Alias::new("refactor_platform"),
                        Alias::new("overarching_goals"),
                    ))
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("overarching_goals_title")
                    .table((
                        Alias::new("refactor_platform"),
                        Alias::new("overarching_goals"),
                    ))
                    .to_owned(),
            )
            .await?;

        // Drop indexes for agreements table
        manager
            .drop_index(
                Index::drop()
                    .name("agreements_updated_at")
                    .table((Alias::new("refactor_platform"), Alias::new("agreements")))
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("agreements_created_at")
                    .table((Alias::new("refactor_platform"), Alias::new("agreements")))
                    .to_owned(),
            )
            .await?;

        // Drop indexes for actions table
        manager
            .drop_index(
                Index::drop()
                    .name("actions_updated_at")
                    .table((Alias::new("refactor_platform"), Alias::new("actions")))
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("actions_created_at")
                    .table((Alias::new("refactor_platform"), Alias::new("actions")))
                    .to_owned(),
            )
            .await?;

        manager
            .drop_index(
                Index::drop()
                    .name("actions_due_by")
                    .table((Alias::new("refactor_platform"), Alias::new("actions")))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
