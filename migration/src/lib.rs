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
        ]
    }
}
