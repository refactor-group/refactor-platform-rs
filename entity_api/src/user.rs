use super::error::{EntityApiErrorKind, Error};
use async_trait::async_trait;
use axum_login::{AuthnBackend, UserId};
use chrono::Utc;
use entity::coaching_relationships::{RelationshipAsCoach, RelationshipAsCoachee};
use entity::users::{ActiveModel, Column, Entity, Model};
use entity::{coaching_relationships, Id};
use log::*;
use password_auth::{generate_hash, verify_password};
use sea_orm::{
    entity::prelude::*, sea_query::Expr, Condition, DatabaseConnection, JoinType, QuerySelect, Set,
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

pub async fn create(db: &DatabaseConnection, user_model: Model) -> Result<Model, Error> {
    debug!(
        "New User Relationship Model to be inserted: {:?}",
        user_model
    );

    let now = Utc::now();

    let user_active_model: ActiveModel = ActiveModel {
        email: Set(user_model.email),
        first_name: Set(user_model.first_name),
        last_name: Set(user_model.last_name),
        display_name: Set(user_model.display_name),
        password: Set(generate_hash(user_model.password)),
        github_username: Set(user_model.github_username),
        github_profile_url: Set(user_model.github_profile_url),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(user_active_model.insert(db).await?)
}

pub async fn find_by_email(db: &DatabaseConnection, email: &str) -> Result<Option<Model>, Error> {
    let user: Option<Model> = Entity::find()
        .filter(Column::Email.eq(email))
        .one(db)
        .await?;

    debug!("User find_by_email result: {:?}", user);

    Ok(user)
}

pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

pub async fn find_by_organization(
    db: &DatabaseConnection,
    organization_id: Id,
) -> Result<Vec<Model>, Error> {
    let query = Entity::find()
        .join_as(
            JoinType::InnerJoin,
            coaching_relationships::Relation::Coaches.def().rev(),
            // alias
            RelationshipAsCoach,
        )
        .join_as(
            JoinType::InnerJoin,
            coaching_relationships::Relation::Coachees.def().rev(),
            // alias
            RelationshipAsCoachee,
        )
        .filter(
            Condition::any()
                .add(
                    Expr::col((
                        RelationshipAsCoach,
                        coaching_relationships::Column::OrganizationId,
                    ))
                    .eq(organization_id),
                )
                .add(
                    Expr::col((
                        RelationshipAsCoachee,
                        coaching_relationships::Column::OrganizationId,
                    ))
                    .eq(organization_id),
                ),
        )
        .distinct();

    let users = query.all(db).await?;

    Ok(users)
}

async fn authenticate_user(creds: Credentials, user: Model) -> Result<Option<Model>, Error> {
    match verify_password(creds.password, &user.password) {
        Ok(_) => Ok(Some(user)),
        Err(_) => Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordUnauthenticated,
        }),
    }
}

#[derive(Debug, Clone)]
pub struct Backend {
    db: Arc<DatabaseConnection>,
}

#[derive(Debug, Clone, ToSchema, IntoParams, Deserialize)]
#[schema(as = domain::user::Credentials)] // OpenAPI schema
pub struct Credentials {
    pub email: String,
    pub password: String,
    pub next: Option<String>,
}

impl Backend {
    pub fn new(db: &Arc<DatabaseConnection>) -> Self {
        info!("** Backend::new()");
        Self {
            // Arc is cloned, but the source DatabaseConnection refers to the same instance
            // as the one passed in to new() (see the Arc documentation for more info)
            db: Arc::clone(db),
        }
    }
}

#[async_trait]
impl AuthnBackend for Backend {
    type User = Model;
    type Credentials = Credentials;
    type Error = Error;

    async fn authenticate(
        &self,
        creds: Self::Credentials,
    ) -> Result<Option<Self::User>, Self::Error> {
        debug!("** authenticate(): {:?}:{:?}", creds.email, creds.password);

        match find_by_email(&self.db, &creds.email).await? {
            Some(user) => authenticate_user(creds, user).await,
            None => Err(Error {
                source: None,
                error_kind: EntityApiErrorKind::RecordUnauthenticated,
            }),
        }
    }

    async fn get_user(&self, user_id: &UserId<Self>) -> Result<Option<Self::User>, Self::Error> {
        debug!("** get_user(): {:?}", *user_id);

        let user: Option<Self::User> = Entity::find_by_id(*user_id).one(self.db.as_ref()).await?;

        debug!("Get user result: {:?}", user);

        Ok(user)
    }
}

pub type AuthSession = axum_login::AuthSession<Backend>;

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
    async fn find_by_email_returns_a_single_record() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let user_email = "test@test.com";
        let _ = find_by_email(&db, user_email).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "users"."id", "users"."email", "users"."first_name", "users"."last_name", "users"."display_name", "users"."password", "users"."github_username", "users"."github_profile_url", "users"."created_at", "users"."updated_at" FROM "refactor_platform"."users" WHERE "users"."email" = $1 LIMIT $2"#,
                [user_email.into(), sea_orm::Value::BigUnsigned(Some(1))]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_id_returns_a_single_record() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let coaching_session_id = Id::new_v4();
        let _ = find_by_id(&db, coaching_session_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "users"."id", "users"."email", "users"."first_name", "users"."last_name", "users"."display_name", "users"."password", "users"."github_username", "users"."github_profile_url", "users"."created_at", "users"."updated_at" FROM "refactor_platform"."users" WHERE "users"."id" = $1 LIMIT $2"#,
                [
                    coaching_session_id.into(),
                    sea_orm::Value::BigUnsigned(Some(1))
                ]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_organization_returns_users_who_are_coaches_or_coachees() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let organization_id = Id::new_v4();
        let _ = find_by_organization(&db, organization_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT DISTINCT "users"."id", "users"."email", "users"."first_name", "users"."last_name", "users"."display_name", "users"."password", "users"."github_username", "users"."github_profile_url", "users"."created_at", "users"."updated_at" FROM "refactor_platform"."users" INNER JOIN "refactor_platform"."coaching_relationships" AS "relationship_as_coach" ON "users"."id" = "relationship_as_coach"."coach_id" INNER JOIN "refactor_platform"."coaching_relationships" AS "relationship_as_coachee" ON "users"."id" = "relationship_as_coachee"."coachee_id" WHERE "relationship_as_coach"."organization_id" = $1 OR "relationship_as_coachee"."organization_id" = $2"#,
                [organization_id.into(), organization_id.into()]
            )]
        );

        Ok(())
    }
}
