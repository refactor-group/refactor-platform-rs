use super::error::Error;
use entity::organizations_users::{Column, Entity};
use entity::Id;
use sea_orm::{ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter};

pub async fn delete_by_user_id(db: &impl ConnectionTrait, user_id: Id) -> Result<(), Error> {
    Entity::delete_many()
        .filter(Condition::all().add(Column::UserId.eq(user_id)))
        .exec(db)
        .await?;
    Ok(())
}

#[cfg(test)]
// We need to gate seaORM's mock feature behind conditional compilation because
// the feature removes the Clone trait implementation from seaORM's DatabaseConnection.
// see https://github.com/SeaQL/sea-orm/issues/830
#[cfg(feature = "mock")]
mod test {
    use super::*;
    use entity::Id;
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

    #[tokio::test]
    async fn test_delete_by_user_id() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let user_id = Id::new_v4();
        let _ = delete_by_user_id(&db, user_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"DELETE FROM "refactor_platform"."organizations_users" WHERE "organizations_users"."user_id" = $1"#,
                [user_id.into()]
            )]
        );

        Ok(())
    }
}
