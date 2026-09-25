//! Covers the window between the uniqueness pre-check and the insert, where a
//! concurrent request can create the same relationship first.

use super::*;
use entity::{organizations, user_roles, users};
use sea_orm::{DatabaseBackend, DatabaseConnection, MockDatabase};

fn organization(id: Id) -> organizations::Model {
    let now = Utc::now();
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

fn user(id: Id, first_name: &str) -> users::Model {
    let now = Utc::now();
    users::Model {
        id,
        email: format!("{first_name}@test.com").to_lowercase(),
        first_name: first_name.to_owned(),
        last_name: "Tester".to_owned(),
        display_name: None,
        password: None,
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_owned(),
        default_coaching_session_duration_minutes: entity::duration::Duration::default_minutes(),
        roles: vec![],
        invite_status: None,
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn relationship(organization_id: Id, coach_id: Id, coachee_id: Id) -> Model {
    let now = Utc::now();
    Model {
        id: Id::new_v4(),
        organization_id,
        coach_id,
        coachee_id,
        slug: "coach-coachee".to_owned(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn statements(db: DatabaseConnection) -> Vec<String> {
    db.into_transaction_log()
        .into_iter()
        .flat_map(|transaction| {
            transaction
                .statements()
                .iter()
                .map(|statement| statement.to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The organization, both users, then a super-admin probe plus an organization list
/// per user. Mirrors what `create` reads before the uniqueness lookup.
fn mock_up_to_uniqueness_check(
    organization_id: Id,
    coach: &users::Model,
    coachee: &users::Model,
) -> MockDatabase {
    let membership = [organization(organization_id)];
    MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id)]])
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([
            vec![(coach.clone(), None)],
            vec![(coachee.clone(), None)],
        ])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .append_query_results([membership.clone()])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .append_query_results([membership])
}

/// The pre-check sees nothing, the insert conflicts because a concurrent request
/// committed first, and the recovery read returns that winner.
#[tokio::test]
async fn returns_the_winner_when_a_concurrent_insert_wins_the_race() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let coach = user(Id::new_v4(), "Coach");
    let coachee = user(Id::new_v4(), "Coachee");
    let winner = relationship(organization_id, coach.id, coachee.id);

    let db = mock_up_to_uniqueness_check(organization_id, &coach, &coachee)
        // pre-check: nothing yet
        .append_query_results([Vec::<Model>::new()])
        // insert: ON CONFLICT DO NOTHING wrote no row
        .append_query_results([Vec::<Model>::new()])
        // recovery read: the concurrent winner
        .append_query_results([[winner.clone()]])
        .into_connection();

    let result = create(
        &db,
        organization_id,
        relationship(organization_id, coach.id, coachee.id),
    )
    .await?;

    assert_eq!(
        result.id, winner.id,
        "losing the race must yield the winning relationship, not an error"
    );

    let sql = statements(db);
    assert!(
        sql.iter().any(|statement| statement.contains("INSERT")),
        "the race is only reachable by attempting the insert: {sql:?}"
    );
    assert!(
        sql.iter()
            .any(|statement| statement.contains("ON CONFLICT")),
        "the insert must not let the unique index raise, or it aborts the caller's transaction: {sql:?}"
    );

    Ok(())
}
