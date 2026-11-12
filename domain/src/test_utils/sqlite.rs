//! In-memory SQLite for tests that must see what SQL actually selects or writes.
//!
//! Prefer the mock database. Reach for this only when correctness depends on the query
//! itself; see `docs/test-plans/sqlite_integration_testing.md` for when and why.

use std::future::Future;
use std::time::Duration;

use chrono::Utc;
use entity::coaching_relationships::Visibility;
use entity::meeting_recording::MeetingRecordingStatus;
use entity::roles::Role;
use entity::status::Status;
use entity::transcription::TranscriptionStatus;
use entity::{
    actions, actions_users, agreements, coaching_relationships, coaching_session_images,
    coaching_session_pending_reschedules, coaching_session_reminders, coaching_session_series,
    coaching_session_topics, coaching_session_views, coaching_sessions, coaching_sessions_goals,
    cost_pricing_config, goals, magic_link_tokens, meeting_recording, notes, oauth_connections,
    organizations, password_reset_attempts, platform_cost_metrics, transcript_analysis,
    transcript_participant, transcript_segment, transcript_suggestion, transcription,
    user_lookup_attempts, user_role_changes, user_roles, users, Id,
};
#[cfg(all(test, feature = "mock"))]
use sea_orm::DbErr;
use sea_orm::{
    ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend,
    EntityTrait, QueryResult, Set, Statement,
};

pub mod access_world;

pub use access_world::{seed_access_world, AccessWorld, Caller};

/// Ceiling on one SQLite test. Per-acquire timeouts still add up across many rows.
const TEST_TIME_LIMIT: Duration = Duration::from_secs(5 * 60);

/// Runs a SQLite test body, failing it once it passes `TEST_TIME_LIMIT`.
pub async fn within_time_limit<F: Future>(test: F) -> F::Output {
    tokio::time::timeout(TEST_TIME_LIMIT, test)
        .await
        .unwrap_or_else(|_| {
            panic!(
                "SQLite test ran past its {TEST_TIME_LIMIT:?} limit; a query likely escaped its transaction"
            )
        })
}

/// A fresh in-memory database holding every entity's table, with foreign keys enforced.
pub async fn database() -> DatabaseConnection {
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
        .register(coaching_session_pending_reschedules::Entity)
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
        .register(transcript_analysis::Entity)
        .register(transcript_participant::Entity)
        .register(transcript_segment::Entity)
        .register(transcript_suggestion::Entity)
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

/// SQLite stand-in for a Postgres timestamp default, in the format SeaORM writes.
const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// Column defaults the migrations declare that inserts through the app rely on:
/// (table, or `None` for every table; column; SQLite default expression).
const MIGRATION_DEFAULTS: &[(Option<&str>, &str, &str)] = &[
    (None, "id", "randomblob(16)"),
    (Some("coaching_session_series"), "ical_sequence", "0"),
    (Some("coaching_sessions"), "ical_sequence", "0"),
    (Some("coaching_sessions"), "notice_given_at", NOW),
    (Some("coaching_session_topics"), "status", "'open'"),
    (Some("users"), "timezone", "'UTC'"),
    (
        Some("users"),
        "default_coaching_session_duration_minutes",
        "60",
    ),
    (Some("users"), "working_hours_start", "'08:00:00'"),
    (Some("users"), "working_hours_end", "'17:30:00'"),
    (Some("user_role_changes"), "changed_at", NOW),
];

/// Unique indexes only the migrations create; `ON CONFLICT` targets need them.
const MIGRATION_INDEXES: &[&str] = &[
    "CREATE UNIQUE INDEX refactor_platform.coaching_relationships_coach_coachee_org \
     ON coaching_relationships (coach_id, coachee_id, organization_id)",
    "CREATE UNIQUE INDEX refactor_platform.user_roles_user_org_unique \
     ON user_roles (user_id, organization_id) WHERE organization_id IS NOT NULL",
    "CREATE UNIQUE INDEX refactor_platform.user_roles_user_global_role_unique \
     ON user_roles (user_id, role) WHERE organization_id IS NULL",
    "CREATE UNIQUE INDEX refactor_platform.actions_users_action_user_unique \
     ON actions_users (action_id, user_id)",
    "CREATE UNIQUE INDEX refactor_platform.coaching_sessions_goals_session_goal_unique \
     ON coaching_sessions_goals (coaching_session_id, goal_id)",
];

/// Rebuilds every table with the column defaults the migrations declare
/// (`MIGRATION_DEFAULTS`) and adds their unique indexes (`MIGRATION_INDEXES`); schema sync
/// leaves both out, and the app's own inserts rely on them. Call before seeding: the
/// tables come back empty.
pub async fn mirror_migration_defaults(db: &DatabaseConnection) {
    let schema = db
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT type, name, sql FROM refactor_platform.sqlite_master WHERE sql IS NOT NULL",
        ))
        .await
        .expect("the schema reads");
    let column = |row: &QueryResult, name: &str| -> String {
        row.try_get("", name)
            .expect("the schema row has the column")
    };
    let of_type = |kind: &'static str| {
        schema
            .iter()
            .filter(move |row| column(row, "type") == kind)
            .map(|row| (column(row, "name"), column(row, "sql")))
    };
    let tables: Vec<(String, String)> = of_type("table").collect();
    let indexes: Vec<(String, String)> = of_type("index").collect();

    db.execute_unprepared("PRAGMA foreign_keys = OFF")
        .await
        .expect("foreign keys pause");
    for (name, sql) in &tables {
        let rebuilt = MIGRATION_DEFAULTS
            .iter()
            .filter(|(table, _, _)| table.is_none_or(|table| table == name))
            .fold(
                qualify(sql, "CREATE TABLE "),
                |sql, (_, column, default)| with_default(&sql, column, default),
            );
        for statement in [format!("DROP TABLE refactor_platform.\"{name}\""), rebuilt] {
            db.execute_unprepared(&statement)
                .await
                .expect("the table is rebuilt");
        }
    }
    let synced_indexes = indexes
        .iter()
        .map(|(_, sql)| qualify(&qualify(sql, "CREATE INDEX "), "CREATE UNIQUE INDEX "));
    for sql in synced_indexes.chain(MIGRATION_INDEXES.iter().map(|sql| sql.to_string())) {
        db.execute_unprepared(&sql)
            .await
            .expect("the index is created");
    }
    db.execute_unprepared("PRAGMA foreign_keys = ON")
        .await
        .expect("foreign keys resume");
}

/// Puts the statement's object in the `refactor_platform` schema.
fn qualify(sql: &str, prefix: &str) -> String {
    sql.replacen(
        &format!("{prefix}\""),
        &format!("{prefix}refactor_platform.\""),
        1,
    )
}

/// Adds `DEFAULT (default)` to the column's definition, if the table has the column.
fn with_default(sql: &str, column: &str, default: &str) -> String {
    let Some(start) = sql
        .find(&format!("( \"{column}\" "))
        .map(|at| at + 2)
        .or_else(|| sql.find(&format!(", \"{column}\" ")).map(|at| at + 2))
    else {
        return sql.to_string();
    };
    let end = start
        + sql[start..]
            .find([',', ')'])
            .expect("a column definition ends");
    format!("{} DEFAULT ({default}){}", &sql[..end], &sql[end..])
}

/// Inserts a user with every column set and returns its id.
pub async fn seed_user(db: &DatabaseConnection) -> Id {
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
        working_hours_start: Set(crate::working_hours::default_start()),
        working_hours_end: Set(crate::working_hours::default_end()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the user is seeded")
    .id
}

/// Inserts an organization with every column set and returns its id.
pub async fn seed_organization(db: &DatabaseConnection) -> Id {
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

/// Gives `user_id` a User role in `organization_id`.
pub async fn seed_membership(db: &DatabaseConnection, user_id: Id, organization_id: Id) {
    let now = Utc::now();
    user_roles::ActiveModel {
        id: Set(Id::new_v4()),
        role: Set(Role::User),
        organization_id: Set(Some(organization_id)),
        user_id: Set(user_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the role is seeded");
}

/// The organization a session's relationship belongs to.
pub async fn organization_of(db: &DatabaseConnection, session_id: Id) -> Id {
    let session = coaching_sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .expect("the session reads")
        .expect("the session exists");
    coaching_relationships::Entity::find_by_id(session.coaching_relationship_id)
        .one(db)
        .await
        .expect("the relationship reads")
        .expect("the relationship exists")
        .organization_id
}

/// Inserts a coaching session and its parents; returns the session and its one user.
pub async fn seed_coaching_session(db: &DatabaseConnection) -> (Id, Id) {
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

/// Inserts a note on `session_id` and returns its id.
pub async fn seed_note(db: &DatabaseConnection, session_id: Id, user_id: Id) -> Id {
    let now = Utc::now();
    notes::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        body: Set(Some("Prep".to_string())),
        user_id: Set(user_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the note is seeded")
    .id
}

/// Inserts an action on `session_id` and returns its id.
pub async fn seed_action(db: &DatabaseConnection, session_id: Id, user_id: Id) -> Id {
    let now = Utc::now();
    actions::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        goal_id: Set(None),
        user_id: Set(user_id),
        body: Set(Some("Follow up".to_string())),
        due_by: Set(None),
        status: Set(Default::default()),
        status_changed_at: Set(now.into()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the action is seeded")
    .id
}

/// Inserts an agreement on `session_id` and returns its id.
pub async fn seed_agreement(db: &DatabaseConnection, session_id: Id, user_id: Id) -> Id {
    let now = Utc::now();
    agreements::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        body: Set(Some("Agreed".to_string())),
        user_id: Set(user_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the agreement is seeded")
    .id
}

/// Inserts a topic on `session_id`, soft-deleted when `deleted`, and returns its id.
pub async fn seed_topic(db: &DatabaseConnection, session_id: Id, user_id: Id, deleted: bool) -> Id {
    let now = Utc::now();
    coaching_session_topics::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        body: Set("Agenda".to_string()),
        user_id: Set(user_id),
        display_order: Set(0),
        status: Set(Default::default()),
        moved_from_session_id: Set(None),
        undo_snapshot: Set(None),
        deleted_at: Set(deleted.then(|| now.into())),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the topic is seeded")
    .id
}

/// A real SQLite foreign key violation, for a mock database to return.
#[cfg(all(test, feature = "mock"))]
pub(crate) async fn foreign_key_violation() -> DbErr {
    let db = database().await;
    let (session_id, user_id) = seed_coaching_session(&db).await;
    seed_note(&db, session_id, user_id).await;
    coaching_sessions::Entity::delete_by_id(session_id)
        .exec(&db)
        .await
        .expect_err("the note's foreign key refuses the delete")
}

/// Ids of a seeded completed transcription and the rows around it.
pub struct SeededTranscription {
    pub session_id: Id,
    pub user_id: Id,
    pub transcription_id: Id,
    pub segment_ids: Vec<Id>,
}

/// Inserts a session, a completed recording and transcription, and `segments` segments.
pub async fn seed_transcription(db: &DatabaseConnection, segments: usize) -> SeededTranscription {
    let (session_id, user_id) = seed_coaching_session(db).await;
    let now = Utc::now();

    let recording_id = Id::new_v4();
    meeting_recording::ActiveModel {
        id: Set(recording_id),
        coaching_session_id: Set(session_id),
        bot_id: Set(format!("bot-{recording_id}")),
        status: Set(MeetingRecordingStatus::Completed),
        video_url: Set(None),
        audio_url: Set(None),
        duration_seconds: Set(None),
        started_at: Set(None),
        ended_at: Set(None),
        error_message: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the recording is seeded");

    let transcription_id = Id::new_v4();
    transcription::ActiveModel {
        id: Set(transcription_id),
        coaching_session_id: Set(session_id),
        meeting_recording_id: Set(recording_id),
        external_id: Set(format!("ext-{transcription_id}")),
        recall_recording_id: Set(None),
        status: Set(TranscriptionStatus::Completed),
        language_code: Set(None),
        speaker_count: Set(None),
        word_count: Set(None),
        duration_seconds: Set(None),
        confidence: Set(None),
        error_message: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the transcription is seeded");

    let mut segment_ids = Vec::with_capacity(segments);
    for index in 0..segments {
        let start_ms = i32::try_from(index).expect("few segments") * 1000;
        let segment = transcript_segment::ActiveModel {
            id: Set(Id::new_v4()),
            transcription_id: Set(transcription_id),
            participant_id: Set(None),
            speaker_label: Set("A".to_string()),
            text: Set(format!("Segment {index}")),
            start_ms: Set(start_ms),
            end_ms: Set(start_ms + 900),
            confidence: Set(None),
            sentiment: Set(None),
            created_at: Set(now.into()),
        }
        .insert(db)
        .await
        .expect("the segment is seeded");
        segment_ids.push(segment.id);
    }

    SeededTranscription {
        session_id,
        user_id,
        transcription_id,
        segment_ids,
    }
}

/// A user who left one of their two organizations: U holds a role in O1 only, coaches R1
/// in O1 and is coached in R2 in O2. Each relationship has one session, one action assigned
/// to U and one goal U wrote; index 0 is R1's, index 1 is R2's.
pub struct LapsedMembership {
    pub user: Id,
    pub o1: Id,
    pub o2: Id,
    pub relationships: [Id; 2],
    pub sessions: [Id; 2],
    pub actions: [Id; 2],
    pub goals: [Id; 2],
}

impl LapsedMembership {
    /// U as their roles stand: a member of O1 only.
    pub fn in_o1(&self) -> Visibility {
        self.member_of(&[self.o1])
    }

    /// U holding a role in both organizations.
    pub fn in_both(&self) -> Visibility {
        self.member_of(&[self.o1, self.o2])
    }

    /// U as a SuperAdmin with no organization role.
    pub fn super_admin(&self) -> Visibility {
        Visibility {
            is_super_admin: true,
            ..self.member_of(&[])
        }
    }

    fn member_of(&self, organization_ids: &[Id]) -> Visibility {
        Visibility {
            user_id: self.user,
            is_super_admin: false,
            admin_org_ids: vec![],
            member_org_ids: organization_ids.to_vec(),
        }
    }
}

/// Seeds a [`LapsedMembership`], with every session dated `date`.
pub async fn seed_lapsed_membership(
    db: &DatabaseConnection,
    date: chrono::NaiveDateTime,
) -> LapsedMembership {
    let now = Utc::now();
    let (user, coachee, coach) = (
        seed_user(db).await,
        seed_user(db).await,
        seed_user(db).await,
    );
    let (o1, o2) = (seed_organization(db).await, seed_organization(db).await);

    user_roles::ActiveModel {
        id: Set(Id::new_v4()),
        role: Set(Role::User),
        organization_id: Set(Some(o1)),
        user_id: Set(user),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the role is seeded");

    let mut seeded = Vec::new();
    for (organization_id, coach_id, coachee_id) in [(o1, user, coachee), (o2, coach, user)] {
        let relationship_id = Id::new_v4();
        coaching_relationships::ActiveModel {
            id: Set(relationship_id),
            organization_id: Set(organization_id),
            coach_id: Set(coach_id),
            coachee_id: Set(coachee_id),
            slug: Set(format!("relationship-{relationship_id}")),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
        .insert(db)
        .await
        .expect("the relationship is seeded");

        let session_id = Id::new_v4();
        coaching_sessions::ActiveModel {
            id: Set(session_id),
            coaching_relationship_id: Set(relationship_id),
            coaching_session_series_id: Set(None),
            ical_sequence: Set(0),
            ical_recurrence_id: Set(None),
            collab_document_name: Set(None),
            date: Set(date),
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
        .expect("the session is seeded");

        let action_id = Id::new_v4();
        actions::ActiveModel {
            id: Set(action_id),
            coaching_session_id: Set(session_id),
            goal_id: Set(None),
            user_id: Set(coach_id),
            body: Set(Some(format!("Action {action_id}"))),
            due_by: Set(None),
            status: Set(Status::InProgress),
            status_changed_at: Set(now.into()),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
        .insert(db)
        .await
        .expect("the action is seeded");
        actions_users::ActiveModel {
            id: Set(Id::new_v4()),
            action_id: Set(action_id),
            user_id: Set(user),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
        .insert(db)
        .await
        .expect("the assignment is seeded");

        let goal_id = Id::new_v4();
        goals::ActiveModel {
            id: Set(goal_id),
            coaching_relationship_id: Set(relationship_id),
            created_in_session_id: Set(None),
            user_id: Set(user),
            title: Set(Some(format!("Goal {goal_id}"))),
            body: Set(None),
            status: Set(Status::InProgress),
            status_changed_at: Set(None),
            completed_at: Set(None),
            target_date: Set(None),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
        .insert(db)
        .await
        .expect("the goal is seeded");

        seeded.push((relationship_id, session_id, action_id, goal_id));
    }
    let [(r1, s1, a1, g1), (r2, s2, a2, g2)] = seeded[..] else {
        unreachable!("two relationships are seeded")
    };

    LapsedMembership {
        user,
        o1,
        o2,
        relationships: [r1, r2],
        sessions: [s1, s2],
        actions: [a1, a2],
        goals: [g1, g2],
    }
}
