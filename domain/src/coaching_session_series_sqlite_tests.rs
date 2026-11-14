//! What a "this and following" reschedule writes to a moved session, which leftover
//! sessions it treats as holding content, and who series writes record as each session's
//! scheduler, against a real SQL engine.

use std::collections::BTreeMap;

use chrono::{NaiveDate, TimeDelta, Utc};
use entity::{
    agreements, coaching_relationships, coaching_session_series as series_entity, coaching_sessions,
};
use entity_api::coaching_session::Landing;
use entity_api::coaching_session_pending_reschedule::{self, Schedule};
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::{ActiveModelTrait, ActiveValue::Unchanged, DatabaseConnection, EntityTrait, Set};

use super::*;
use crate::test_utils::sqlite::fixture::move_series_into_the_past;
use crate::test_utils::sqlite::{
    database, mirror_migration_defaults, seed_action, seed_agreement, seed_coaching_session,
    seed_membership, seed_organization, seed_topic, seed_user, within_time_limit,
};

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
        scheduled_by_user_id: Set(None),
        meeting_url_created_by_user_id: Set(None),
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

#[tokio::test]
async fn move_to_slot_moves_only_the_target_and_keeps_what_hangs_off_it() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed_series(&db).await;
        let target = opened(&db, seed_series_session(&db, &seeded, monday()).await).await;
        let bystander = seed_series_session(&db, &seeded, monday() + TimeDelta::weeks(1)).await;
        let agreement_id = seed_agreement(&db, target.id, seeded.user_id, "Agreed").await;
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
            Some(seeded.user_id),
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
            agreements::Entity::find_by_id(agreement_id)
                .one(&db)
                .await
                .expect("the agreement reads")
                .map(|agreement| agreement.coaching_session_id),
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
            Some(seeded.user_id),
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

/// Greptile on #471: the session that takes over an overridden occurrence is published
/// above the old override, so the row is raised to match, and its next edit outranks both.
#[tokio::test]
async fn the_occupant_of_a_reused_occurrence_is_raised_above_the_old_override() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed_series(&db).await;
        let occupant = seed_series_session(&db, &seeded, monday()).await;
        let neighbour = seed_series_session(&db, &seeded, monday() + TimeDelta::weeks(1)).await;
        let old_override = 8;

        entity_api::coaching_session::raise_ical_sequence_at(
            &db,
            seeded.series_id,
            monday(),
            old_override + 1,
        )
        .await
        .expect("the raise succeeds");
        let next_edit = entity_api::coaching_session::increment_ical_sequence(&db, occupant.id)
            .await
            .expect("the edit bumps");

        assert_eq!(reread(&db, neighbour.id).await, neighbour);
        assert!(next_edit.ical_sequence > old_override + 1);

        // Never lowered: a floor below the row's own leaves it alone.
        entity_api::coaching_session::raise_ical_sequence_at(&db, seeded.series_id, monday(), 0)
            .await
            .expect("the raise succeeds");
        assert_eq!(
            reread(&db, occupant.id).await.ical_sequence,
            next_edit.ical_sequence
        );
    })
    .await;
}

#[tokio::test]
async fn find_ids_with_content_flags_each_kind_of_content_and_nothing_else() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed_series(&db).await;
        let week = |n| monday() + TimeDelta::weeks(n);
        let with_action = seed_series_session(&db, &seeded, week(1)).await.id;
        let with_agreement = seed_series_session(&db, &seeded, week(2)).await.id;
        let with_topic = seed_series_session(&db, &seeded, week(3)).await.id;
        let with_deleted_topic = seed_series_session(&db, &seeded, week(4)).await.id;
        let empty = seed_series_session(&db, &seeded, week(5)).await.id;
        let not_asked_about = seed_series_session(&db, &seeded, week(6)).await.id;
        let with_shared_notes = opened(&db, seed_series_session(&db, &seeded, week(7)).await)
            .await
            .id;

        seed_action(&db, with_action, seeded.user_id, "Follow up").await;
        seed_agreement(&db, with_agreement, seeded.user_id, "Agreed").await;
        seed_topic(&db, with_topic, seeded.user_id, "Agenda", 0, false).await;
        seed_topic(&db, with_deleted_topic, seeded.user_id, "Agenda", 0, true).await;
        seed_action(&db, not_asked_about, seeded.user_id, "Follow up").await;

        let flagged = entity_api::coaching_session::find_ids_with_content(
            &db,
            &[
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
            [with_action, with_agreement, with_topic, with_shared_notes]
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

/// Two moved series sessions, each with a pending reschedule from an hour earlier.
async fn moved_with_pending_reschedules(db: &DatabaseConnection) -> Vec<coaching_sessions::Model> {
    let seeded = seed_series(db).await;
    let mut moved = Vec::new();
    for week in 0..2 {
        let session = seed_series_session(db, &seeded, monday() + TimeDelta::weeks(week)).await;
        let before = coaching_sessions::Model {
            date: session.date - TimeDelta::hours(1),
            ..session.clone()
        };
        coaching_session_pending_reschedule::record_change(
            db,
            session.id,
            Schedule::from(&before),
            Schedule::from(&session),
            Utc::now().naive_utc(),
            coaching_session_pending_reschedule::LEASE,
        )
        .await
        .expect("the move is pending");
        moved.push(session);
    }
    moved
}

async fn relationship_of(
    db: &DatabaseConnection,
    session: &coaching_sessions::Model,
) -> coaching_relationships::Model {
    coaching_relationships::Entity::find_by_id(session.coaching_relationship_id)
        .one(db)
        .await
        .expect("the relationship reads")
        .expect("the relationship exists")
}

/// A send that did `delivery` for both participants.
fn told_both(delivery: emails::Delivery) -> emails::Delivered {
    emails::Delivered {
        coachee: delivery,
        coach: delivery,
    }
}

async fn pending(db: &DatabaseConnection, id: Id) -> bool {
    coaching_session_pending_reschedule::find(db, id)
        .await
        .expect("the pending row reads")
        .is_some()
}

/// A coachee no longer in the organization is owed nothing: the moved sessions settle, but
/// none is stamped as noticed.
#[tokio::test]
async fn a_following_reschedule_skipping_the_coachee_settles_without_stamping_notice() {
    within_time_limit(async {
        let db = database().await;
        let moved = moved_with_pending_reschedules(&db).await;

        let relationship = relationship_of(&db, &moved[0]).await;
        let returned = record_following_notice(
            &db,
            &relationship,
            told_both(emails::Delivery::NoLongerAMember),
            moved.clone(),
        )
        .await;

        assert_eq!(returned, moved);
        for session in &moved {
            assert!(!pending(&db, session.id).await, "the move settles");
            assert_eq!(
                reread(&db, session.id).await.notice_given_at,
                session.notice_given_at,
                "nobody was told, so notice is not stamped"
            );
        }
    })
    .await;
}

/// The coachee told: each moved session settles and is stamped as noticed.
#[tokio::test]
async fn a_following_reschedule_that_told_the_coachee_settles_and_stamps_notice() {
    within_time_limit(async {
        let db = database().await;
        let moved = moved_with_pending_reschedules(&db).await;

        let relationship = relationship_of(&db, &moved[0]).await;
        let returned = record_following_notice(
            &db,
            &relationship,
            told_both(emails::Delivery::Sent),
            moved.clone(),
        )
        .await;

        for (session, returned) in moved.iter().zip(&returned) {
            assert!(!pending(&db, session.id).await, "the move settles");
            assert!(returned.notice_given_at > session.notice_given_at);
        }
    })
    .await;
}

/// A failed send told nobody: the moves stay pending and unstamped.
#[tokio::test]
async fn a_following_reschedule_that_failed_records_nothing() {
    within_time_limit(async {
        let db = database().await;
        let moved = moved_with_pending_reschedules(&db).await;

        let relationship = relationship_of(&db, &moved[0]).await;
        record_following_notice(
            &db,
            &relationship,
            told_both(emails::Delivery::Failed),
            moved.clone(),
        )
        .await;

        for session in &moved {
            assert!(pending(&db, session.id).await, "the move stays pending");
            assert_eq!(
                reread(&db, session.id).await.notice_given_at,
                session.notice_given_at
            );
        }
    })
    .await;
}

#[tokio::test]
async fn move_to_slot_records_who_moved_the_session() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed_series(&db).await;
        let target = seed_series_session(&db, &seeded, monday()).await;
        let bystander = seed_series_session(&db, &seeded, monday() + TimeDelta::weeks(1)).await;
        let mover = seed_user(&db).await;
        let slot = monday() + TimeDelta::hours(1);

        entity_api::coaching_session::move_to_slot(
            &db,
            target.id,
            Landing {
                series_id: seeded.series_id,
                slot,
                date: slot,
                duration: Duration::from_minutes_unchecked(60),
            },
            Some(mover),
        )
        .await
        .expect("the move succeeds");

        assert_eq!(
            reread(&db, target.id).await.scheduled_by_user_id,
            Some(mover)
        );
        assert_eq!(reread(&db, bystander.id).await, bystander);
    })
    .await;
}

#[tokio::test]
async fn move_to_slot_without_a_scheduler_keeps_the_one_recorded() {
    within_time_limit(async {
        let db = database().await;
        let seeded = seed_series(&db).await;
        let scheduler = seed_user(&db).await;
        let target = coaching_sessions::ActiveModel {
            id: Unchanged(seed_series_session(&db, &seeded, monday()).await.id),
            scheduled_by_user_id: Set(Some(scheduler)),
            ..Default::default()
        }
        .update(&db)
        .await
        .expect("the scheduler is seeded");
        let slot = monday() + TimeDelta::hours(1);

        entity_api::coaching_session::move_to_slot(
            &db,
            target.id,
            Landing {
                series_id: seeded.series_id,
                slot,
                date: slot,
                duration: Duration::from_minutes_unchecked(60),
            },
            None,
        )
        .await
        .expect("the move succeeds");

        let moved = reread(&db, target.id).await;
        assert_eq!(moved.date, slot);
        assert_eq!(moved.ical_recurrence_id, Some(slot));
        assert_eq!(moved.scheduled_by_user_id, Some(scheduler));
    })
    .await;
}

/// A relationship whose coach and coachee both hold a role, in a database with the
/// migrations' defaults; returns it and its coach.
async fn scheduling_relationship(db: &DatabaseConnection) -> (Id, Id) {
    mirror_migration_defaults(db).await;
    let (coach_id, coachee_id) = (seed_user(db).await, seed_user(db).await);
    let organization_id = seed_organization(db).await;
    for user_id in [coach_id, coachee_id] {
        seed_membership(db, user_id, organization_id).await;
    }
    let now = Utc::now();
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
    (relationship_id, coach_id)
}

fn weekly(count: u32) -> Recurrence {
    Recurrence {
        frequency: Frequency::Weekly,
        interval: 1,
        by_weekdays: None,
        count: Some(count),
        until: None,
    }
}

fn days_ahead(days: i64) -> NaiveDateTime {
    (Utc::now() + TimeDelta::days(days))
        .naive_utc()
        .date()
        .and_hms_opt(15, 0, 0)
        .expect("a valid time")
}

async fn schedulers(db: &DatabaseConnection, series_id: Id) -> Vec<Option<Id>> {
    entity_api::coaching_session::find_by_series_id(db, series_id)
        .await
        .expect("the sessions read")
        .into_iter()
        .map(|session| session.scheduled_by_user_id)
        .collect()
}

#[tokio::test]
async fn a_created_series_records_the_acting_user_on_every_session() {
    within_time_limit(async {
        let db = database().await;
        let (relationship_id, coach_id) = scheduling_relationship(&db).await;

        let (series, _) = create_with_sessions(
            &db,
            &Config::default(),
            relationship_id,
            coach_id,
            days_ahead(7),
            weekly(3),
            None,
        )
        .await
        .expect("the series is created");

        assert_eq!(schedulers(&db, series.id).await, vec![Some(coach_id); 3]);
    })
    .await;
}

/// The coach created the series; someone else rescheduling it is recorded on every session
/// the reschedule materializes, and the past sessions keep the coach.
#[tokio::test]
async fn a_rescheduled_series_records_the_acting_user_on_the_sessions_it_creates() {
    within_time_limit(async {
        let db = database().await;
        let (relationship_id, coach_id) = scheduling_relationship(&db).await;
        let (series, _) = create_with_sessions(
            &db,
            &Config::default(),
            relationship_id,
            coach_id,
            days_ahead(7),
            weekly(2),
            None,
        )
        .await
        .expect("the series is created");
        // Upcoming sessions take Postgres advisory locks; past ones are kept.
        move_series_into_the_past(&db, series.id).await;
        let acting_user = seed_user(&db).await;

        let rescheduled = reschedule(
            &db,
            &Config::default(),
            series.id,
            acting_user,
            days_ahead(14),
            weekly(3),
            None,
        )
        .await
        .expect("the series is rescheduled");

        assert_eq!(rescheduled.sessions.len(), 3);
        let mut expected = vec![Some(coach_id); 2];
        expected.extend([Some(acting_user); 3]);
        assert_eq!(schedulers(&db, series.id).await, expected);
    })
    .await;
}

async fn notices(db: &DatabaseConnection, session_id: Id) -> BTreeMap<Id, DateTimeWithTimeZone> {
    entity_api::coaching_session_notice::find_by_coaching_session(db, session_id)
        .await
        .expect("the notices read")
        .into_iter()
        .map(|row| (row.user_id, row.notice_given_at))
        .collect()
}

/// Every created session holds a notice row for the coach and the coachee, each at the
/// session's own notice.
#[tokio::test]
async fn a_created_series_stamps_both_participants_notice_on_every_session() {
    within_time_limit(async {
        let db = database().await;
        let (relationship_id, coach_id) = scheduling_relationship(&db).await;

        let (_, sessions) = create_with_sessions(
            &db,
            &Config::default(),
            relationship_id,
            coach_id,
            days_ahead(7),
            weekly(3),
            None,
        )
        .await
        .expect("the series is created");

        let coachee_id = relationship_of(&db, &sessions[0]).await.coachee_id;
        assert_eq!(sessions.len(), 3);
        for session in &sessions {
            let stored = reread(&db, session.id).await;
            assert_eq!(
                notices(&db, session.id).await,
                BTreeMap::from([
                    (coach_id, stored.notice_given_at),
                    (coachee_id, stored.notice_given_at),
                ])
            );
        }
    })
    .await;
}

/// A "this and following" email restamps the moved sessions' notice of each participant it
/// reached, and nobody else's.
#[tokio::test]
async fn a_following_reschedule_restamps_only_the_participants_it_reached() {
    within_time_limit(async {
        for (coachee, coach) in [
            (emails::Delivery::Sent, emails::Delivery::Failed),
            (emails::Delivery::Failed, emails::Delivery::Sent),
            (emails::Delivery::NoLongerAMember, emails::Delivery::Sent),
            (emails::Delivery::Sent, emails::Delivery::NoLongerAMember),
        ] {
            let db = database().await;
            let (relationship_id, coach_id) = scheduling_relationship(&db).await;
            let (_, moved) = create_with_sessions(
                &db,
                &Config::default(),
                relationship_id,
                coach_id,
                days_ahead(7),
                weekly(2),
                None,
            )
            .await
            .expect("the series is created");
            let relationship = relationship_of(&db, &moved[0]).await;
            let before: Vec<_> = stream::iter(&moved)
                .then(|session| notices(&db, session.id))
                .collect()
                .await;

            record_following_notice(
                &db,
                &relationship,
                emails::Delivered { coachee, coach },
                moved.clone(),
            )
            .await;

            for (session, before) in moved.iter().zip(&before) {
                let after = notices(&db, session.id).await;
                for (user_id, delivery) in [
                    (relationship.coachee_id, coachee),
                    (relationship.coach_id, coach),
                ] {
                    match delivery {
                        emails::Delivery::Sent => assert!(
                            after[&user_id] > before[&user_id],
                            "{coachee:?}/{coach:?}: a participant told is restamped"
                        ),
                        _ => assert_eq!(
                            after[&user_id], before[&user_id],
                            "{coachee:?}/{coach:?}: a participant not told keeps their notice"
                        ),
                    }
                }
            }
        }
    })
    .await;
}
