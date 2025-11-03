use chrono::Utc;
use password_auth::generate_hash;
use sea_orm::{DbBackend, Statement, Value};
use sea_orm_migration::prelude::*;
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

    // Delete organizations_users join (if table still exists)
    let delete_org_users_sql = r#"
        DELETE FROM organizations_users
        WHERE user_id IN (SELECT id FROM users WHERE email = $1)
    "#;
    let _ = db
        .execute(Statement::from_sql_and_values(
            DbBackend::Postgres,
            delete_org_users_sql,
            vec![Value::String(Some(Box::new(
                "admin@refactorcoach.com".to_owned(),
            )))],
        ))
        .await;

    // Delete user
    let delete_user_sql = r#"
        DELETE FROM users WHERE email = $1
    "#;
    db.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        delete_user_sql,
        vec![Value::String(Some(Box::new(
            "admin@refactorcoach.com".to_owned(),
        )))],
    ))
    .await?;

    // Delete organization
    let delete_org_sql = r#"
        DELETE FROM organizations WHERE name = $1
    "#;
    db.execute(Statement::from_sql_and_values(
        DbBackend::Postgres,
        delete_org_sql,
        vec![Value::String(Some(Box::new("Refactor Group".to_owned())))],
    ))
    .await?;

    Ok(())
}
