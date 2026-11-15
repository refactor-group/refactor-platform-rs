//! Sweep behaviour that only shows up with a database and an upstream together.
//!
//! Reachable under mocks because the send path loads people with plain queries. While it
//! used `find_with_related`, nothing past the first user load could be driven from a test.

use std::collections::BTreeMap;
use std::sync::Arc;

use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult, Value};
use service::config::Config;

use super::*;
use crate::{coaching_relationships, coaching_sessions, organizations, users, Id};

fn config_for(server_url: &str) -> Config {
    Config::from_args([
        "test",
        "--resend-api-key=test_key",
        "--session-reminder-email-template-id=reminder_template",
        "--frontend-base-url=https://app.example.com",
        &format!("--resend-base-url={server_url}"),
    ])
}

/// One claimed `(session, recipient)` row as `claim_due_reminders` selects it.
fn claim_row(session_id: Id, user_id: Id, start: chrono::NaiveDateTime) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("coaching_session_id".to_string(), session_id.into()),
        ("user_id".to_string(), user_id.into()),
        ("sent_for_start".to_string(), start.into()),
        ("claim_id".to_string(), Id::new_v4().into()),
    ])
}

fn session(id: Id, relationship_id: Id, date: chrono::NaiveDateTime) -> coaching_sessions::Model {
    coaching_sessions::Model {
        id,
        coaching_relationship_id: relationship_id,
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        collab_document_name: None,
        date,
        duration_minutes: 60,
        title: None,
        meeting_url: None,
        provider: None,
        created_at: chrono::Utc::now().into(),
        updated_at: chrono::Utc::now().into(),
        hydrated_at: None,
        scheduled_by_user_id: Id::nil(),
        meeting_url_created_by_user_id: None,
    }
}

fn relationship(id: Id, coach_id: Id, coachee_id: Id) -> coaching_relationships::Model {
    coaching_relationships::Model {
        id,
        organization_id: Id::new_v4(),
        coach_id,
        coachee_id,
        kind: crate::coaching_relationship_kind::Kind::Coaching,
        slug: "rel".to_string(),
        created_at: chrono::Utc::now().into(),
        updated_at: chrono::Utc::now().into(),
    }
}

fn user(id: Id) -> users::Model {
    user_with_email(id, "coachee@example.com")
}

fn user_with_email(id: Id, email: &str) -> users::Model {
    users::Model {
        id,
        email: email.to_string(),
        first_name: "Test".to_string(),
        last_name: "Coachee".to_string(),
        display_name: None,
        password: Some("x".to_string()),
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        roles: vec![],
        invite_status: None,
        created_at: chrono::Utc::now().into(),
        updated_at: chrono::Utc::now().into(),
        default_coaching_session_duration_minutes: 60,
        working_hours_start: crate::working_hours::default_start(),
        working_hours_end: crate::working_hours::default_end(),
    }
}

fn organization(id: Id) -> organizations::Model {
    organizations::Model {
        id,
        name: "Acme".to_string(),
        logo: None,
        slug: "acme".to_string(),
        archived_at: None,
        archived_by: None,
        created_at: chrono::Utc::now().into(),
        updated_at: chrono::Utc::now().into(),
    }
}

/// A limiter refuses the whole tick, not one recipient, so continuing through the batch
/// buys nothing and spends the budget every other email in the app shares. The tick stops
/// and hands back what it never tried, leaving it due next tick.
///
/// The abort is what the satisfiable second send path proves: without it the loop reaches
/// a second request and the mock's `expect(1)` fails.
#[tokio::test]
async fn rate_limited_send_abandons_the_rest_of_the_tick() {
    let mut server = mockito::Server::new_async().await;
    // Exactly one send: the second reminder must never be attempted.
    let mock = server
        .mock("POST", "/emails")
        .with_status(429)
        .with_header("retry-after", "2")
        .with_body(r#"{"message":"Too many requests"}"#)
        .expect(1)
        .create_async()
        .await;

    let (first, second) = (Id::new_v4(), Id::new_v4());
    let relationship_id = Id::new_v4();
    let coach_id = Id::new_v4();
    let coachee_id = Id::new_v4();
    let start = chrono::Utc::now().naive_utc() + chrono::Duration::hours(2);
    let rel = relationship(relationship_id, coach_id, coachee_id);

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        // The claim, then the batch load of the sessions it claimed.
        .append_query_results([vec![
            claim_row(first, coachee_id, start),
            claim_row(second, coachee_id, start),
        ]])
        .append_query_results([vec![
            session(first, relationship_id, start),
            session(second, relationship_id, start),
        ]])
        // The first reminder's send path: relationship, coach, coachee, organization,
        // then the notify-set membership check.
        .append_query_results([vec![rel.clone()]])
        .append_query_results([vec![user(coach_id)]])
        .append_query_results([vec![user(coachee_id)]])
        .append_query_results([vec![organization(rel.organization_id)]])
        .append_query_results([vec![BTreeMap::from([(
            "user_id".to_string(),
            Value::from(coachee_id),
        )])]])
        // The second reminder's send path, deliberately satisfiable: without the abort
        // the loop would get all the way to a second request, and the mock's expect(1)
        // is what catches it. Left unused when the tick stops as it should.
        .append_query_results([vec![rel.clone()]])
        .append_query_results([vec![user(coach_id)]])
        .append_query_results([vec![user(coachee_id)]])
        .append_query_results([vec![organization(rel.organization_id)]])
        .append_query_results([vec![BTreeMap::from([(
            "user_id".to_string(),
            Value::from(coachee_id),
        )])]])
        // Releasing the failed claim, then the one never attempted.
        .append_exec_results([
            MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            },
            MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            },
        ])
        .into_connection();

    let ctx = Context {
        db: Arc::new(db),
        config: config_for(&server.url()),
    };
    let sweep = Sweep::from_config(&ctx.config).expect("sweep is configured");

    let outcome = sweep
        .run(&ctx)
        .await
        .expect("a rate limit is not a tick error");

    assert_eq!(
        (outcome.processed, outcome.attempted),
        (0, 2),
        "the tick found two and sent none, which must not read as idle"
    );
    // The assertion that matters: a second send would have been a second request.
    mock.assert_async().await;

    let Context { db, .. } = ctx;
    let statements = Arc::try_unwrap(db)
        .expect("the sweep holds no reference once it has returned")
        .into_transaction_log();
    let deletes = statements
        .iter()
        .filter(|s| format!("{s:?}").contains("DELETE FROM"))
        .count();
    assert_eq!(
        deletes, 2,
        "both the failed claim and the unattempted one must be handed back, or the \
         unattempted reminder is stranded until its session starts: {statements:?}"
    );
}

/// The limiter hitting mid-batch, which the two-item case cannot show: a reminder already
/// delivered must keep its claim. Releasing from `index` rather than `index + 1` would
/// hand back the failed one twice, and a wider slip would hand back a sent one, which
/// sends it again next tick.
#[tokio::test]
async fn a_send_that_already_succeeded_keeps_its_claim_when_the_tick_aborts() {
    let mut server = mockito::Server::new_async().await;
    let (first, second, third) = (Id::new_v4(), Id::new_v4(), Id::new_v4());

    // Matched by session id so the order cannot silently drift: the first is delivered,
    // the second meets the limiter, the third is never attempted.
    let delivered = server
        .mock("POST", "/emails")
        .match_body(mockito::Matcher::Regex(first.to_string()))
        .with_status(200)
        .with_body(r#"{"id":"sent"}"#)
        .expect(1)
        .create_async()
        .await;
    let limited = server
        .mock("POST", "/emails")
        .match_body(mockito::Matcher::Regex(second.to_string()))
        .with_status(429)
        .with_body(r#"{"message":"Too many requests"}"#)
        .expect(1)
        .create_async()
        .await;
    let never_attempted = server
        .mock("POST", "/emails")
        .match_body(mockito::Matcher::Regex(third.to_string()))
        .with_status(200)
        .expect(0)
        .create_async()
        .await;

    let relationship_id = Id::new_v4();
    let coach_id = Id::new_v4();
    let coachee_id = Id::new_v4();
    let start = chrono::Utc::now().naive_utc() + chrono::Duration::hours(2);
    let rel = relationship(relationship_id, coach_id, coachee_id);

    let mut db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![
            claim_row(first, coachee_id, start),
            claim_row(second, coachee_id, start),
            claim_row(third, coachee_id, start),
        ]])
        .append_query_results([vec![
            session(first, relationship_id, start),
            session(second, relationship_id, start),
            session(third, relationship_id, start),
        ]]);

    // Every send path is satisfiable, including the third: the abort is what stops it,
    // not an exhausted mock.
    for _ in 0..3 {
        db = db
            .append_query_results([vec![rel.clone()]])
            .append_query_results([vec![user(coach_id)]])
            .append_query_results([vec![user(coachee_id)]])
            .append_query_results([vec![organization(rel.organization_id)]])
            .append_query_results([vec![BTreeMap::from([(
                "user_id".to_string(),
                Value::from(coachee_id),
            )])]]);
    }

    let db = db
        .append_exec_results([
            MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            },
            MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            },
        ])
        .into_connection();

    let ctx = Context {
        db: Arc::new(db),
        config: config_for(&server.url()),
    };
    let sweep = Sweep::from_config(&ctx.config).expect("sweep is configured");

    let outcome = sweep
        .run(&ctx)
        .await
        .expect("a rate limit is not a tick error");

    assert_eq!(
        (outcome.processed, outcome.attempted),
        (1, 3),
        "one delivered before the limiter, three found: a tick that aborts must still \
         report what it managed"
    );
    delivered.assert_async().await;
    limited.assert_async().await;
    never_attempted.assert_async().await;

    let Context { db, .. } = ctx;
    let statements = Arc::try_unwrap(db)
        .expect("the sweep holds no reference once it has returned")
        .into_transaction_log();
    let deleted: Vec<String> = statements
        .iter()
        .map(|s| format!("{s:?}"))
        .filter(|s| s.contains("DELETE FROM"))
        .collect();

    assert_eq!(
        deleted.len(),
        2,
        "the limited claim and the unattempted one, and nothing else: {deleted:?}"
    );
    assert!(
        !deleted.iter().any(|s| s.contains(&first.to_string())),
        "the delivered reminder's claim must survive, or it is sent again next tick: \
         {deleted:?}"
    );
    assert!(
        deleted.iter().any(|s| s.contains(&second.to_string()))
            && deleted.iter().any(|s| s.contains(&third.to_string())),
        "both the limited and the unattempted claims must be handed back: {deleted:?}"
    );
}

/// An outage is as much a reason to stop as a limiter: 200 sends against a dead upstream
/// pays a timeout each, serially, which is the hammering this exists to avoid. The abort
/// branch has to cover every failure that is about the upstream, not just the one that
/// prompted it.
#[tokio::test]
async fn a_failing_upstream_abandons_the_tick_the_same_as_a_limiter() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/emails")
        .with_status(503)
        .with_body(r#"{"message":"upstream down"}"#)
        .expect(1)
        .create_async()
        .await;

    let (first, second) = (Id::new_v4(), Id::new_v4());
    let relationship_id = Id::new_v4();
    let coach_id = Id::new_v4();
    let coachee_id = Id::new_v4();
    let start = chrono::Utc::now().naive_utc() + chrono::Duration::hours(2);
    let rel = relationship(relationship_id, coach_id, coachee_id);

    let mut db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![
            claim_row(first, coachee_id, start),
            claim_row(second, coachee_id, start),
        ]])
        .append_query_results([vec![
            session(first, relationship_id, start),
            session(second, relationship_id, start),
        ]]);

    for _ in 0..2 {
        db = db
            .append_query_results([vec![rel.clone()]])
            .append_query_results([vec![user(coach_id)]])
            .append_query_results([vec![user(coachee_id)]])
            .append_query_results([vec![organization(rel.organization_id)]])
            .append_query_results([vec![BTreeMap::from([(
                "user_id".to_string(),
                Value::from(coachee_id),
            )])]]);
    }

    let db = db
        .append_exec_results([
            MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            },
            MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            },
        ])
        .into_connection();

    let ctx = Context {
        db: Arc::new(db),
        config: config_for(&server.url()),
    };
    let sweep = Sweep::from_config(&ctx.config).expect("sweep is configured");
    let outcome = sweep
        .run(&ctx)
        .await
        .expect("an outage is not a tick error");

    assert_eq!((outcome.processed, outcome.attempted), (0, 2));
    mock.assert_async().await;
}

/// One coachee with an unusable address in the database must cost that reminder and no
/// more. The payload is refused before it is sent, which is a fact about that recipient,
/// not about Resend, so the tick carries on to everyone else.
#[tokio::test]
async fn a_recipient_with_a_bad_address_does_not_abandon_the_tick() {
    let mut server = mockito::Server::new_async().await;
    // The healthy recipient's send still has to happen.
    let mock = server
        .mock("POST", "/emails")
        .with_status(200)
        .with_body(r#"{"id":"sent"}"#)
        .expect(1)
        .create_async()
        .await;

    let (first, second) = (Id::new_v4(), Id::new_v4());
    let relationship_id = Id::new_v4();
    let coach_id = Id::new_v4();
    let bad_coachee = Id::new_v4();
    let good_coachee = Id::new_v4();
    let start = chrono::Utc::now().naive_utc() + chrono::Duration::hours(2);
    let bad_rel = relationship(relationship_id, coach_id, bad_coachee);
    let good_rel = relationship(relationship_id, coach_id, good_coachee);

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![
            claim_row(first, bad_coachee, start),
            claim_row(second, good_coachee, start),
        ]])
        .append_query_results([vec![
            session(first, relationship_id, start),
            session(second, relationship_id, start),
        ]])
        // The unusable address fails in the builder, before any request is made.
        .append_query_results([vec![bad_rel.clone()]])
        .append_query_results([vec![user(coach_id)]])
        .append_query_results([vec![user_with_email(bad_coachee, "not-an-email")]])
        .append_query_results([vec![organization(bad_rel.organization_id)]])
        .append_query_results([vec![BTreeMap::from([(
            "user_id".to_string(),
            Value::from(bad_coachee),
        )])]])
        // The healthy one, which must still be reached.
        .append_query_results([vec![good_rel.clone()]])
        .append_query_results([vec![user(coach_id)]])
        .append_query_results([vec![user(good_coachee)]])
        .append_query_results([vec![organization(good_rel.organization_id)]])
        .append_query_results([vec![BTreeMap::from([(
            "user_id".to_string(),
            Value::from(good_coachee),
        )])]])
        // Only the refused reminder's claim comes back.
        .append_exec_results([MockExecResult {
            last_insert_id: 0,
            rows_affected: 1,
        }])
        .into_connection();

    let ctx = Context {
        db: Arc::new(db),
        config: config_for(&server.url()),
    };
    let sweep = Sweep::from_config(&ctx.config).expect("sweep is configured");
    let outcome = sweep
        .run(&ctx)
        .await
        .expect("a bad address is not a tick error");

    assert_eq!(
        (outcome.processed, outcome.attempted),
        (1, 2),
        "the healthy recipient must still be mailed"
    );
    mock.assert_async().await;

    let Context { db, .. } = ctx;
    let deletes = Arc::try_unwrap(db)
        .expect("the sweep holds no reference once it has returned")
        .into_transaction_log()
        .iter()
        .filter(|s| format!("{s:?}").contains("DELETE FROM"))
        .count();
    assert_eq!(
        deletes, 1,
        "only the refused reminder is handed back: a second release means the tick was \
         abandoned over one unusable address"
    );
}

/// A session moved after the claim may hold a change participants were not told yet, so
/// the reminder waits rather than announce it; releasing lets a later tick decide afresh.
#[tokio::test]
async fn a_session_moved_after_its_claim_is_released_unsent() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/emails")
        .with_status(200)
        .with_body(r#"{"id":"sent"}"#)
        .expect(0)
        .create_async()
        .await;

    let session_id = Id::new_v4();
    let coachee = Id::new_v4();
    let claimed = chrono::Utc::now().naive_utc() + chrono::Duration::hours(2);
    let db = Arc::new(
        MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([vec![claim_row(session_id, coachee, claimed)]])
            .append_query_results([vec![session(
                session_id,
                Id::new_v4(),
                claimed + chrono::Duration::hours(1),
            )]])
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection(),
    );
    let ctx = Context {
        db: db.clone(),
        config: config_for(&server.url()),
    };

    let outcome = Sweep::from_config(&ctx.config)
        .expect("sweep is configured")
        .run(&ctx)
        .await
        .expect("a moved session is not a tick error");

    mock.assert_async().await;
    assert_eq!((outcome.processed, outcome.attempted), (1, 1));
    drop(ctx);
    let log = Arc::try_unwrap(db)
        .expect("the context has been dropped")
        .into_transaction_log();
    assert!(
        log.iter()
            .map(|txn| format!("{txn:?}"))
            .any(|sql| sql.contains("DELETE") && sql.contains("coaching_session_reminders")),
        "the claim must be released: {log:?}"
    );
}

fn named(id: Id, first_name: &str, email: &str) -> users::Model {
    users::Model {
        first_name: first_name.to_string(),
        last_name: "Smith".to_string(),
        ..user_with_email(id, email)
    }
}

/// One send path: the relationship, its two people, the organization, then the members.
fn send_path(
    db: MockDatabase,
    rel: &coaching_relationships::Model,
    coach: &users::Model,
    coachee: &users::Model,
) -> MockDatabase {
    db.append_query_results([vec![rel.clone()]])
        .append_query_results([vec![coach.clone()]])
        .append_query_results([vec![coachee.clone()]])
        .append_query_results([vec![organization(rel.organization_id)]])
        .append_query_results([vec![
            BTreeMap::from([("user_id".to_string(), Value::from(rel.coach_id))]),
            BTreeMap::from([("user_id".to_string(), Value::from(rel.coachee_id))]),
        ]])
}

/// A Resend mock expecting `times` reminders to `recipient` that name `other`.
async fn reminder_to(
    server: &mut mockito::ServerGuard,
    recipient: &users::Model,
    other: &users::Model,
    times: usize,
) -> mockito::Mock {
    server
        .mock("POST", "/emails")
        .match_body(mockito::Matcher::PartialJson(serde_json::json!({
            "to": [format!("\"{} Smith\" <{}>", recipient.first_name, recipient.email)],
            "template": { "variables": {
                "first_name": recipient.first_name,
                "coach_first_name": other.first_name,
                "coach_full_name": format!("{} Smith", other.first_name),
            }},
        })))
        .with_status(200)
        .with_body(r#"{"id":"sent"}"#)
        .expect(times)
        .create_async()
        .await
}

/// JOB-06 and MAIL-15: one sweep reminds both colleagues, each naming the other, and a
/// second sweep, whose claim finds nothing current, sends nothing more.
#[tokio::test]
async fn both_colleagues_are_reminded_once_each_naming_the_other() {
    let mut server = mockito::Server::new_async().await;
    let (ann_id, bea_id) = (Id::new_v4(), Id::new_v4());
    let ann = named(ann_id, "Ann", "ann@example.com");
    let bea = named(bea_id, "Bea", "bea@example.com");
    let to_ann = reminder_to(&mut server, &ann, &bea, 1).await;
    let to_bea = reminder_to(&mut server, &bea, &ann, 1).await;

    let (session_id, relationship_id) = (Id::new_v4(), Id::new_v4());
    let start = chrono::Utc::now().naive_utc() + chrono::Duration::hours(2);
    let rel = coaching_relationships::Model {
        kind: crate::coaching_relationship_kind::Kind::Colleague,
        ..relationship(relationship_id, ann_id, bea_id)
    };

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![
            claim_row(session_id, bea_id, start),
            claim_row(session_id, ann_id, start),
        ]])
        .append_query_results([vec![session(session_id, relationship_id, start)]]);
    let db = send_path(send_path(db, &rel, &ann, &bea), &rel, &ann, &bea)
        // The second sweep's claim: both rows already hold this start.
        .append_query_results([Vec::<BTreeMap<String, Value>>::new()])
        .into_connection();

    let ctx = Context {
        db: Arc::new(db),
        config: config_for(&server.url()),
    };
    let sweep = Sweep::from_config(&ctx.config).expect("sweep is configured");

    let first = sweep.run(&ctx).await.expect("the first sweep runs");
    let second = sweep.run(&ctx).await.expect("the second sweep runs");

    assert_eq!((first.processed, first.attempted), (2, 2));
    assert_eq!((second.processed, second.attempted), (0, 0));
    to_ann.assert_async().await;
    to_bea.assert_async().await;
}

/// JOB-06: a coaching relationship never reminds its coach, even handed a claim for them.
#[tokio::test]
async fn a_coach_is_not_reminded_of_a_coaching_session() {
    let mut server = mockito::Server::new_async().await;
    let (coach_id, coachee_id) = (Id::new_v4(), Id::new_v4());
    let coach = named(coach_id, "Cal", "cal@example.com");
    let coachee = named(coachee_id, "Dee", "dee@example.com");
    let to_coach = reminder_to(&mut server, &coach, &coachee, 0).await;
    let to_coachee = reminder_to(&mut server, &coachee, &coach, 1).await;

    let (session_id, relationship_id) = (Id::new_v4(), Id::new_v4());
    let start = chrono::Utc::now().naive_utc() + chrono::Duration::hours(2);
    let rel = relationship(relationship_id, coach_id, coachee_id);

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![
            claim_row(session_id, coachee_id, start),
            claim_row(session_id, coach_id, start),
        ]])
        .append_query_results([vec![session(session_id, relationship_id, start)]]);
    // The coach's attempt stops at the relationship, before any other load.
    let db = send_path(db, &rel, &coach, &coachee)
        .append_query_results([vec![rel.clone()]])
        .into_connection();

    let ctx = Context {
        db: Arc::new(db),
        config: config_for(&server.url()),
    };
    let outcome = Sweep::from_config(&ctx.config)
        .expect("sweep is configured")
        .run(&ctx)
        .await
        .expect("the sweep runs");

    assert_eq!((outcome.processed, outcome.attempted), (2, 2));
    to_coach.assert_async().await;
    to_coachee.assert_async().await;
}
