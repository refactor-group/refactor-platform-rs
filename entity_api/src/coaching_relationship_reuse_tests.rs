use super::*;
use entity::{organizations, user_roles, users};
use sea_orm::{DatabaseBackend, DatabaseConnection, MockDatabase, MockExecResult, Value};

fn organization(id: Id, archived: bool) -> organizations::Model {
    let now = Utc::now();
    organizations::Model {
        id,
        name: "Test Org".to_owned(),
        logo: None,
        slug: "test-org".to_owned(),
        created_at: now.into(),
        updated_at: now.into(),
        archived_at: archived.then(|| now.into()),
        archived_by: archived.then(Id::new_v4),
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
        working_hours_start: crate::working_hours::default_start(),
        working_hours_end: crate::working_hours::default_end(),
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
        kind: Kind::Coaching,
        slug: "coach-coachee".to_owned(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

/// Every statement the mock saw, paired with the values bound to it.
///
/// `to_string()` on a SELECT keeps the placeholders, so the only way to prove which id
/// went into which column is to read the bindings.
fn logged(db: DatabaseConnection) -> Vec<(String, Vec<String>)> {
    db.into_transaction_log()
        .into_iter()
        .flat_map(|transaction| transaction.statements().to_vec())
        .map(|statement| {
            let values = statement
                .values
                .clone()
                .map(|values| values.0.iter().map(|value| format!("{value:?}")).collect())
                .unwrap_or_default();
            (statement.to_string(), values)
        })
        .collect()
}

/// Every statement the mock saw, flattened across transactions.
fn statements(db: DatabaseConnection) -> Vec<String> {
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

/// Mocks every query `create` runs before the uniqueness lookup: the organization,
/// both users, then a super-admin probe plus an organization list per user, then the
/// pair's advisory lock.
fn mock_up_to_uniqueness_check(
    organization_id: Id,
    coach: &users::Model,
    coachee: &users::Model,
) -> MockDatabase {
    let membership = [organization(organization_id, false)];
    MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id, false)]])
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([
            vec![(coach.clone(), None)],
            vec![(coachee.clone(), None)],
        ])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .append_query_results([membership.clone()])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .append_query_results([membership])
        .append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }])
}

/// The uniqueness lookup finds the requested pair, so no insert result is queued.
fn mock_reuse(
    organization_id: Id,
    coach: &users::Model,
    coachee: &users::Model,
    existing: &Model,
) -> DatabaseConnection {
    mock_up_to_uniqueness_check(organization_id, coach, coachee)
        .append_query_results([[existing.clone()]])
        .into_connection()
}

#[tokio::test]
async fn reuses_an_existing_relationship_instead_of_erroring() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let coach = user(Id::new_v4(), "Coach");
    let coachee = user(Id::new_v4(), "Coachee");
    let existing = relationship(organization_id, coach.id, coachee.id);

    let db = mock_reuse(organization_id, &coach, &coachee, &existing);

    let reused = create(
        &db,
        organization_id,
        Draft {
            coach_id: coach.id,
            coachee_id: coachee.id,
            kind: Kind::Coaching,
        },
    )
    .await?
    .relationship;

    assert_eq!(reused.id, existing.id);
    assert_eq!(reused.coach_id, coach.id);
    assert_eq!(reused.coachee_id, coachee.id);

    Ok(())
}

#[tokio::test]
async fn reuse_does_not_issue_an_insert() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let coach = user(Id::new_v4(), "Coach");
    let coachee = user(Id::new_v4(), "Coachee");
    let existing = relationship(organization_id, coach.id, coachee.id);

    let db = mock_reuse(organization_id, &coach, &coachee, &existing);

    create(
        &db,
        organization_id,
        Draft {
            coach_id: coach.id,
            coachee_id: coachee.id,
            kind: Kind::Coaching,
        },
    )
    .await?;

    let sql = statements(db);
    assert!(
        !sql.iter().any(|statement| statement.contains("INSERT")),
        "reuse must return the surviving row, not write a new one: {sql:?}"
    );

    Ok(())
}

/// `(coach=A, coachee=B)` and `(coach=B, coachee=A)` are different coaching relationships.
#[tokio::test]
async fn does_not_reuse_a_reversed_pair() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let coach = user(Id::new_v4(), "Coach");
    let coachee = user(Id::new_v4(), "Coachee");
    let reversed = relationship(organization_id, coachee.id, coach.id);
    let inserted = relationship(organization_id, coach.id, coachee.id);

    // The lookup reads the pair in both directions, so it finds the reversed row.
    let db = mock_up_to_uniqueness_check(organization_id, &coach, &coachee)
        .append_query_results([[reversed.clone()]])
        .append_query_results([[inserted.clone()]])
        .into_connection();

    let created = create(
        &db,
        organization_id,
        Draft {
            coach_id: coach.id,
            coachee_id: coachee.id,
            kind: Kind::Coaching,
        },
    )
    .await?
    .relationship;

    assert_ne!(created.id, reversed.id);
    assert_eq!(created.id, inserted.id);

    // Organization, coach, coachee, in that order. Swap the last two and these fail,
    // which is what keeps the reversed pair a different relationship.
    let logged = logged(db);
    let (_, bindings) = logged
        .iter()
        .find(|(sql, _)| sql.contains("\"coach_id\" = "))
        .expect("the uniqueness lookup must filter on the pair");

    assert!(
        bindings[1].contains(&coach.id.to_string()),
        "coach must be matched as the coach, got {bindings:?}"
    );
    assert!(
        bindings[2].contains(&coachee.id.to_string()),
        "coachee must be matched as the coachee, got {bindings:?}"
    );
    assert!(
        logged.iter().any(|(sql, _)| sql.contains("INSERT")),
        "the reversed pair is not a match, so the requested one must be created"
    );

    Ok(())
}

#[tokio::test]
async fn rejects_a_self_coaching_relationship_before_any_query() {
    let organization_id = Id::new_v4();
    let user_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

    let error = create(
        &db,
        organization_id,
        Draft {
            coach_id: user_id,
            coachee_id: user_id,
            kind: Kind::Coaching,
        },
    )
    .await
    .expect_err("expected self-coaching rejection");

    assert!(matches!(
        error.error_kind,
        EntityApiErrorKind::ValidationError { .. }
    ));
    assert!(
        statements(db).is_empty(),
        "self-coaching must be rejected before any statement runs"
    );
}

/// Reuse must not become a way past the archived-organization write freeze.
#[tokio::test]
async fn rejects_an_archived_organization_before_the_uniqueness_check() {
    let organization_id = Id::new_v4();
    let coach_id = Id::new_v4();
    let coachee_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id, true)]])
        .into_connection();

    let error = create(
        &db,
        organization_id,
        Draft {
            coach_id,
            coachee_id,
            kind: Kind::Coaching,
        },
    )
    .await
    .expect_err("expected archived-org rejection");

    assert!(matches!(
        error.error_kind,
        EntityApiErrorKind::OrganizationArchived
    ));
    let sql = statements(db);
    assert!(
        !sql.iter().any(|statement| statement.contains("INSERT")),
        "an archived organization must not be written to: {sql:?}"
    );
}

/// The pair's advisory lock is taken before the pair is looked up, so the lookup and the
/// insert see no concurrent create for the pair.
#[tokio::test]
async fn takes_the_pairs_advisory_lock_before_the_pair_lookup() -> Result<(), Error> {
    let organization_id = Id::new_v4();
    let coach = user(Id::new_v4(), "Coach");
    let coachee = user(Id::new_v4(), "Coachee");
    let existing = relationship(organization_id, coach.id, coachee.id);
    let db = mock_reuse(organization_id, &coach, &coachee, &existing);

    create(
        &db,
        organization_id,
        Draft {
            coach_id: coach.id,
            coachee_id: coachee.id,
            kind: Kind::Coaching,
        },
    )
    .await?;

    let logged = logged(db);
    let lock = logged
        .iter()
        .position(|(sql, _)| sql.contains("pg_advisory_xact_lock"))
        .expect("the pair's advisory lock is taken");
    let lookup = logged
        .iter()
        .position(|(sql, _)| sql.contains("\"coach_id\" = "))
        .expect("the pair is looked up");
    assert!(lock < lookup, "{logged:?}");
    assert_eq!(
        logged[lock].1,
        [
            format!("{:?}", Value::from(PAIR_LOCK_NAMESPACE)),
            format!(
                "{:?}",
                Value::from(pair_lock_key(organization_id, coach.id, coachee.id))
            ),
        ],
        "{logged:?}"
    );

    Ok(())
}

#[tokio::test]
async fn a_colleague_outside_the_organization_is_refused_without_naming_a_coach() {
    let organization_id = Id::new_v4();
    let member = user(Id::new_v4(), "Member");
    let outsider = user(Id::new_v4(), "Outsider");

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([[organization(organization_id, false)]])
        .append_query_results::<(users::Model, Option<user_roles::Model>), _, _>([
            vec![(member.clone(), None)],
            vec![(outsider.clone(), None)],
        ])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .append_query_results([[organization(organization_id, false)]])
        .append_query_results([Vec::<user_roles::Model>::new()])
        .append_query_results([Vec::<organizations::Model>::new()])
        .into_connection();

    let error = create(
        &db,
        organization_id,
        Draft {
            coach_id: member.id,
            coachee_id: outsider.id,
            kind: Kind::Colleague,
        },
    )
    .await
    .expect_err("a non-member must be refused");

    assert_eq!(
        error.error_kind,
        EntityApiErrorKind::ValidationError {
            message: "Both people must belong to the specified organization.".into(),
            details: None,
        }
    );
}
