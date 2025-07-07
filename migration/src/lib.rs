pub use sea_orm_migration::prelude::*;

mod m20240210_153056_create_schema_and_base_db_setup;
mod m20240211_174355_base_migration;
mod m20250509_164646_add_initial_user;
mod m20250610_104530_add_role_to_users;
mod m20250611_115337_promote_admin_user_to_admin_role;
mod m20250705_200000_add_timezone_to_users;

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
        ]
    }
}
