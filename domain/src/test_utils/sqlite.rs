//! In-memory SQLite for tests that must see what SQL actually selects or writes.
//!
//! Prefer the mock database. Reach for this only when correctness depends on the query
//! itself; see `docs/test-plans/sqlite_integration_testing.md` for when and why.

use std::future::Future;
use std::time::Duration;

use chrono::Utc;
use entity::{
    actions, actions_users, agreements, coaching_relationships, coaching_session_images,
    coaching_session_reminders, coaching_session_series, coaching_session_topics,
    coaching_session_views, coaching_sessions, coaching_sessions_goals, cost_pricing_config, goals,
    magic_link_tokens, meeting_recording, notes, oauth_connections, organizations,
    password_reset_attempts, platform_cost_metrics, transcript_segment, transcription,
    user_lookup_attempts, user_role_changes, user_roles, users, Id,
};
use sea_orm::{
    ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Set,
};

/// Ceiling on one SQLite test. Per-acquire timeouts still add up across many rows.
const TEST_TIME_LIMIT: Duration = Duration::from_secs(5 * 60);

/// Runs a SQLite test body, failing it once it passes `TEST_TIME_LIMIT`.
pub(crate) async fn within_time_limit<F: Future>(test: F) -> F::Output {
    tokio::time::timeout(TEST_TIME_LIMIT, test)
        .await
        .unwrap_or_else(|_| {
            panic!(
                "SQLite test ran past its {TEST_TIME_LIMIT:?} limit; a query likely escaped its transaction"
            )
        })
}

/// A fresh in-memory database holding every entity's table, with foreign keys enforced.
pub(crate) async fn database() -> DatabaseConnection {
    let mut options = ConnectOptions::new("sqlite::memory:");
    // Every in-memory SQLite connection is its own database, so there must be exactly one.
    // A query that escapes an open transaction then waits for a second connection that never
    // frees up. The short timeout turns that wait into a prompt failure; every legitimate
    // acquire here is sequential and gets the connection back within microseconds.
    options
        .max_connections(1)
        .min_connections(1)
        .acquire_timeout(Duration::from_secs(1))
        .sqlx_logging(false);
    let db = Database::connect(options)
        .await
        .expect("in-memory SQLite opens");

    // Entities name the `refactor_platform` schema, which SQLite resolves as an attached database.
    for statement in [
        "ATTACH DATABASE ':memory:' AS refactor_platform",
        "PRAGMA foreign_keys = ON",
    ] {
        db.execute_unprepared(statement)
            .await
            .expect("the database is prepared");
    }

    // Every entity except `coaches` and `coachees`, which are narrower views of `users`.
    db.get_schema_builder()
        .register(actions::Entity)
        .register(actions_users::Entity)
        .register(agreements::Entity)
        .register(coaching_relationships::Entity)
        .register(coaching_session_images::Entity)
        .register(coaching_session_reminders::Entity)
        .register(coaching_session_series::Entity)
        .register(coaching_session_topics::Entity)
        .register(coaching_session_views::Entity)
        .register(coaching_sessions::Entity)
        .register(coaching_sessions_goals::Entity)
        .register(cost_pricing_config::Entity)
        .register(goals::Entity)
        .register(magic_link_tokens::Entity)
        .register(meeting_recording::Entity)
        .register(notes::Entity)
        .register(oauth_connections::Entity)
        .register(organizations::Entity)
        .register(password_reset_attempts::Entity)
        .register(platform_cost_metrics::Entity)
        .register(transcript_segment::Entity)
        .register(transcription::Entity)
        .register(user_lookup_attempts::Entity)
        .register(user_role_changes::Entity)
        .register(user_roles::Entity)
        .register(users::Entity)
        .sync(&db)
        .await
        .expect("the schema is synced");

    db
}

/// Inserts a user with every column set and returns its id.
pub(crate) async fn seed_user(db: &DatabaseConnection) -> Id {
    let id = Id::new_v4();
    let now = Utc::now();
    users::ActiveModel {
        id: Set(id),
        email: Set(format!("{id}@example.com")),
        first_name: Set("Test".to_string()),
        last_name: Set("User".to_string()),
        display_name: Set(None),
        password: Set(None),
        github_username: Set(None),
        github_profile_url: Set(None),
        timezone: Set("UTC".to_string()),
        default_coaching_session_duration_minutes: Set(60),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the user is seeded")
    .id
}

/// Inserts an organization with every column set and returns its id.
pub(crate) async fn seed_organization(db: &DatabaseConnection) -> Id {
    let id = Id::new_v4();
    let now = Utc::now();
    organizations::ActiveModel {
        id: Set(id),
        name: Set(format!("Organization {id}")),
        logo: Set(None),
        slug: Set(format!("organization-{id}")),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        archived_at: Set(None),
        archived_by: Set(None),
    }
    .insert(db)
    .await
    .expect("the organization is seeded")
    .id
}

/// Inserts a coaching session and its parents; returns the session and its one user.
pub(crate) async fn seed_coaching_session(db: &DatabaseConnection) -> (Id, Id) {
    let user_id = seed_user(db).await;
    let organization_id = seed_organization(db).await;
    let now = Utc::now();

    let relationship_id = Id::new_v4();
    coaching_relationships::ActiveModel {
        id: Set(relationship_id),
        organization_id: Set(organization_id),
        coach_id: Set(user_id),
        coachee_id: Set(user_id),
        slug: Set(format!("relationship-{relationship_id}")),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the coaching relationship is seeded");

    let session = coaching_sessions::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_relationship_id: Set(relationship_id),
        coaching_session_series_id: Set(None),
        ical_sequence: Set(0),
        ical_recurrence_id: Set(None),
        collab_document_name: Set(None),
        date: Set(now.naive_utc()),
        duration_minutes: Set(60),
        title: Set(None),
        meeting_url: Set(None),
        provider: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        hydrated_at: Set(None),
        notice_given_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the coaching session is seeded");

    (session.id, user_id)
}
