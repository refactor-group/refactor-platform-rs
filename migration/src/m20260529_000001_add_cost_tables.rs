use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // pipeline_provider enum: services that form the record → transcribe → analyze chain
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.pipeline_provider AS ENUM ('recall_ai', 'llm_gateway')",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.pipeline_provider OWNER TO refactor")
            .await?;

        // cost_metric enum: what is being measured for billing purposes
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.cost_metric AS ENUM \
                 ('bot_minutes', 'transcription_hours', 'llm_tokens')",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.cost_metric OWNER TO refactor")
            .await?;

        // cost_unit enum: the unit of measure for each cost_metric
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE TYPE refactor_platform.cost_unit AS ENUM ('minutes', 'hours', 'tokens')",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("ALTER TYPE refactor_platform.cost_unit OWNER TO refactor")
            .await?;

        // cost_pricing_config: per-unit rates by provider+metric, with full rate history
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE IF NOT EXISTS refactor_platform.cost_pricing_config (
                    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                    provider            refactor_platform.pipeline_provider NOT NULL,
                    metric              refactor_platform.cost_metric NOT NULL,
                    unit                refactor_platform.cost_unit NOT NULL,
                    cost_per_unit_low   NUMERIC(20, 10) NOT NULL,
                    cost_per_unit_high  NUMERIC(20, 10) NOT NULL,
                    cost_per_unit_avg   NUMERIC(20, 10) NOT NULL,
                    effective_from      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    UNIQUE (provider, metric, effective_from)
                )
                "#,
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.cost_pricing_config OWNER TO refactor",
            )
            .await?;

        // platform_cost_metrics: one row per session per provider+metric, written by webhook handlers
        manager
            .get_connection()
            .execute_unprepared(
                r#"
                CREATE TABLE IF NOT EXISTS refactor_platform.platform_cost_metrics (
                    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
                    provider            refactor_platform.pipeline_provider NOT NULL,
                    metric              refactor_platform.cost_metric NOT NULL,
                    coaching_session_id UUID REFERENCES refactor_platform.coaching_sessions(id)
                                        ON DELETE SET NULL,
                    source_record_id    UUID NOT NULL,
                    cost_low            NUMERIC(14, 6) NOT NULL,
                    cost_high           NUMERIC(14, 6) NOT NULL,
                    cost_avg            NUMERIC(14, 6) NOT NULL,
                    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                    -- Defense-in-depth: one cost row per source record per metric.
                    -- Idempotency normally rests on the upstream claim gates; this
                    -- guards against double-recording if cost recording is ever
                    -- invoked from a second path (backfill/admin).
                    UNIQUE (source_record_id, metric)
                )
                "#,
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.platform_cost_metrics OWNER TO refactor",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_platform_cost_metrics_provider_created_at \
                 ON refactor_platform.platform_cost_metrics(provider, created_at DESC)",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX IF NOT EXISTS idx_platform_cost_metrics_coaching_session_id \
                 ON refactor_platform.platform_cost_metrics(coaching_session_id)",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "DROP INDEX IF EXISTS \
                 refactor_platform.idx_platform_cost_metrics_coaching_session_id",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "DROP INDEX IF EXISTS \
                 refactor_platform.idx_platform_cost_metrics_provider_created_at",
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.platform_cost_metrics")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TABLE IF EXISTS refactor_platform.cost_pricing_config")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.cost_unit")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.cost_metric")
            .await?;

        manager
            .get_connection()
            .execute_unprepared("DROP TYPE IF EXISTS refactor_platform.pipeline_provider")
            .await?;

        Ok(())
    }
}
