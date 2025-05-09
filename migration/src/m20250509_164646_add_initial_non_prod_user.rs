use chrono::Utc;
use entity::users::{ActiveModel, Column, Entity};
use password_auth::generate_hash;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, Set};
use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ModelTrait, QueryFilter};
use service::config::RustEnv;
use std::env;
use std::str::FromStr;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let rust_env: RustEnv = RustEnv::from_str(
            env::var("RUST_ENV")
                .unwrap_or_else(|_| "development".to_string())
                .as_str(),
        )
        .unwrap();

        match rust_env {
            RustEnv::Development => insert_initial_admin_user(manager).await,
            RustEnv::Staging => insert_initial_admin_user(manager).await,
            RustEnv::Production => {
                // We have a different process for initial setup in production
                Ok(())
            }
        }
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let rust_env: RustEnv = RustEnv::from_str(
            env::var("RUST_ENV")
                .unwrap_or_else(|_| "development".to_string())
                .as_str(),
        )
        .unwrap();

        match rust_env {
            RustEnv::Development => delete_initial_admin_user(manager).await,
            RustEnv::Staging => delete_initial_admin_user(manager).await,
            RustEnv::Production => {
                // We have a different process for initial setup in production
                Ok(())
            }
        }
    }
}

async fn insert_initial_admin_user(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let db = manager.get_connection();
    let now = Utc::now();

    ActiveModel {
        email: Set("admin@refactorcoach.com".to_owned()),
        first_name: Set("admin".to_owned()),
        last_name: Set("admin".to_owned()),
        display_name: Set(Some("admin".to_owned())),
        password: Set(generate_hash("password")),
        github_username: Set(None),
        github_profile_url: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .map_err(|e| DbErr::from(e))?;

    Ok(())
}

async fn delete_initial_admin_user(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let db = manager.get_connection();

    let user = Entity::find()
        .filter(Column::Email.eq("admin@refactorcoach.com"))
        .one(db)
        .await?
        .unwrap();

    user.delete(db).await?;

    Ok(())
}
