use chrono::Utc;
use entity::organizations::{
    ActiveModel as OrganizationActiveModel, Column as OrganizationColumn,
    Entity as OrganizationEntity,
};
use entity::organizations_users::{
    ActiveModel as OrganizationUsersActiveModel, Column as OrganizationUsersColumn,
    Entity as OrganizationUsersEntity,
};
use entity::users::{
    ActiveModel as UsersActiveModel, Column as UsersColumn, Entity as UsersEntity,
};
use password_auth::generate_hash;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, Set};
use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ModelTrait, QueryFilter};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        insert_initial_admin_user_and_org(manager).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        delete_initial_admin_user_and_org(manager).await
    }
}

async fn insert_initial_admin_user_and_org(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let db = manager.get_connection();
    let now = Utc::now();

    let admin_user = UsersActiveModel {
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
    .unwrap();

    let org = OrganizationActiveModel {
        name: Set("Refactor Group".to_owned()),
        slug: Set("refactor-group".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    OrganizationUsersActiveModel {
        organization_id: org.id,
        user_id: admin_user.id,
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    Ok(())
}

async fn delete_initial_admin_user_and_org(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let db = manager.get_connection();

    let user = UsersEntity::find()
        .filter(UsersColumn::Email.eq("admin@refactorcoach.com"))
        .one(db)
        .await?
        .unwrap();

    let org = OrganizationEntity::find()
        .filter(OrganizationColumn::Name.eq("Refactor Group"))
        .one(db)
        .await?
        .unwrap();

    let organizations_users_join = OrganizationUsersEntity::find()
        .filter(OrganizationUsersColumn::OrganizationId.eq(org.id))
        .filter(OrganizationUsersColumn::UserId.eq(user.id))
        .one(db)
        .await?
        .unwrap();

    organizations_users_join.delete(db).await?;
    user.delete(db).await?;
    org.delete(db).await?;

    Ok(())
}
