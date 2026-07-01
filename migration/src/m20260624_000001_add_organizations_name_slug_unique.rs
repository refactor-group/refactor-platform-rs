use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Pre-flight: fail with an actionable message naming the offending values
        // rather than an opaque 23505 if a non-prod DB seeded duplicate names/slugs.
        manager
            .get_connection()
            .execute_unprepared(
                "DO $$ \
                 DECLARE dups text; \
                 BEGIN \
                   SELECT string_agg(name, ', ') INTO dups FROM ( \
                     SELECT name FROM refactor_platform.organizations \
                     GROUP BY name HAVING count(*) > 1) t; \
                   IF dups IS NOT NULL THEN \
                     RAISE EXCEPTION 'Cannot add UNIQUE index on organizations.name: duplicate names present (%). Dedupe them before migrating.', dups; \
                   END IF; \
                   SELECT string_agg(slug, ', ') INTO dups FROM ( \
                     SELECT slug FROM refactor_platform.organizations \
                     GROUP BY slug HAVING count(*) > 1) t; \
                   IF dups IS NOT NULL THEN \
                     RAISE EXCEPTION 'Cannot add UNIQUE index on organizations.slug: duplicate slugs present (%). Dedupe them before migrating.', dups; \
                   END IF; \
                 END $$",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS organizations_name_key \
                 ON refactor_platform.organizations(name)",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX IF NOT EXISTS organizations_slug_key \
                 ON refactor_platform.organizations(slug)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared("DROP INDEX IF EXISTS refactor_platform.organizations_name_key")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP INDEX IF EXISTS refactor_platform.organizations_slug_key")
            .await?;

        Ok(())
    }
}
