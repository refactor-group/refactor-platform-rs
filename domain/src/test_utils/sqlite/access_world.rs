//! The relationship access matrix's fixture (section 2 of
//! `docs/test-plans/relationship_access_manual_testing.md`), seeded into SQLite.
//!
//! S4 and S5 are left out of the shared world: tests seed them with
//! [`seed_content_free_session`], so no list row includes them.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use chrono::{DateTime, NaiveDateTime, TimeDelta, Timelike, Utc};
use entity::meeting_recording::MeetingRecordingStatus;
use entity::roles::Role;
use entity::status::Status;
use entity::topic_status::Status as TopicStatus;
use entity::transcript_analysis::Status as AnalysisStatus;
use entity::transcript_participant::MatchSource;
use entity::transcript_suggestion::Status as SuggestionStatus;
use entity::transcript_suggestion_target::Target;
use entity::transcription::TranscriptionStatus;
use entity::{
    actions, actions_users, agreements, coaching_relationships, coaching_session_images,
    coaching_session_series, coaching_session_topics, coaching_session_views, coaching_sessions,
    coaching_sessions_goals, goals, meeting_recording, notes, transcript_analysis,
    transcript_participant, transcript_segment, transcript_suggestion, transcription,
    user_role_changes, user_roles, users, Id,
};
use entity_api::user::generate_hash;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use crate::action::ActionWithAssignees;
use crate::transcript_suggestion::ActionProposal;

use super::seed_organization;

/// The password every fixture person logs in with.
pub const PASSWORD: &str = "kumquat-password";

/// The search rows look for this word, which every seeded body and title holds.
const SEARCH_WORD: &str = "kumquat";

/// IMG1's stored bytes: a PNG signature.
pub const IMG1_BYTES: &[u8] = b"\x89PNG\r\n\x1a\n";

/// IMG1's content type.
pub const IMG1_CONTENT_TYPE: &str = "image/png";

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
    pub img1: Id,
    /// Where IMG1's bytes live in the object store; the store itself is the caller's.
    pub img1_storage_key: String,
    pub tr1: Id,
    /// The completed transcription of S3 that SG3 comes from; the matrix leaves it unnamed.
    pub tr3: Id,
    pub sg1: Id,
    pub sg2: Id,
    pub sg3: Id,
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
    let (img1, img1_storage_key) = seed_image(db, s1, coach).await;

    let tr1 = seed_transcript(db, s1, coach, id(Caller::Coachee)).await;
    let tr3 = seed_transcript(db, s3, coach, id(Caller::Removed)).await;
    let sg1 = seed_suggestion(db, s1, &tr1, vec![coach]).await;
    let sg2 = seed_suggestion(db, s1, &tr1, vec![id(Caller::Coachee)]).await;
    let sg3 = seed_suggestion(db, s3, &tr3, vec![id(Caller::Removed)]).await;

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
        img1,
        img1_storage_key,
        tr1: tr1.transcription,
        tr3: tr3.transcription,
        sg1,
        sg2,
        sg3,
    }
}

/// A future session with no notes, actions or agreements, like the matrix's S4 (in R1) and
/// S5 (in R3). Seed it per test so list rows never see it.
pub async fn seed_content_free_session(db: &DatabaseConnection, relationship_id: Id) -> Id {
    seed_session(
        db,
        relationship_id,
        None,
        whole_seconds(Utc::now() + TimeDelta::days(4)),
    )
    .await
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

/// The session's relationship, or `None` once it is deleted.
pub async fn session_relationship(db: &DatabaseConnection, session_id: Id) -> Option<Id> {
    coaching_sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .expect("the session reads")
        .map(|session| session.coaching_relationship_id)
}

/// The session's calendar SEQUENCE, or `None` once it is deleted.
pub async fn session_ical_sequence(db: &DatabaseConnection, session_id: Id) -> Option<i32> {
    coaching_sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .expect("the session reads")
        .map(|session| session.ical_sequence)
}

/// Ids of the session's notes, agreements and actions.
pub async fn session_content(db: &DatabaseConnection, session_id: Id) -> BTreeSet<Id> {
    let notes = notes::Entity::find()
        .filter(notes::Column::CoachingSessionId.eq(session_id))
        .all(db)
        .await
        .expect("the notes read")
        .into_iter()
        .map(|note| note.id);
    let agreements = agreements::Entity::find()
        .filter(agreements::Column::CoachingSessionId.eq(session_id))
        .all(db)
        .await
        .expect("the agreements read")
        .into_iter()
        .map(|agreement| agreement.id);
    let actions = actions::Entity::find()
        .filter(actions::Column::CoachingSessionId.eq(session_id))
        .all(db)
        .await
        .expect("the actions read")
        .into_iter()
        .map(|action| action.id);
    notes.chain(agreements).chain(actions).collect()
}

/// The action's assignees, or `None` when there is no such action.
pub async fn action_assignees(db: &DatabaseConnection, action_id: Id) -> Option<BTreeSet<Id>> {
    actions::Entity::find_by_id(action_id)
        .one(db)
        .await
        .expect("the action reads")?;
    Some(
        actions_users::Entity::find()
            .filter(actions_users::Column::ActionId.eq(action_id))
            .all(db)
            .await
            .expect("the assignments read")
            .into_iter()
            .map(|assignment| assignment.user_id)
            .collect(),
    )
}

/// The session's title, or `None` once it is deleted.
pub async fn session_title(db: &DatabaseConnection, session_id: Id) -> Option<Option<String>> {
    coaching_sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .expect("the session reads")
        .map(|session| session.title)
}

/// Ids of every session in the relationship.
pub async fn relationship_sessions(db: &DatabaseConnection, relationship_id: Id) -> BTreeSet<Id> {
    coaching_sessions::Entity::find()
        .filter(coaching_sessions::Column::CoachingRelationshipId.eq(relationship_id))
        .all(db)
        .await
        .expect("the sessions read")
        .into_iter()
        .map(|session| session.id)
        .collect()
}

/// Ids of every series in the relationship.
pub async fn relationship_series(db: &DatabaseConnection, relationship_id: Id) -> BTreeSet<Id> {
    coaching_session_series::Entity::find()
        .filter(coaching_session_series::Column::CoachingRelationshipId.eq(relationship_id))
        .all(db)
        .await
        .expect("the series read")
        .into_iter()
        .map(|series| series.id)
        .collect()
}

/// The series' rule, calendar SEQUENCE and sessions with their starts, or `None` once it is
/// deleted.
pub async fn series_state(
    db: &DatabaseConnection,
    series_id: Id,
) -> Option<(serde_json::Value, i32, BTreeSet<(Id, NaiveDateTime)>)> {
    let series = coaching_session_series::Entity::find_by_id(series_id)
        .one(db)
        .await
        .expect("the series reads")?;
    let sessions = coaching_sessions::Entity::find()
        .filter(coaching_sessions::Column::CoachingSessionSeriesId.eq(series_id))
        .all(db)
        .await
        .expect("the series sessions read")
        .into_iter()
        .map(|session| (session.id, session.date))
        .collect();
    Some((series.rule, series.ical_sequence, sessions))
}

/// Ids of the users who marked the session viewed.
pub async fn session_viewers(db: &DatabaseConnection, session_id: Id) -> BTreeSet<Id> {
    coaching_session_views::Entity::find()
        .filter(coaching_session_views::Column::CoachingSessionId.eq(session_id))
        .all(db)
        .await
        .expect("the views read")
        .into_iter()
        .map(|view| view.user_id)
        .collect()
}

/// Ids of every relationship from `coach_id` to `coachee_id`.
pub async fn relationships_between(
    db: &DatabaseConnection,
    coach_id: Id,
    coachee_id: Id,
) -> BTreeSet<Id> {
    coaching_relationships::Entity::find()
        .filter(coaching_relationships::Column::CoachId.eq(coach_id))
        .filter(coaching_relationships::Column::CoacheeId.eq(coachee_id))
        .all(db)
        .await
        .expect("the relationships read")
        .into_iter()
        .map(|relationship| relationship.id)
        .collect()
}

/// The id of the user with `email`, if there is one.
pub async fn user_with_email(db: &DatabaseConnection, email: &str) -> Option<Id> {
    users::Entity::find()
        .filter(users::Column::Email.eq(email))
        .one(db)
        .await
        .expect("the user reads")
        .map(|user| user.id)
}

/// Whether the user still exists.
pub async fn user_exists(db: &DatabaseConnection, user_id: Id) -> bool {
    users::Entity::find_by_id(user_id)
        .one(db)
        .await
        .expect("the user reads")
        .is_some()
}

/// Drops every role `user_id` holds in `organization_id`, with no audit row.
pub async fn remove_roles(db: &DatabaseConnection, user_id: Id, organization_id: Id) {
    user_roles::Entity::delete_many()
        .filter(user_roles::Column::UserId.eq(user_id))
        .filter(user_roles::Column::OrganizationId.eq(organization_id))
        .exec(db)
        .await
        .expect("the roles are removed");
}

/// The audit row an admin's removal of `user_id` from `organization_id` leaves, which lets
/// that organization's admins attach the user again.
pub async fn record_removal(db: &DatabaseConnection, user_id: Id, organization_id: Id) {
    user_role_changes::ActiveModel {
        id: Set(Id::new_v4()),
        actor_user_id: Set(None),
        target_user_id: Set(user_id),
        organization_id: Set(Some(organization_id)),
        previous_role: Set(Some(Role::User)),
        new_role: Set(None),
        changed_at: Set(Utc::now().into()),
    }
    .insert(db)
    .await
    .expect("the removal is recorded");
}

/// The suggestion's status and the entity accepting it created.
pub async fn suggestion_outcome(
    db: &DatabaseConnection,
    suggestion_id: Id,
) -> (SuggestionStatus, Option<Id>) {
    transcript_suggestion::Entity::find_by_id(suggestion_id)
        .one(db)
        .await
        .expect("the suggestion reads")
        .map(|suggestion| (suggestion.status, suggestion.created_entity_id))
        .expect("the suggestion exists")
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

/// A note image row in `session_id`; the bytes are the caller's to store at the key.
async fn seed_image(db: &DatabaseConnection, session_id: Id, uploader: Id) -> (Id, String) {
    let now = Utc::now();
    let id = Id::new_v4();
    let storage_key = format!("coaching-sessions/{session_id}/images/{id}.png");
    coaching_session_images::ActiveModel {
        id: Set(id),
        coaching_session_id: Set(session_id),
        uploaded_by_id: Set(uploader),
        storage_key: Set(storage_key.clone()),
        mime_type: Set(IMG1_CONTENT_TYPE.to_string()),
        byte_size: Set(i64::try_from(IMG1_BYTES.len()).expect("a small image")),
        width: Set(Some(1)),
        height: Set(Some(1)),
        deleted_at: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the image is seeded");
    (id, storage_key)
}

/// A completed transcription and the completed action scan over it.
struct Transcript {
    transcription: Id,
    analysis: Id,
    /// The coachee's line, which every suggestion quotes.
    coachee_segment: Id,
}

/// A completed recording and transcription of `session_id`, one attributed line from each
/// participant, and a completed action scan.
async fn seed_transcript(
    db: &DatabaseConnection,
    session_id: Id,
    coach: Id,
    coachee: Id,
) -> Transcript {
    let now = Utc::now();
    let recording = meeting_recording::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        bot_id: Set(format!("bot-{session_id}")),
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

    let transcription = transcription::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        meeting_recording_id: Set(recording.id),
        external_id: Set(format!("ext-{}", recording.id)),
        recall_recording_id: Set(None),
        status: Set(TranscriptionStatus::Completed),
        language_code: Set(None),
        speaker_count: Set(Some(2)),
        word_count: Set(None),
        duration_seconds: Set(None),
        confidence: Set(None),
        error_message: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the transcription is seeded")
    .id;

    let mut segments = Vec::new();
    for (index, (user_id, text)) in [
        (coach, format!("What will you do next, {SEARCH_WORD}?")),
        (coachee, format!("I will send the {SEARCH_WORD} plan.")),
    ]
    .into_iter()
    .enumerate()
    {
        let provider_id = format!("{}", 100 * (index + 1));
        let participant = transcript_participant::ActiveModel {
            id: Set(Id::new_v4()),
            transcription_id: Set(transcription),
            provider_participant_id: Set(provider_id.clone()),
            display_name: Set(None),
            is_host: Set(Some(false)),
            platform: Set(None),
            platform_account_id: Set(None),
            extra_data: Set(None),
            user_id: Set(Some(user_id)),
            match_source: Set(Some(MatchSource::Account)),
            created_at: Set(now.into()),
        }
        .insert(db)
        .await
        .expect("the participant is seeded");
        let start_ms = i32::try_from(index).expect("two lines") * 4000;
        let segment = transcript_segment::ActiveModel {
            id: Set(Id::new_v4()),
            transcription_id: Set(transcription),
            participant_id: Set(Some(participant.id)),
            speaker_label: Set(provider_id),
            text: Set(text),
            start_ms: Set(start_ms),
            end_ms: Set(start_ms + 3000),
            confidence: Set(None),
            sentiment: Set(None),
            created_at: Set(now.into()),
        }
        .insert(db)
        .await
        .expect("the segment is seeded");
        segments.push(segment.id);
    }

    let analysis = transcript_analysis::ActiveModel {
        id: Set(Id::new_v4()),
        transcription_id: Set(transcription),
        target: Set(Target::Action),
        status: Set(AnalysisStatus::Completed),
        attempts: Set(1),
        claimed_at: Set(None),
        error_message: Set(None),
        model: Set(None),
        input_tokens: Set(None),
        output_tokens: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the analysis is seeded")
    .id;

    Transcript {
        transcription,
        analysis,
        coachee_segment: segments[1],
    }
}

/// A pending action suggestion from `transcript`, assigned to `assignee_ids`.
async fn seed_suggestion(
    db: &DatabaseConnection,
    session_id: Id,
    transcript: &Transcript,
    assignee_ids: Vec<Id>,
) -> Id {
    let now = Utc::now();
    let placeholder = DateTime::<Utc>::UNIX_EPOCH.fixed_offset();
    let proposal = ActionProposal {
        action: ActionWithAssignees {
            action: actions::Model {
                id: Id::nil(),
                coaching_session_id: session_id,
                goal_id: None,
                user_id: Id::nil(),
                body: Some(format!("Send the {SEARCH_WORD} plan")),
                due_by: None,
                status: Status::NotStarted,
                status_changed_at: placeholder,
                created_at: placeholder,
                updated_at: placeholder,
            },
            assignee_ids,
        },
        due_phrase: None,
    };
    transcript_suggestion::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        transcript_analysis_id: Set(transcript.analysis),
        target: Set(Target::Action),
        proposal: Set(serde_json::to_value(proposal).expect("the proposal serializes")),
        segment_id: Set(transcript.coachee_segment),
        quote: Set(format!("send the {SEARCH_WORD} plan")),
        duplicate_of_id: Set(None),
        status: Set(SuggestionStatus::Pending),
        resolved_by: Set(None),
        resolved_at: Set(None),
        created_entity_id: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the suggestion is seeded")
    .id
}
