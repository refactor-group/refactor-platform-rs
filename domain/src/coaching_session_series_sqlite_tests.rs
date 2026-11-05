//! What a "this and following" reschedule writes to a moved session, and which leftover
//! sessions it treats as holding content, against a real SQL engine.

use chrono::{NaiveDate, TimeDelta, Utc};
use entity::{
    actions, agreements, coaching_session_series as series_entity, coaching_session_topics,
    coaching_sessions, notes,
};
use sea_orm::{ActiveModelTrait, ActiveValue::Unchanged, DatabaseConnection, EntityTrait, Set};

use super::*;
use crate::test_utils::sqlite::{database, seed_coaching_session, within_time_limit};
use entity_api::coaching_session::Landing;

fn monday() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 11, 9)
        .unwrap()
        .and_hms_opt(15, 0, 0)
        .unwrap()
}

/// A series and the user who coaches it, seeded beside one standalone session.
struct Seeded {
    relationship_id: Id,
    user_id: Id,
    series_id: Id,
}

async fn seed_series(db: &DatabaseConnection) -> Seeded {
    let (session_id, user_id) = seed_coaching_session(db).await;
    let relationship_id = coaching_sessions::Entity::find_by_id(session_id)
        .one(db)
        .await
        .expect("the session reads")
        .expect("the session exists")
        .coaching_relationship_id;
    let now = Utc::now();
    let series_id = series_entity::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_relationship_id: Set(relationship_id),
        rule: Set(serde_json::json!({})),
        ical_sequence: Set(0),
        created_by_user_id: Set(user_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the series is seeded")
    .id;
    Seeded {
        relationship_id,
        user_id,
        series_id,
    }
}

async fn seed_series_session(
    db: &DatabaseConnection,
    seeded: &Seeded,
    date: NaiveDateTime,
) -> coaching_sessions::Model {
    seed_series_session_with_id(db, seeded, Id::new_v4(), date).await
}

async fn seed_series_session_with_id(
    db: &DatabaseConnection,
    seeded: &Seeded,
    id: Id,
    date: NaiveDateTime,
) -> coaching_sessions::Model {
    let now = Utc::now();
    coaching_sessions::ActiveModel {
        id: Set(id),
        coaching_relationship_id: Set(seeded.relationship_id),
        coaching_session_series_id: Set(Some(seeded.series_id)),
        ical_sequence: Set(2),
        ical_recurrence_id: Set(Some(date)),
        collab_document_name: Set(None),
        date: Set(date),
        duration_minutes: Set(60),
        title: Set(Some("Weekly".to_string())),
        meeting_url: Set(Some("https://meet.example/abc".to_string())),
        provider: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        hydrated_at: Set(Some(now.into())),
        notice_given_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the series session is seeded")
}

/// The session as it is once first opened: it has a shared notes document.
async fn opened(
    db: &DatabaseConnection,
    session: coaching_sessions::Model,
) -> coaching_sessions::Model {
    coaching_sessions::ActiveModel {
        id: Unchanged(session.id),
        collab_document_name: Set(Some(format!("doc-{}", session.id))),
        ..Default::default()
    }
    .update(db)
    .await
    .expect("the session is opened")
}

async fn reread(db: &DatabaseConnection, id: Id) -> coaching_sessions::Model {
    coaching_sessions::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the session reads")
        .expect("the session still exists")
}

async fn seed_note(db: &DatabaseConnection, session_id: Id, user_id: Id) -> Id {
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

async fn seed_action(db: &DatabaseConnection, session_id: Id, user_id: Id) {
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
    .expect("the action is seeded");
}

async fn seed_agreement(db: &DatabaseConnection, session_id: Id, user_id: Id) {
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
    .expect("the agreement is seeded");
}

async fn seed_topic(db: &DatabaseConnection, session_id: Id, user_id: Id, deleted: bool) {
    let now = Utc::now();
    coaching_session_topics::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_session_id: Set(session_id),
        body: Set("Agenda".to_string()),
        user_id: Set(user_id),
        display_order: Set(0),
        priority: Set(None),
        status: Set(Default::default()),
        moved_from_session_id: Set(None),
        undo_snapshot: Set(None),
        deleted_at: Set(deleted.then(|| now.into())),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the topic is seeded");
}

#[tokio::test]
async fn move_to_slot_moves_only_the_target_and_keeps_what_hangs_off_it() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed_series(&db).await;
        let target = opened(&db, seed_series_session(&db, &seeded, monday()).await).await;
        let bystander = seed_series_session(&db, &seeded, monday() + TimeDelta::weeks(1)).await;
        let note_id = seed_note(&db, target.id, seeded.user_id).await;
        let new_series = series_entity::ActiveModel {
            id: Set(Id::new_v4()),
            coaching_relationship_id: Set(seeded.relationship_id),
            rule: Set(serde_json::json!({})),
            ical_sequence: Set(0),
            created_by_user_id: Set(seeded.user_id),
            created_at: Set(Utc::now().into()),
            updated_at: Set(Utc::now().into()),
        }
        .insert(&db)
        .await
        .expect("the new series is seeded")
        .id;
        let slot = monday() + TimeDelta::days(1) + TimeDelta::hours(2);

        entity_api::coaching_session::move_to_slot(
            &db,
            target.id,
            Landing {
                series_id: new_series,
                slot,
                date: slot,
                duration: Duration::from_minutes_unchecked(45),
            },
        )
        .await
        .expect("the move succeeds");

        let moved = reread(&db, target.id).await;
        assert_eq!(moved.date, slot);
        assert_eq!(moved.ical_recurrence_id, Some(slot));
        assert_eq!(moved.coaching_session_series_id, Some(new_series));
        assert_eq!(moved.duration_minutes, 45);
        assert_eq!(moved.ical_sequence, target.ical_sequence + 1);
        assert_eq!(moved.title, target.title);
        assert_eq!(moved.meeting_url, target.meeting_url);
        assert_eq!(moved.collab_document_name, target.collab_document_name);
        assert_eq!(
            notes::Entity::find_by_id(note_id)
                .one(&db)
                .await
                .expect("the note reads")
                .map(|note| note.coaching_session_id),
            Some(target.id),
        );

        let untouched = reread(&db, bystander.id).await;
        assert_eq!(untouched, bystander);
    })
    .await;
}

#[tokio::test]
async fn move_to_slot_keeps_a_date_of_its_own_while_taking_the_slot() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed_series(&db).await;
        let target = seed_series_session(&db, &seeded, monday()).await;
        let slot = monday() + TimeDelta::weeks(1);
        let own_date = slot + TimeDelta::days(1) + TimeDelta::hours(1);

        entity_api::coaching_session::move_to_slot(
            &db,
            target.id,
            Landing {
                series_id: seeded.series_id,
                slot,
                date: own_date,
                duration: Duration::from_minutes_unchecked(90),
            },
        )
        .await
        .expect("the move succeeds");

        let moved = reread(&db, target.id).await;
        assert_eq!(moved.date, own_date);
        assert_eq!(moved.ical_recurrence_id, Some(slot));
        assert_eq!(moved.duration_minutes, 90);
        assert_eq!(moved.ical_sequence, target.ical_sequence + 1);
    })
    .await;
}

#[tokio::test]
async fn find_ids_with_content_flags_each_kind_of_content_and_nothing_else() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed_series(&db).await;
        let week = |n| monday() + TimeDelta::weeks(n);
        let with_note = seed_series_session(&db, &seeded, week(0)).await.id;
        let with_action = seed_series_session(&db, &seeded, week(1)).await.id;
        let with_agreement = seed_series_session(&db, &seeded, week(2)).await.id;
        let with_topic = seed_series_session(&db, &seeded, week(3)).await.id;
        let with_deleted_topic = seed_series_session(&db, &seeded, week(4)).await.id;
        let empty = seed_series_session(&db, &seeded, week(5)).await.id;
        let not_asked_about = seed_series_session(&db, &seeded, week(6)).await.id;
        let with_shared_notes = opened(&db, seed_series_session(&db, &seeded, week(7)).await)
            .await
            .id;

        seed_note(&db, with_note, seeded.user_id).await;
        seed_action(&db, with_action, seeded.user_id).await;
        seed_agreement(&db, with_agreement, seeded.user_id).await;
        seed_topic(&db, with_topic, seeded.user_id, false).await;
        seed_topic(&db, with_deleted_topic, seeded.user_id, true).await;
        seed_note(&db, not_asked_about, seeded.user_id).await;

        let flagged = entity_api::coaching_session::find_ids_with_content(
            &db,
            &[
                with_note,
                with_action,
                with_agreement,
                with_topic,
                with_deleted_topic,
                empty,
                with_shared_notes,
            ],
        )
        .await
        .expect("the query runs");

        assert_eq!(
            flagged,
            [
                with_note,
                with_action,
                with_agreement,
                with_topic,
                with_shared_notes
            ]
            .into_iter()
            .collect()
        );
    })
    .await;
}

#[tokio::test]
async fn find_ids_with_content_of_no_sessions_is_empty() {
    within_time_limit(async {
        let db = database().await;
        let flagged = entity_api::coaching_session::find_ids_with_content(&db, &[])
            .await
            .expect("the query runs");
        assert!(flagged.is_empty());
    })
    .await;
}

#[tokio::test]
async fn find_by_series_id_breaks_date_ties_by_id() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed_series(&db).await;
        let later = seed_series_session(&db, &seeded, monday() + TimeDelta::weeks(1)).await;
        // The larger id goes in first, so only the id tie-break can put it second.
        let (first, second) = (Id::from_u128(1), Id::from_u128(2));
        seed_series_session_with_id(&db, &seeded, second, monday()).await;
        seed_series_session_with_id(&db, &seeded, first, monday()).await;

        let ordered: Vec<Id> =
            entity_api::coaching_session::find_by_series_id(&db, seeded.series_id)
                .await
                .expect("the read succeeds")
                .into_iter()
                .map(|session| session.id)
                .collect();

        assert_eq!(ordered, vec![first, second, later.id]);
    })
    .await;
}
