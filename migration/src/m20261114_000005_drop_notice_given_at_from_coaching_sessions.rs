use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Gives each participant a row the column alone was standing in for. DO NOTHING
        // keeps existing rows and covers a coach coaching themselves.
        manager
            .get_connection()
            .execute_unprepared(
                "INSERT INTO refactor_platform.coaching_session_notices
                     (coaching_session_id, user_id, notice_given_at, created_at, updated_at)
                 SELECT cs.id, participant.user_id, cs.notice_given_at, cs.notice_given_at,
                        cs.notice_given_at
                 FROM refactor_platform.coaching_sessions cs
                 JOIN refactor_platform.coaching_relationships cr
                   ON cr.id = cs.coaching_relationship_id
                 CROSS JOIN LATERAL (VALUES (cr.coach_id), (cr.coachee_id)) AS participant(user_id)
                 ON CONFLICT (coaching_session_id, user_id) DO NOTHING",
            )
            .await?;

        // The column follows the coachee's notice, so carrying a later value into their row
        // keeps the reminder sweep's answer the same across the cutover.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.coaching_session_notices n
                 SET notice_given_at = cs.notice_given_at, updated_at = NOW()
                 FROM refactor_platform.coaching_sessions cs
                 JOIN refactor_platform.coaching_relationships cr
                   ON cr.id = cs.coaching_relationship_id
                 WHERE n.coaching_session_id = cs.id
                   AND n.user_id = cr.coachee_id
                   AND n.notice_given_at < cs.notice_given_at",
            )
            .await?;

        // Notice now lives only in coaching_session_notices.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 DROP COLUMN notice_given_at",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Nullable until every row is filled.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 ADD COLUMN notice_given_at TIMESTAMPTZ",
            )
            .await?;

        // The column tracked the coachee's notice; booking time is the original backfill's
        // fallback for a session without a coachee row.
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE refactor_platform.coaching_sessions cs
                 SET notice_given_at = COALESCE(n.notice_given_at, s.created_at)
                 FROM refactor_platform.coaching_sessions s
                 JOIN refactor_platform.coaching_relationships cr
                   ON cr.id = s.coaching_relationship_id
                 LEFT JOIN refactor_platform.coaching_session_notices n
                   ON n.coaching_session_id = s.id AND n.user_id = cr.coachee_id
                 WHERE s.id = cs.id",
            )
            .await?;

        // Matches the original column definition.
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE refactor_platform.coaching_sessions
                 ALTER COLUMN notice_given_at SET DEFAULT NOW(),
                 ALTER COLUMN notice_given_at SET NOT NULL",
            )
            .await?;

        Ok(())
    }
}
