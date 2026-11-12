//! The relationship access matrix's fixture (section 2 of
//! `docs/test-plans/relationship_access_manual_testing.md`), seeded into SQLite.
//!
//! Not seeded yet: IMG1, TR1 and SG1 to SG3.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use chrono::{NaiveDateTime, TimeDelta, Timelike, Utc};
use entity::roles::Role;
use entity::status::Status;
use entity::topic_status::Status as TopicStatus;
use entity::{
    actions, actions_users, agreements, coaching_relationships, coaching_session_series,
    coaching_session_topics, coaching_sessions, coaching_sessions_goals, goals, notes, user_roles,
    users, Id,
};
use entity_api::user::generate_hash;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use super::seed_organization;

/// The password every fixture person logs in with.
pub const PASSWORD: &str = "kumquat-password";

/// The search rows look for this word, which every seeded body and title holds.
const SEARCH_WORD: &str = "kumquat";

/// The matrix's cast, by fixture name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Caller {
    Coach,
    Coachee,
    OtherCoach,
    Member,
    Admin,
    Outsider,
    Super,
    Removed,
}

impl Caller {
    pub const ALL: [Caller; 8] = [
        Caller::Coach,
        Caller::Coachee,
        Caller::OtherCoach,
        Caller::Member,
        Caller::Admin,
        Caller::Outsider,
        Caller::Super,
        Caller::Removed,
    ];

    /// The login email, for example `other_coach@access.test`.
    pub fn email(self) -> String {
        let name = match self {
            Caller::Coach => "coach",
            Caller::Coachee => "coachee",
            Caller::OtherCoach => "other_coach",
            Caller::Member => "member",
            Caller::Admin => "admin",
            Caller::Outsider => "outsider",
            Caller::Super => "super",
            Caller::Removed => "removed",
        };
        format!("{name}@access.test")
    }
}

/// Ids of every seeded row, named as in the matrix.
pub struct AccessWorld {
    pub users: BTreeMap<Caller, Id>,
    pub o1: Id,
    pub o2: Id,
    pub r1: Id,
    pub r2: Id,
    pub r3: Id,
    pub s1: Id,
    pub s2: Id,
    pub s3: Id,
    pub sr1: Id,
    pub sr1_sessions: [Id; 2],
    pub sr3: Id,
    pub sr3_sessions: [Id; 2],
    pub a1: Id,
    pub a2: Id,
    pub a3: Id,
    pub a4: Id,
    pub a5: Id,
    pub a6: Id,
    pub a7: Id,
    pub g1: Id,
    pub g2: Id,
    pub n1: Id,
    pub ag1: Id,
    pub t1: Id,
    pub t2: Id,
}

impl AccessWorld {
    /// The caller's user id.
    pub fn user(&self, caller: Caller) -> Id {
        self.users[&caller]
    }
}

/// Seeds the whole cast and its rows. Every session is in the future and already
/// hydrated, with a collaboration document; first-read hydration needs Postgres.
pub async fn seed_access_world(db: &DatabaseConnection) -> AccessWorld {
    let (o1, o2) = (seed_organization(db).await, seed_organization(db).await);

    let mut users = BTreeMap::new();
    for caller in Caller::ALL {
        users.insert(caller, seed_person(db, caller).await);
    }
    let id = |caller: Caller| users[&caller];

    for (caller, role, organization_id) in [
        (Caller::Coach, Role::User, Some(o1)),
        (Caller::Coachee, Role::User, Some(o1)),
        (Caller::OtherCoach, Role::User, Some(o1)),
        (Caller::Member, Role::User, Some(o1)),
        (Caller::Admin, Role::Admin, Some(o1)),
        (Caller::Outsider, Role::User, Some(o2)),
        (Caller::Super, Role::SuperAdmin, None),
    ] {
        seed_role(db, id(caller), role, organization_id).await;
    }

    let r1 = seed_relationship(db, o1, id(Caller::Coach), id(Caller::Coachee)).await;
    let r2 = seed_relationship(db, o1, id(Caller::OtherCoach), id(Caller::Coachee)).await;
    let r3 = seed_relationship(db, o1, id(Caller::Coach), id(Caller::Removed)).await;

    let soon = whole_seconds(Utc::now() + TimeDelta::days(3));
    let s1 = seed_session(db, r1, None, soon).await;
    let s2 = seed_session(db, r2, None, soon).await;
    let s3 = seed_session(db, r3, None, soon).await;
    let (sr1, sr1_sessions) = seed_series(db, r1, id(Caller::Coach)).await;
    let (sr3, sr3_sessions) = seed_series(db, r3, id(Caller::Coach)).await;

    let g1 = seed_goal(db, r1, s1, id(Caller::Coach)).await;
    let g2 = seed_goal(db, r2, s2, id(Caller::OtherCoach)).await;

    let coach = id(Caller::Coach);
    let other_coach = id(Caller::OtherCoach);
    let a1 = seed_action(db, s1, coach, Some(coach)).await;
    let a2 = seed_action(db, s1, coach, Some(id(Caller::Coachee))).await;
    let a3 = seed_action(db, s1, coach, None).await;
    let a4 = seed_action(db, s2, other_coach, Some(other_coach)).await;
    let a5 = seed_action(db, s2, other_coach, Some(id(Caller::Coachee))).await;
    let a6 = seed_action(db, s3, coach, Some(id(Caller::Removed))).await;
    let a7 = seed_action(db, s3, coach, None).await;

    let n1 = seed_note(db, s1, coach).await;
    let ag1 = seed_agreement(db, s1, coach).await;
    let t1 = seed_topic(db, s1, coach, 0).await;
    let t2 = seed_topic(db, s1, id(Caller::Coachee), 1).await;

    AccessWorld {
        users,
        o1,
        o2,
        r1,
        r2,
        r3,
        s1,
        s2,
        s3,
        sr1,
        sr1_sessions,
        sr3,
        sr3_sessions,
        a1,
        a2,
        a3,
        a4,
        a5,
        a6,
        a7,
        g1,
        g2,
        n1,
        ag1,
        t1,
        t2,
    }
}

/// Whether `user_id` holds any role in `organization_id`.
pub async fn holds_role(db: &DatabaseConnection, user_id: Id, organization_id: Id) -> bool {
    user_roles::Entity::find()
        .filter(user_roles::Column::UserId.eq(user_id))
        .filter(user_roles::Column::OrganizationId.eq(organization_id))
        .one(db)
        .await
        .expect("the role reads")
        .is_some()
}

/// The session's stored start and duration, or `None` once it is deleted.
pub async fn session_schedule(
    db: &DatabaseConnection,
    session_id: Id,
) -> Option<(NaiveDateTime, i16)> {
    coaching_sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .expect("the session reads")
        .map(|session| (session.date, session.duration_minutes))
}

/// Argon2 is slow by design; one hash serves every person in every test.
fn password_hash() -> String {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| generate_hash(PASSWORD.to_string()))
        .clone()
}

fn whole_seconds(at: chrono::DateTime<Utc>) -> NaiveDateTime {
    at.naive_utc().with_nanosecond(0).expect("whole seconds")
}

async fn seed_person(db: &DatabaseConnection, caller: Caller) -> Id {
    let now = Utc::now();
    users::ActiveModel {
        id: Set(Id::new_v4()),
        email: Set(caller.email()),
        first_name: Set(format!("{caller:?}")),
        last_name: Set("Access".to_string()),
        display_name: Set(None),
        password: Set(Some(password_hash())),
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
    .expect("the person is seeded")
    .id
}

async fn seed_role(db: &DatabaseConnection, user_id: Id, role: Role, organization_id: Option<Id>) {
    let now = Utc::now();
    user_roles::ActiveModel {
        id: Set(Id::new_v4()),
        role: Set(role),
        organization_id: Set(organization_id),
        user_id: Set(user_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the role is seeded");
}

async fn seed_relationship(
    db: &DatabaseConnection,
    organization_id: Id,
    coach_id: Id,
    coachee_id: Id,
) -> Id {
    let now = Utc::now();
    let id = Id::new_v4();
    coaching_relationships::ActiveModel {
        id: Set(id),
        organization_id: Set(organization_id),
        coach_id: Set(coach_id),
        coachee_id: Set(coachee_id),
        slug: Set(format!("relationship-{id}")),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the relationship is seeded")
    .id
}

async fn seed_session(
    db: &DatabaseConnection,
    relationship_id: Id,
    series_id: Option<Id>,
    date: NaiveDateTime,
) -> Id {
    let now = Utc::now();
    let id = Id::new_v4();
    coaching_sessions::ActiveModel {
        id: Set(id),
        coaching_relationship_id: Set(relationship_id),
        coaching_session_series_id: Set(series_id),
        ical_sequence: Set(0),
        ical_recurrence_id: Set(series_id.map(|_| date)),
        collab_document_name: Set(Some(format!(
            "access.relationship-{relationship_id}.{id}-v0"
        ))),
        date: Set(date),
        duration_minutes: Set(60),
        title: Set(Some(format!("Session {SEARCH_WORD}"))),
        meeting_url: Set(None),
        provider: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        hydrated_at: Set(Some(now.into())),
        notice_given_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the session is seeded")
    .id
}

/// A weekly series of two sessions, starting next week.
async fn seed_series(db: &DatabaseConnection, relationship_id: Id, creator: Id) -> (Id, [Id; 2]) {
    let start = whole_seconds(Utc::now() + TimeDelta::weeks(1));
    let now = Utc::now();
    let series_id = Id::new_v4();
    coaching_session_series::ActiveModel {
        id: Set(series_id),
        coaching_relationship_id: Set(relationship_id),
        rule: Set(serde_json::json!({
            "start_at": start,
            "recurrence": { "frequency": "weekly", "interval": 1, "count": 2 },
            "duration_minutes": 60,
            "timezone": "UTC",
        })),
        ical_sequence: Set(0),
        created_by_user_id: Set(creator),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the series is seeded");
    let first = seed_session(db, relationship_id, Some(series_id), start).await;
    let second = seed_session(
        db,
        relationship_id,
        Some(series_id),
        start + TimeDelta::weeks(1),
    )
    .await;
    (series_id, [first, second])
}

/// A goal created in `session_id` and linked to it.
async fn seed_goal(db: &DatabaseConnection, relationship_id: Id, session_id: Id, author: Id) -> Id {
    let now = Utc::now();
    let id = goals::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_relationship_id: Set(relationship_id),
        created_in_session_id: Set(Some(session_id)),
        user_id: Set(author),
        title: Set(Some(format!("Goal {SEARCH_WORD}"))),
        body: Set(Some(format!("Goal body {SEARCH_WORD}"))),
        status: Set(Status::InProgress),
        status_changed_at: Set(None),
        completed_at: Set(None),
        target_date: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the goal is seeded")
    .id;
    coaching_sessions_goals::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        goal_id: Set(id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the goal link is seeded");
    id
}

async fn seed_action(
    db: &DatabaseConnection,
    session_id: Id,
    author: Id,
    assignee: Option<Id>,
) -> Id {
    let now = Utc::now();
    let id = actions::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        goal_id: Set(None),
        user_id: Set(author),
        body: Set(Some(format!("Action {SEARCH_WORD}"))),
        due_by: Set(None),
        status: Set(Status::InProgress),
        status_changed_at: Set(now.into()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the action is seeded")
    .id;
    if let Some(user_id) = assignee {
        actions_users::ActiveModel {
            id: Set(Id::new_v4()),
            action_id: Set(id),
            user_id: Set(user_id),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
        .insert(db)
        .await
        .expect("the assignment is seeded");
    }
    id
}

async fn seed_note(db: &DatabaseConnection, session_id: Id, author: Id) -> Id {
    let now = Utc::now();
    notes::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        body: Set(Some(format!("Note {SEARCH_WORD}"))),
        user_id: Set(author),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the note is seeded")
    .id
}

async fn seed_agreement(db: &DatabaseConnection, session_id: Id, author: Id) -> Id {
    let now = Utc::now();
    agreements::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        body: Set(Some(format!("Agreement {SEARCH_WORD}"))),
        user_id: Set(author),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the agreement is seeded")
    .id
}

async fn seed_topic(db: &DatabaseConnection, session_id: Id, author: Id, display_order: i32) -> Id {
    let now = Utc::now();
    coaching_session_topics::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        body: Set(format!("Topic {SEARCH_WORD}")),
        user_id: Set(author),
        display_order: Set(display_order),
        priority: Set(None),
        status: Set(TopicStatus::Open),
        moved_from_session_id: Set(None),
        undo_snapshot: Set(None),
        deleted_at: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the topic is seeded")
    .id
}
