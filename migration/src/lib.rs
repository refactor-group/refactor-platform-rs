pub use sea_orm_migration::prelude::*;

mod m20240210_153056_create_schema_and_base_db_setup;
mod m20240211_174355_base_migration;
mod m20250509_164646_add_initial_user;
mod m20250610_104530_add_role_to_users;
mod m20250611_115337_promote_admin_user_to_admin_role;
mod m20250705_200000_add_timezone_to_users;
mod m20250730_000000_add_coaching_sessions_sorting_indexes;
mod m20250801_000000_add_sorting_indexes;
mod m20251007_093603_add_user_roles_table_and_super_admin;
mod m20251008_000000_migrate_admin_users_to_super_admin_role;
mod m20251009_000000_migrate_regular_users_to_user_roles;
mod m20251024_000000_remove_organizations_users_table;
mod m20251220_000001_add_oauth_connections;
mod m20251220_000002_add_meeting_fields_to_sessions;
mod m20251228_000001_add_actions_users_table;
mod m20260228_000000_rename_overarching_goals_to_goals;
mod m20260309_000000_goal_relationship_scoping;
mod m20260309_000001_backfill_coaching_sessions_goals;
mod m20260311_064303_add_zoom_to_provider_enum;
mod m20260312_000000_add_goal_id_to_actions;
mod m20260316_000000_fix_goals_session_fk_on_delete;
mod m20260317_000000_add_on_hold_to_status_enum;
mod m20260330_000000_add_magic_link_tokens;
mod m20260407_000001_add_meeting_recordings;
mod m20260407_000002_add_transcriptions;
mod m20260407_000003_add_transcript_segments;
mod m20260511_000000_add_hydrated_at_to_coaching_sessions;
mod m20260513_000000_add_purpose_to_magic_link_tokens;
mod m20260514_000000_add_password_reset_attempts;
mod m20260515_000000_add_duration_minutes_to_coaching_sessions;
mod m20260515_000001_add_default_coaching_session_duration_minutes_to_users;
mod m20260529_000000_rename_provider_to_meeting_provider;
mod m20260529_000001_add_cost_tables;
mod m20260607_000000_add_title_to_coaching_sessions;
mod m20260607_000001_create_coaching_session_topics;
mod m20260607_000002_add_topic_priority_status;
mod m20260610_000000_add_topic_undo_snapshot;
mod m20260610_000000_create_coaching_session_views;
mod m20260611_000000_add_coaching_session_series;
mod m20260611_000000_add_topic_deleted_at;
mod m20260624_000000_add_archive_to_organizations;
mod m20260624_000001_add_organizations_name_slug_unique;
mod m20260701_000000_user_roles_org_fk_restrict;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240210_153056_create_schema_and_base_db_setup::Migration),
            Box::new(m20240211_174355_base_migration::Migration),
            Box::new(m20250509_164646_add_initial_user::Migration),
            Box::new(m20250610_104530_add_role_to_users::Migration),
            Box::new(m20250611_115337_promote_admin_user_to_admin_role::Migration),
            Box::new(m20250705_200000_add_timezone_to_users::Migration),
            Box::new(m20250730_000000_add_coaching_sessions_sorting_indexes::Migration),
            Box::new(m20250801_000000_add_sorting_indexes::Migration),
            Box::new(m20251007_093603_add_user_roles_table_and_super_admin::Migration),
            Box::new(m20251008_000000_migrate_admin_users_to_super_admin_role::Migration),
            Box::new(m20251009_000000_migrate_regular_users_to_user_roles::Migration),
            Box::new(m20251024_000000_remove_organizations_users_table::Migration),
            Box::new(m20251220_000001_add_oauth_connections::Migration),
            Box::new(m20251220_000002_add_meeting_fields_to_sessions::Migration),
            Box::new(m20251228_000001_add_actions_users_table::Migration),
            Box::new(m20260228_000000_rename_overarching_goals_to_goals::Migration),
            Box::new(m20260311_064303_add_zoom_to_provider_enum::Migration),
            Box::new(m20260309_000000_goal_relationship_scoping::Migration),
            Box::new(m20260309_000001_backfill_coaching_sessions_goals::Migration),
            Box::new(m20260312_000000_add_goal_id_to_actions::Migration),
            Box::new(m20260316_000000_fix_goals_session_fk_on_delete::Migration),
            Box::new(m20260317_000000_add_on_hold_to_status_enum::Migration),
            Box::new(m20260330_000000_add_magic_link_tokens::Migration),
            Box::new(m20260407_000001_add_meeting_recordings::Migration),
            Box::new(m20260407_000002_add_transcriptions::Migration),
            Box::new(m20260407_000003_add_transcript_segments::Migration),
            Box::new(m20260511_000000_add_hydrated_at_to_coaching_sessions::Migration),
            Box::new(m20260513_000000_add_purpose_to_magic_link_tokens::Migration),
            Box::new(m20260514_000000_add_password_reset_attempts::Migration),
            Box::new(m20260515_000000_add_duration_minutes_to_coaching_sessions::Migration),
            Box::new(
                m20260515_000001_add_default_coaching_session_duration_minutes_to_users::Migration,
            ),
            Box::new(m20260529_000000_rename_provider_to_meeting_provider::Migration),
            Box::new(m20260529_000001_add_cost_tables::Migration),
            Box::new(m20260607_000000_add_title_to_coaching_sessions::Migration),
            Box::new(m20260607_000001_create_coaching_session_topics::Migration),
            Box::new(m20260607_000002_add_topic_priority_status::Migration),
            Box::new(m20260610_000000_add_topic_undo_snapshot::Migration),
            Box::new(m20260611_000000_add_topic_deleted_at::Migration),
            Box::new(m20260610_000000_create_coaching_session_views::Migration),
            Box::new(m20260611_000000_add_coaching_session_series::Migration),
            Box::new(m20260624_000000_add_archive_to_organizations::Migration),
            Box::new(m20260624_000001_add_organizations_name_slug_unique::Migration),
            Box::new(m20260701_000000_user_roles_org_fk_restrict::Migration),
        ]
    }
}
