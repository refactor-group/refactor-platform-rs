use chrono::Utc;
use entity::organizations::{Column as OrganizationColumn, Entity as OrganizationEntity};
use entity::organizations_users::{
    Column as OrganizationUsersColumn, Entity as OrganizationUsersEntity,
};
use entity::users::{Column as UsersColumn, Entity as UsersEntity};
use password_auth::generate_hash;
use sea_orm::{ColumnTrait, DbBackend, EntityTrait, Statement, Value};
use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_orm::{ModelTrait, QueryFilter};
use uuid::Uuid;

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

// NOTE: We use raw SQL here to avoid issues with entity type changes in future migrations.
// Using the ORM can break if new fields are added later, but raw SQL remains compatible.
async fn insert_initial_admin_user_and_org(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let db = manager.get_connection();
    let now = Utc::now();

    let password_hash = generate_hash("password");

    // Insert admin user
    let user_sql = r#"
        INSERT INTO users (
            email, first_name, last_name, display_name, password, created_at, updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id
    "#;
    let user_row = db
        .query_one(Statement::from_sql_and_values(
            DbBackend::Postgres,
            user_sql,
            vec![
                Value::String(Some(Box::new("admin@refactorcoach.com".to_owned()))),
                Value::String(Some(Box::new("Admin".to_owned()))),
                Value::String(Some(Box::new("Admin".to_owned()))),
                Value::String(Some(Box::new("Admin".to_owned()))),
                Value::String(Some(Box::new(password_hash))),
                Value::ChronoDateTimeUtc(Some(Box::new(now))),
                Value::ChronoDateTimeUtc(Some(Box::new(now))),
            ],
        ))
        .await
        .unwrap();
    let admin_user_id: Uuid = user_row.unwrap().try_get("", "id").unwrap();

    // Insert organization
    let org_sql = r#"
        INSERT INTO organizations (
            name, slug, created_at, updated_at
        ) VALUES ($1, $2, $3, $4)
        RETURNING id
    "#;
    let org_row = db
        .query_one(Statement::from_sql_and_values(
            DbBackend::Postgres,
            org_sql,
            vec![
                Value::String(Some(Box::new("Refactor Group".to_owned()))),
                Value::String(Some(Box::new("refactor-group".to_owned()))),
                Value::ChronoDateTimeUtc(Some(Box::new(now))),
                Value::ChronoDateTimeUtc(Some(Box::new(now))),
            ],
        ))
        .await
        .unwrap();
    let org_id: Uuid = org_row.unwrap().try_get("", "id").unwrap();

    // Insert into organizations_users
    let org_user_sql = r#"
        INSERT INTO organizations_users (
            organization_id, user_id, created_at, updated_at
        ) VALUES ($1, $2, $3, $4)
    "#;
    db.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        org_user_sql,
        vec![
            Value::Uuid(Some(Box::new(org_id))),
            Value::Uuid(Some(Box::new(admin_user_id))),
            Value::ChronoDateTimeUtc(Some(Box::new(now))),
            Value::ChronoDateTimeUtc(Some(Box::new(now))),
        ],
    ))
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
