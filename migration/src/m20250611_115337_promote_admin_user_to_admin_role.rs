use entity::roles::Role;
use entity::users;
use entity_api::{mutate, mutate::UpdateMap, user};
use sea_orm::{IntoActiveModel, Value};
use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let email = "admin@refactorcoach.com";
        let db = manager.get_connection();

        let user = user::find_by_email(db, email).await.unwrap();

        if let Some(user) = user {
            let active_model = user.into_active_model();
            let mut update_map = UpdateMap::new();
            update_map.insert(
                "role".to_string(),
                Some(Value::String(Some(Box::new(Role::Admin.to_string())))),
            );
            mutate::update::<users::ActiveModel, users::Column>(db, active_model, update_map)
                .await
                .unwrap();
        }

        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let email = "admin@refactorcoach.com";
        let db = manager.get_connection();

        let user = user::find_by_email(db, email).await.unwrap();

        if let Some(user) = user {
            let active_model = user.into_active_model();
            let mut update_map = UpdateMap::new();
            update_map.insert(
                "role".to_string(),
                Some(Value::String(Some(Box::new(Role::User.to_string())))),
            );
            mutate::update::<users::ActiveModel, users::Column>(db, active_model, update_map)
                .await
                .unwrap();
        }

        Ok(())
    }
}
