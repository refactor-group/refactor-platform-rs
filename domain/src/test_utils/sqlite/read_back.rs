//! Reads of the rows a request may change, for asserting what it left in the database.

use std::collections::BTreeSet;

use chrono::{NaiveDateTime, NaiveTime};
use entity::meeting_recording::MeetingRecordingStatus;
use entity::roles::Role;
use entity::status::Status;
use entity::topic_status::Status as TopicStatus;
use entity::transcript_suggestion::Status as SuggestionStatus;
use entity::{
    actions, actions_users, agreements, coaching_relationships, coaching_session_images,
    coaching_session_series, coaching_session_topics, coaching_session_views, coaching_sessions,
    coaching_sessions_goals, goals, meeting_recording, notes, oauth_connections, organizations,
    transcript_analysis, transcript_segment, transcript_suggestion, user_roles, users, Id,
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

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

/// The role `user_id` holds in `organization_id`, if any.
pub async fn role_in(db: &DatabaseConnection, user_id: Id, organization_id: Id) -> Option<Role> {
    user_roles::Entity::find()
        .filter(user_roles::Column::UserId.eq(user_id))
        .filter(user_roles::Column::OrganizationId.eq(organization_id))
        .one(db)
        .await
        .expect("the role reads")
        .map(|role| role.role)
}

/// The id of the organization named `name`, if there is one.
pub async fn organization_named(db: &DatabaseConnection, name: &str) -> Option<Id> {
    organizations::Entity::find()
        .filter(organizations::Column::Name.eq(name))
        .one(db)
        .await
        .expect("the organization reads")
        .map(|organization| organization.id)
}

/// The organization's name and whether it is archived, or `None` once it is deleted.
pub async fn organization_state(
    db: &DatabaseConnection,
    organization_id: Id,
) -> Option<(String, bool)> {
    organizations::Entity::find_by_id(organization_id)
        .one(db)
        .await
        .expect("the organization reads")
        .map(|organization| (organization.name, organization.archived_at.is_some()))
}

/// The user's stored working hours, start and end.
pub async fn working_hours(db: &DatabaseConnection, user_id: Id) -> Option<(NaiveTime, NaiveTime)> {
    users::Entity::find_by_id(user_id)
        .one(db)
        .await
        .expect("the user reads")
        .map(|user| (user.working_hours_start, user.working_hours_end))
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

/// The note's body, or `None` once it is deleted.
pub async fn note_body(db: &DatabaseConnection, note_id: Id) -> Option<Option<String>> {
    notes::Entity::find_by_id(note_id)
        .one(db)
        .await
        .expect("the note reads")
        .map(|note| note.body)
}

/// The agreement's body, or `None` once it is deleted.
pub async fn agreement_body(db: &DatabaseConnection, agreement_id: Id) -> Option<Option<String>> {
    agreements::Entity::find_by_id(agreement_id)
        .one(db)
        .await
        .expect("the agreement reads")
        .map(|agreement| agreement.body)
}

/// The action's body and status, or `None` once it is deleted.
pub async fn action_state(
    db: &DatabaseConnection,
    action_id: Id,
) -> Option<(Option<String>, Status)> {
    actions::Entity::find_by_id(action_id)
        .one(db)
        .await
        .expect("the action reads")
        .map(|action| (action.body, action.status))
}

/// The goal's title, body and status, or `None` once it is deleted.
pub async fn goal_state(
    db: &DatabaseConnection,
    goal_id: Id,
) -> Option<(Option<String>, Option<String>, Status)> {
    goals::Entity::find_by_id(goal_id)
        .one(db)
        .await
        .expect("the goal reads")
        .map(|goal| (goal.title, goal.body, goal.status))
}

/// Ids of every goal in the relationship.
pub async fn relationship_goals(db: &DatabaseConnection, relationship_id: Id) -> BTreeSet<Id> {
    goals::Entity::find()
        .filter(goals::Column::CoachingRelationshipId.eq(relationship_id))
        .all(db)
        .await
        .expect("the goals read")
        .into_iter()
        .map(|goal| goal.id)
        .collect()
}

/// Ids of the goals linked to the session.
pub async fn session_goals(db: &DatabaseConnection, session_id: Id) -> BTreeSet<Id> {
    coaching_sessions_goals::Entity::find()
        .filter(coaching_sessions_goals::Column::CoachingSessionId.eq(session_id))
        .all(db)
        .await
        .expect("the goal links read")
        .into_iter()
        .map(|link| link.goal_id)
        .collect()
}

/// A topic as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopicRow {
    pub session: Id,
    pub author: Id,
    pub body: String,
    pub status: TopicStatus,
    pub display_order: i32,
    pub deleted: bool,
}

/// The topic, deleted or not, or `None` when there is no such topic.
pub async fn topic_row(db: &DatabaseConnection, topic_id: Id) -> Option<TopicRow> {
    coaching_session_topics::Entity::find_by_id(topic_id)
        .one(db)
        .await
        .expect("the topic reads")
        .map(|topic| TopicRow {
            session: topic.coaching_session_id,
            author: topic.user_id,
            body: topic.body,
            status: topic.status,
            display_order: topic.display_order,
            deleted: topic.deleted_at.is_some(),
        })
}

/// Ids of every topic in the session, deleted ones included.
pub async fn session_topics(db: &DatabaseConnection, session_id: Id) -> BTreeSet<Id> {
    coaching_session_topics::Entity::find()
        .filter(coaching_session_topics::Column::CoachingSessionId.eq(session_id))
        .all(db)
        .await
        .expect("the topics read")
        .into_iter()
        .map(|topic| topic.id)
        .collect()
}

/// Ids of the transcription's segments.
pub async fn transcription_segments(db: &DatabaseConnection, transcription_id: Id) -> BTreeSet<Id> {
    transcript_segment::Entity::find()
        .filter(transcript_segment::Column::TranscriptionId.eq(transcription_id))
        .all(db)
        .await
        .expect("the segments read")
        .into_iter()
        .map(|segment| segment.id)
        .collect()
}

/// Ids of the session's meeting recordings.
pub async fn session_recordings(db: &DatabaseConnection, session_id: Id) -> BTreeSet<Id> {
    meeting_recording::Entity::find()
        .filter(meeting_recording::Column::CoachingSessionId.eq(session_id))
        .all(db)
        .await
        .expect("the recordings read")
        .into_iter()
        .map(|recording| recording.id)
        .collect()
}

/// The recording's bot id and status.
pub async fn recording_state(
    db: &DatabaseConnection,
    recording_id: Id,
) -> Option<(String, MeetingRecordingStatus)> {
    meeting_recording::Entity::find_by_id(recording_id)
        .one(db)
        .await
        .expect("the recording reads")
        .map(|recording| (recording.bot_id, recording.status))
}

/// Whether the transcription has an action scan, in any state.
pub async fn transcription_scanned(db: &DatabaseConnection, transcription_id: Id) -> bool {
    transcript_analysis::Entity::find()
        .filter(transcript_analysis::Column::TranscriptionId.eq(transcription_id))
        .one(db)
        .await
        .expect("the scan reads")
        .is_some()
}

/// Ids of the session's note images, deleted ones included.
pub async fn session_images(db: &DatabaseConnection, session_id: Id) -> BTreeSet<Id> {
    coaching_session_images::Entity::find()
        .filter(coaching_session_images::Column::CoachingSessionId.eq(session_id))
        .all(db)
        .await
        .expect("the images read")
        .into_iter()
        .map(|image| image.id)
        .collect()
}

/// Whether the image is marked deleted, or `None` when there is no such image.
pub async fn image_deleted(db: &DatabaseConnection, image_id: Id) -> Option<bool> {
    coaching_session_images::Entity::find_by_id(image_id)
        .one(db)
        .await
        .expect("the image reads")
        .map(|image| image.deleted_at.is_some())
}

/// The session's meeting link, or `None` once it is deleted.
pub async fn session_meeting_url(
    db: &DatabaseConnection,
    session_id: Id,
) -> Option<Option<String>> {
    coaching_sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .expect("the session reads")
        .map(|session| session.meeting_url)
}

/// Ids of the users holding any provider connection.
pub async fn connected_users(db: &DatabaseConnection) -> BTreeSet<Id> {
    oauth_connections::Entity::find()
        .all(db)
        .await
        .expect("the connections read")
        .into_iter()
        .map(|connection| connection.user_id)
        .collect()
}
