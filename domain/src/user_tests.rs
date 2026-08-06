use super::*;
use crate::{coaching_relationships, organizations, user_roles};
use sea_orm::{DatabaseBackend, DbErr, MockDatabase};

fn organization(id: Id) -> organizations::Model {
    let now = chrono::Utc::now();
    organizations::Model {
        id,
        name: "Test Org".to_owned(),
        logo: None,
        slug: "test-org".to_owned(),
        created_at: now.into(),
        updated_at: now.into(),
        archived_at: None,
        archived_by: None,
    }
}

fn user(id: Id) -> users::Model {
    let now = chrono::Utc::now();
    users::Model {
        id,
        email: "member@test.com".to_owned(),
        first_name: "Member".to_owned(),
        last_name: "User".to_owned(),
        display_name: None,
        password: None,
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
        created_at: now.into(),
        updated_at: now.into(),
        role: Role::User,
        roles: vec![],
        invite_status: None,
    }
}

fn user_role_model(user_id: Id, organization_id: Id) -> user_roles::Model {
    let now = chrono::Utc::now();
    user_roles::Model {
        id: Id::new_v4(),
        user_id,
        organization_id: Some(organization_id),
        role: Role::User,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

/// Every statement the mock saw, flattened across transactions.
fn statements(db: sea_orm::DatabaseConnection) -> Vec<String> {
    db.into_transaction_log()
        .iter()
        .flat_map(|transaction| {
            transaction
                .statements()
                .iter()
                .map(|statement| statement.sql.clone())
        })
        .collect()
}

/// Mocks the queries `entity_api::user::create_by_organization` issues.
fn mock_user_creation(
    db: MockDatabase,
    organization_id: Id,
    new_user: users::Model,
) -> MockDatabase {
    let role = user_role_model(new_user.id, organization_id);
    db.append_query_results([[organization(organization_id)]])
        .append_query_results([[new_user]])
        .append_query_results([[role]])
}

/// Mocks the lookups `entity_api::coaching_relationship::create` runs before it
/// inserts: the organization, both parties, their memberships, the duplicate
/// check, then both parties again for the slug.
fn mock_relationship_preflight(
    db: MockDatabase,
    organization_id: Id,
    coach: users::Model,
    coachee: users::Model,
) -> MockDatabase {
    let membership = [organization(organization_id)];
    db.append_query_results([[organization(organization_id)]])
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([
            vec![(coach.clone(), None)],
            vec![(coachee.clone(), None)],
        ])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .append_query_results([membership.clone()])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .append_query_results([membership])
        .append_query_results([Vec::<coaching_relationships::Model>::new()])
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([
            vec![(coach, None)],
            vec![(coachee, None)],
        ])
}

#[tokio::test]
async fn create_in_organization_without_a_coach_commits_the_user_and_role_only() -> Result<(), Error>
{
    let organization_id = Id::new_v4();
    let new_user = user(Id::new_v4());

    let db = mock_user_creation(
        MockDatabase::new(DatabaseBackend::Postgres),
        organization_id,
        new_user.clone(),
    )
    .into_connection();

    let created = create_in_organization(&db, organization_id, new_user.clone(), None).await?;

    assert_eq!(created.id, new_user.id);

    let sql = statements(db);
    assert!(sql.iter().any(|s| s == "COMMIT"), "{sql:?}");
    assert!(!sql.iter().any(|s| s == "ROLLBACK"), "{sql:?}");
    assert!(
        !sql.iter()
            .any(|s| s.contains("INSERT") && s.contains("coaching_relationships")),
        "no coach was requested, so no relationship may be inserted: {sql:?}"
    );

    Ok(())
}

/// The assertion the whole atomic-add change rests on: if the relationship
/// insert fails, nothing commits, so the caller never reaches the invite email.
#[tokio::test]
async fn create_in_organization_rolls_back_when_the_relationship_insert_fails() {
    let organization_id = Id::new_v4();
    let new_user = user(Id::new_v4());
    let coach = user(Id::new_v4());

    let db = mock_relationship_preflight(
        mock_user_creation(
            MockDatabase::new(DatabaseBackend::Postgres),
            organization_id,
            new_user.clone(),
        ),
        organization_id,
        coach.clone(),
        new_user.clone(),
    )
    .append_query_errors([DbErr::Custom("relationship insert failed".to_owned())])
    .into_connection();

    create_in_organization(&db, organization_id, new_user, Some(coach.id))
        .await
        .expect_err("expected the failed relationship insert to fail the whole call");

    let sql = statements(db);
    assert!(
        sql.iter()
            .any(|s| s.contains("INSERT") && s.contains("coaching_relationships")),
        "the failure must be the relationship insert, not an earlier lookup: {sql:?}"
    );
    assert!(
        sql.iter().any(|s| s == "ROLLBACK"),
        "the transaction must roll back: {sql:?}"
    );
    assert!(
        !sql.iter().any(|s| s == "COMMIT"),
        "nothing may commit, or the user would be invited without their coach: {sql:?}"
    );
}

#[tokio::test]
async fn create_in_organization_commits_the_user_and_the_relationship_together() -> Result<(), Error>
{
    let organization_id = Id::new_v4();
    let new_user = user(Id::new_v4());
    let coach = user(Id::new_v4());
    let now = chrono::Utc::now();

    let db = mock_relationship_preflight(
        mock_user_creation(
            MockDatabase::new(DatabaseBackend::Postgres),
            organization_id,
            new_user.clone(),
        ),
        organization_id,
        coach.clone(),
        new_user.clone(),
    )
    .append_query_results([[coaching_relationships::Model {
        id: Id::new_v4(),
        organization_id,
        coach_id: coach.id,
        coachee_id: new_user.id,
        slug: "member-member".to_owned(),
        created_at: now.into(),
        updated_at: now.into(),
    }]])
    .into_connection();

    create_in_organization(&db, organization_id, new_user, Some(coach.id)).await?;

    let sql = statements(db);
    let commits = sql.iter().filter(|s| s.as_str() == "COMMIT").count();
    assert_eq!(commits, 1, "{sql:?}");
    assert!(!sql.iter().any(|s| s == "ROLLBACK"), "{sql:?}");
    assert!(
        sql.iter()
            .any(|s| s.contains("INSERT") && s.contains("coaching_relationships")),
        "{sql:?}"
    );

    Ok(())
}
