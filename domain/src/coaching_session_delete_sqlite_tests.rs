//! Which sessions a delete refuses, and what a delete leaves behind, against a real SQL
//! engine.

use chrono::Utc;
use entity::{coaching_session_topics, coaching_session_views, coaching_sessions};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use service::config::Config;

use super::*;
use crate::test_utils::sqlite::{
    database, seed_action, seed_agreement, seed_coaching_session, seed_note, seed_topic,
    within_time_limit,
};

async fn read(db: &DatabaseConnection, id: Id) -> Option<coaching_sessions::Model> {
    coaching_sessions::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the session reads")
}

/// Deletes `session_id`, expecting the content refusal, and checks nothing changed.
async fn assert_refused(db: &DatabaseConnection, session_id: Id) {
    let before = read(db, session_id).await.expect("the session exists");

    let err = delete(db, &Config::default(), session_id)
        .await
        .expect_err("a session with content is refused");

    let DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::Conflict {
        details: Some(details),
        ..
    })) = err.error_kind
    else {
        panic!("expected a conflict, got {err:?}");
    };
    assert_eq!(details["reason"], "sessions_have_content");
    assert_eq!(
        details["coaching_session_id"],
        serde_json::json!(session_id)
    );
    let after = read(db, session_id).await.expect("the session survives");
    assert_eq!(after.ical_sequence, before.ical_sequence);
}

async fn seed_view(db: &DatabaseConnection, session_id: Id, user_id: Id) -> Id {
    let now = Utc::now();
    coaching_session_views::ActiveModel {
        id: Set(Id::new_v4()),
        user_id: Set(user_id),
        coaching_session_id: Set(session_id),
        last_viewed_at: Set(now.into()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the view is seeded")
    .id
}

#[tokio::test]
async fn sd_01_a_session_with_an_action_is_refused_and_unchanged() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, user_id) = seed_coaching_session(&db).await;
        seed_action(&db, session_id, user_id, "Follow up").await;

        assert_refused(&db, session_id).await;
    })
    .await;
}

#[tokio::test]
async fn sd_02_a_session_with_a_note_or_an_agreement_is_refused() {
    within_time_limit(async {
        let db = database().await;
        let (with_note, user_id) = seed_coaching_session(&db).await;
        seed_note(&db, with_note, user_id, "Prep").await;
        let (with_agreement, user_id) = seed_coaching_session(&db).await;
        seed_agreement(&db, with_agreement, user_id, "Agreed").await;

        assert_refused(&db, with_note).await;
        assert_refused(&db, with_agreement).await;
    })
    .await;
}

#[tokio::test]
async fn sd_03_a_session_without_content_is_deleted_with_its_topics_and_views() {
    within_time_limit(async {
        let db = database().await;
        let (session_id, user_id) = seed_coaching_session(&db).await;
        let topic_id = seed_topic(&db, session_id, user_id, "Agenda", 0, false).await;
        let view_id = seed_view(&db, session_id, user_id).await;

        delete(&db, &Config::default(), session_id)
            .await
            .expect("the delete succeeds");

        assert_eq!(read(&db, session_id).await, None);
        let topic = coaching_session_topics::Entity::find_by_id(topic_id)
            .one(&db)
            .await
            .expect("the topic reads");
        assert_eq!(topic, None);
        let view = coaching_session_views::Entity::find_by_id(view_id)
            .one(&db)
            .await
            .expect("the view reads");
        assert_eq!(view, None);
    })
    .await;
}
