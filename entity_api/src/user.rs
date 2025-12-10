use super::error::{EntityApiErrorKind, Error};
use async_trait::async_trait;
use axum_login::{AuthnBackend, UserId};
use chrono::Utc;

use entity::users::{ActiveModel, Column, Entity, Model};
use entity::{roles, user_roles, Id};
use log::*;
use password_auth;
use sea_orm::{
    entity::prelude::*, Condition, ConnectionTrait, DatabaseConnection, Set, TransactionTrait,
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

pub use entity::users::Role;

pub async fn create(db: &impl ConnectionTrait, user_model: Model) -> Result<Model, Error> {
    debug!("New User Relationship Model to be inserted: {user_model:?}");

    let now = Utc::now();
    let user_active_model: ActiveModel = ActiveModel {
        email: Set(user_model.email),
        first_name: Set(user_model.first_name),
        last_name: Set(user_model.last_name),
        display_name: Set(user_model.display_name),
        password: Set(generate_hash(user_model.password)),
        github_username: Set(user_model.github_username),
        github_profile_url: Set(user_model.github_profile_url),
        timezone: Set(user_model.timezone),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    let mut created_user = user_active_model.insert(db).await?;

    // Newly created users will not have roles at this point so we will add an empty vec manually
    created_user.roles = Vec::new();
    Ok(created_user)
}

pub async fn create_by_organization(
    db: &impl TransactionTrait,
    organization_id: Id,
    user_model: Model,
) -> Result<Model, Error> {
    let txn = db.begin().await?;

    let mut user = create(&txn, user_model).await?;
    let now = Utc::now();

    let default_user_role = user_roles::ActiveModel {
        user_id: Set(user.id),
        organization_id: Set(Some(organization_id)),
        role: Set(roles::Role::User),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    let role = default_user_role.insert(&txn).await?;

    user.roles = vec![role];

    txn.commit().await?;

    Ok(user)
}

pub async fn find_by_email(db: &impl ConnectionTrait, email: &str) -> Result<Option<Model>, Error> {
    let results = Entity::find()
        .filter(Column::Email.eq(email))
        .find_with_related(user_roles::Entity)
        .all(db)
        .await?;
    match results.into_iter().next() {
        Some((mut user, roles)) => {
            user.roles = roles;
            Ok(Some(user))
        }
        None => Ok(None),
    }
}

pub async fn find_by_id(db: &impl ConnectionTrait, id: Id) -> Result<Model, Error> {
    let results = Entity::find_by_id(id)
        .find_with_related(user_roles::Entity)
        .all(db)
        .await?;

    match results.into_iter().next() {
        Some((mut user, roles)) => {
            user.roles = roles;
            Ok(user)
        }
        None => Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        }),
    }
}

pub async fn find_by_organization(
    db: &DatabaseConnection,
    organization_id: Id,
) -> Result<Vec<Model>, Error> {
    let results = Entity::find()
        .find_with_related(user_roles::Entity)
        .all(db)
        .await?;

    Ok(results
        .into_iter()
        .filter_map(|(mut user, roles)| {
            // Check if user has any role in the specified organization
            let has_role_in_org = roles
                .iter()
                .any(|r| r.organization_id == Some(organization_id));

            if has_role_in_org {
                user.roles = roles;
                Some(user)
            } else {
                None
            }
        })
        .collect())
}

/// Checks if a user has admin privileges for an organization.
///
/// Returns `true` if the user is:
/// - A SuperAdmin (has `SuperAdmin` role with `organization_id = NULL`), OR
/// - An Admin for the specific organization (has `Admin` role for the given organization)
///
/// This function encapsulates the SeaORM query logic for role checking,
/// keeping database-specific implementation details out of the domain layer.
pub async fn has_admin_access(
    db: &impl ConnectionTrait,
    user_id: Id,
    organization_id: Id,
) -> Result<bool, Error> {
    let admin_role = user_roles::Entity::find()
        .filter(user_roles::Column::UserId.eq(user_id))
        .filter(
            Condition::any()
                // SuperAdmin with organization_id = NULL
                .add(
                    Condition::all()
                        .add(user_roles::Column::Role.eq(Role::SuperAdmin))
                        .add(user_roles::Column::OrganizationId.is_null()),
                )
                // Admin for this specific organization
                .add(
                    Condition::all()
                        .add(user_roles::Column::Role.eq(Role::Admin))
                        .add(user_roles::Column::OrganizationId.eq(organization_id)),
                ),
        )
        .one(db)
        .await?;

    Ok(admin_role.is_some())
}

pub async fn delete(db: &impl ConnectionTrait, user_id: Id) -> Result<(), Error> {
    Entity::delete_by_id(user_id).exec(db).await?;
    Ok(())
}

pub async fn verify_password(password_to_verify: &str, password_hash: &str) -> Result<(), Error> {
    match password_auth::verify_password(password_to_verify, password_hash) {
        Ok(_) => Ok(()),
        Err(_) => Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordUnauthenticated,
        }),
    }
}

pub fn generate_hash(password: String) -> String {
    password_auth::generate_hash(password)
}

async fn authenticate_user(creds: Credentials, user: Model) -> Result<Option<Model>, Error> {
    match password_auth::verify_password(creds.password, &user.password) {
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
        match find_by_email(self.db.as_ref(), &creds.email).await? {
            Some(user) => authenticate_user(creds, user).await,
            None => Err(Error {
                source: None,
                error_kind: EntityApiErrorKind::RecordUnauthenticated,
            }),
        }
    }

    async fn get_user(&self, user_id: &UserId<Self>) -> Result<Option<Self::User>, Self::Error> {
        let results = Entity::find_by_id(*user_id)
            .find_with_related(user_roles::Entity)
            .all(self.db.as_ref())
            .await?;
        match results.into_iter().next() {
            Some((mut user, roles)) => {
                user.roles = roles;
                Ok(Some(user))
            }
            None => Ok(None),
        }
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
                r#"SELECT "users"."id" AS "A_id", "users"."email" AS "A_email", "users"."first_name" AS "A_first_name", "users"."last_name" AS "A_last_name", "users"."display_name" AS "A_display_name", "users"."password" AS "A_password", "users"."github_username" AS "A_github_username", "users"."github_profile_url" AS "A_github_profile_url", "users"."timezone" AS "A_timezone", CAST("users"."role" AS "text") AS "A_role", "users"."created_at" AS "A_created_at", "users"."updated_at" AS "A_updated_at", "user_roles"."id" AS "B_id", CAST("user_roles"."role" AS "text") AS "B_role", "user_roles"."organization_id" AS "B_organization_id", "user_roles"."user_id" AS "B_user_id", "user_roles"."created_at" AS "B_created_at", "user_roles"."updated_at" AS "B_updated_at" FROM "refactor_platform"."users" LEFT JOIN "refactor_platform"."user_roles" ON "users"."id" = "user_roles"."user_id" WHERE "users"."email" = $1 ORDER BY "users"."id" ASC"#,
                [user_email.into()]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_id_returns_a_single_record() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let user_id = Id::new_v4();
        let _ = find_by_id(&db, user_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "users"."id" AS "A_id", "users"."email" AS "A_email", "users"."first_name" AS "A_first_name", "users"."last_name" AS "A_last_name", "users"."display_name" AS "A_display_name", "users"."password" AS "A_password", "users"."github_username" AS "A_github_username", "users"."github_profile_url" AS "A_github_profile_url", "users"."timezone" AS "A_timezone", CAST("users"."role" AS "text") AS "A_role", "users"."created_at" AS "A_created_at", "users"."updated_at" AS "A_updated_at", "user_roles"."id" AS "B_id", CAST("user_roles"."role" AS "text") AS "B_role", "user_roles"."organization_id" AS "B_organization_id", "user_roles"."user_id" AS "B_user_id", "user_roles"."created_at" AS "B_created_at", "user_roles"."updated_at" AS "B_updated_at" FROM "refactor_platform"."users" LEFT JOIN "refactor_platform"."user_roles" ON "users"."id" = "user_roles"."user_id" WHERE "users"."id" = $1 ORDER BY "users"."id" ASC"#,
                [user_id.into()]
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
                r#"SELECT "users"."id" AS "A_id", "users"."email" AS "A_email", "users"."first_name" AS "A_first_name", "users"."last_name" AS "A_last_name", "users"."display_name" AS "A_display_name", "users"."password" AS "A_password", "users"."github_username" AS "A_github_username", "users"."github_profile_url" AS "A_github_profile_url", "users"."timezone" AS "A_timezone", CAST("users"."role" AS "text") AS "A_role", "users"."created_at" AS "A_created_at", "users"."updated_at" AS "A_updated_at", "user_roles"."id" AS "B_id", CAST("user_roles"."role" AS "text") AS "B_role", "user_roles"."organization_id" AS "B_organization_id", "user_roles"."user_id" AS "B_user_id", "user_roles"."created_at" AS "B_created_at", "user_roles"."updated_at" AS "B_updated_at" FROM "refactor_platform"."users" LEFT JOIN "refactor_platform"."user_roles" ON "users"."id" = "user_roles"."user_id" ORDER BY "users"."id" ASC"#,
                []
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn create_by_organization_returns_a_new_user_model() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let user_id = Id::new_v4();
        let organization_id = Id::new_v4();
        let user_role_id = Id::new_v4();

        let user_model = entity::users::Model {
            id: user_id,
            email: "test@test.com".to_owned(),
            first_name: "Test".to_owned(),
            last_name: "User".to_owned(),
            display_name: None,
            password: "password123".to_owned(),
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
            role: entity::users::Role::User,
            roles: vec![],
        };

        let user_role_model = entity::user_roles::Model {
            id: user_role_id,
            user_id,
            organization_id: Some(organization_id),
            role: entity::roles::Role::User,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([[user_model.clone()]])
            .append_query_results([[user_role_model.clone()]])
            .into_connection();

        let user = create_by_organization(&db, organization_id, user_model.clone()).await?;

        assert_eq!(user.id, user_model.id);
        assert_eq!(user.email, user_model.email);
        assert_eq!(user.first_name, user_model.first_name);
        assert_eq!(user.last_name, user_model.last_name);
        // The returned user should have the role populated
        assert_eq!(user.roles.len(), 1);
        assert_eq!(user.roles[0].role, entity::roles::Role::User);

        Ok(())
    }

    #[tokio::test]
    async fn create_by_organization_returns_error_on_duplicate_email() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let user_id = Id::new_v4();
        let organization_id = Id::new_v4();

        let user_model = entity::users::Model {
            id: user_id,
            email: "test@test.com".to_owned(),
            first_name: "Test".to_owned(),
            last_name: "User".to_owned(),
            display_name: None,
            password: "password123".to_owned(),
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
            role: entity::users::Role::User,
            roles: vec![],
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_errors([sea_orm::DbErr::Custom("Duplicate email".to_string())])
            .into_connection();

        let result = create_by_organization(&db, organization_id, user_model).await;
        assert!(result.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn delete_deletes_a_user() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let user_id = Id::new_v4();
        let _ = delete(&db, user_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"DELETE FROM "refactor_platform"."users" WHERE "users"."id" = $1"#,
                [user_id.into()]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn has_admin_access_returns_true_for_super_admin() -> Result<(), Error> {
        let user_id = Id::new_v4();
        let organization_id = Id::new_v4();
        let role_id = Id::new_v4();
        let now = chrono::Utc::now();

        // Create a SuperAdmin role with NULL organization_id
        let super_admin_role = user_roles::Model {
            id: role_id,
            user_id,
            role: Role::SuperAdmin,
            organization_id: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![super_admin_role]])
            .into_connection();

        let result = has_admin_access(&db, user_id, organization_id).await?;

        assert!(
            result,
            "SuperAdmin should have admin access to any organization"
        );

        Ok(())
    }

    #[tokio::test]
    async fn has_admin_access_returns_true_for_organization_admin() -> Result<(), Error> {
        let user_id = Id::new_v4();
        let organization_id = Id::new_v4();
        let role_id = Id::new_v4();
        let now = chrono::Utc::now();

        // Create an Admin role for the specific organization
        let org_admin_role = user_roles::Model {
            id: role_id,
            user_id,
            role: Role::Admin,
            organization_id: Some(organization_id),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![org_admin_role]])
            .into_connection();

        let result = has_admin_access(&db, user_id, organization_id).await?;

        assert!(
            result,
            "Organization Admin should have admin access to their organization"
        );

        Ok(())
    }

    #[tokio::test]
    async fn has_admin_access_returns_false_for_regular_user() -> Result<(), Error> {
        let user_id = Id::new_v4();
        let organization_id = Id::new_v4();

        // Mock returns empty result (no admin roles found)
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<user_roles::Model>::new()])
            .into_connection();

        let result = has_admin_access(&db, user_id, organization_id).await?;

        assert!(!result, "Regular users should not have admin access");

        Ok(())
    }

    #[tokio::test]
    async fn has_admin_access_returns_false_for_admin_of_different_organization(
    ) -> Result<(), Error> {
        let user_id = Id::new_v4();
        let organization_id_a = Id::new_v4(); // Organization being queried
        let _organization_id_b = Id::new_v4(); // Organization where user is admin

        // Mock returns empty result (no matching admin role for org A)
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<user_roles::Model>::new()])
            .into_connection();

        let result = has_admin_access(&db, user_id, organization_id_a).await?;

        assert!(
            !result,
            "Admin of different organization should not have access"
        );

        Ok(())
    }

    #[tokio::test]
    async fn has_admin_access_returns_false_for_nonexistent_user() -> Result<(), Error> {
        let nonexistent_user_id = Id::new_v4();
        let organization_id = Id::new_v4();

        // Mock returns empty result (user doesn't exist)
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<user_roles::Model>::new()])
            .into_connection();

        let result = has_admin_access(&db, nonexistent_user_id, organization_id).await?;

        assert!(!result, "Nonexistent user should not have admin access");

        Ok(())
    }

    #[tokio::test]
    async fn has_admin_access_with_admin_role_for_multiple_organizations() -> Result<(), Error> {
        let user_id = Id::new_v4();
        let organization_id_a = Id::new_v4();
        let organization_id_b = Id::new_v4();
        let role_id = Id::new_v4();
        let now = chrono::Utc::now();

        // Create an Admin role for organization A
        let org_admin_role = user_roles::Model {
            id: role_id,
            user_id,
            role: Role::Admin,
            organization_id: Some(organization_id_a),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![org_admin_role]])
            .into_connection();

        // Should have access to organization A
        let result_a = has_admin_access(&db, user_id, organization_id_a).await?;
        assert!(result_a, "Should have admin access to organization A");

        // Create new mock for organization B query (no matching role)
        let db_b = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<user_roles::Model>::new()])
            .into_connection();

        // Should NOT have access to organization B
        let result_b = has_admin_access(&db_b, user_id, organization_id_b).await?;
        assert!(!result_b, "Should not have admin access to organization B");

        Ok(())
    }
}
