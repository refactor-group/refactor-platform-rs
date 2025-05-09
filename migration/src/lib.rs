pub use sea_orm_migration::prelude::*;

mod m20240211_174355_base_migration;
mod m20250509_164646_add_initial_non_prod_user;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20240211_174355_base_migration::Migration),
            Box::new(m20250509_164646_add_initial_non_prod_user::Migration),
        ]
    }
}
