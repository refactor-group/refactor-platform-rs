//! Which sessions the nearest window and the prior/next lookups select, against a
//! real SQL engine.

use std::collections::BTreeMap;

use chrono::{NaiveDate, Utc};
use entity::{coaching_relationships, coaching_sessions};
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};

use super::*;
use crate::test_utils::sqlite::{database, seed_organization, seed_user, within_time_limit};

/// 2026-10-30 14:00 UTC, the window's center in every test.
fn around() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, 30)
        .unwrap()
        .and_hms_opt(14, 0, 0)
        .unwrap()
}

fn days_from_around(days: i64) -> NaiveDateTime {
    around() + TimeDelta::days(days)
}

struct Relationship {
    id: Id,
    coach_id: Id,
    coachee_id: Id,
}

async fn seed_relationship(db: &DatabaseConnection) -> Relationship {
    let (id, coach_id, coachee_id) = (Id::new_v4(), seed_user(db).await, seed_user(db).await);
    let now = Utc::now();
    coaching_relationships::ActiveModel {
        id: Set(id),
        organization_id: Set(seed_organization(db).await),
        coach_id: Set(coach_id),
        coachee_id: Set(coachee_id),
        slug: Set(format!("relationship-{id}")),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the relationship is seeded");
    Relationship {
        id,
        coach_id,
        coachee_id,
    }
}

async fn seed_session_with_id(
    db: &DatabaseConnection,
    coaching_relationship_id: Id,
    id: Id,
    date: NaiveDateTime,
) -> Id {
    let now = Utc::now();
    coaching_sessions::ActiveModel {
        id: Set(id),
        coaching_relationship_id: Set(coaching_relationship_id),
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
    .expect("the coaching session is seeded")
    .id
}

async fn seed_session(
    db: &DatabaseConnection,
    coaching_relationship_id: Id,
    date: NaiveDateTime,
) -> Id {
    seed_session_with_id(db, coaching_relationship_id, Id::new_v4(), date).await
}

fn window(coaching_relationship_id: Id, before: u64, after: u64) -> NearestWindow {
    NearestWindow {
        coaching_relationship_id,
        around: around(),
        before,
        after,
    }
}

async fn nearest_ids(
    db: &DatabaseConnection,
    user_id: Id,
    window: NearestWindow,
) -> Result<Vec<Id>, Error> {
    Ok(
        find_nearest_by_user_with_includes(db, user_id, window, IncludeOptions::default())
            .await?
            .into_iter()
            .map(|s| s.session.id)
            .collect(),
    )
}

/// The `before` latest earlier sessions and the `after` earliest later ones, latest first.
#[tokio::test]
async fn each_side_keeps_only_the_sessions_closest_to_around() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let rel = seed_relationship(&db).await;
        let mut by_day = BTreeMap::new();
        for day in [-3, -2, -1, 1, 2, 3, 4] {
            by_day.insert(day, seed_session(&db, rel.id, days_from_around(day)).await);
        }

        let ids = nearest_ids(&db, rel.coach_id, window(rel.id, 2, 3)).await?;

        assert_eq!(
            ids,
            [by_day[&3], by_day[&2], by_day[&1], by_day[&-1], by_day[&-2]]
        );
        Ok::<(), Error>(())
    })
    .await
}

/// A session exactly at `around` is the first "after" row, never a "before" one.
#[tokio::test]
async fn a_session_exactly_at_around_counts_as_after() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let rel = seed_relationship(&db).await;
        let earlier = seed_session(&db, rel.id, days_from_around(-1)).await;
        let current = seed_session(&db, rel.id, around()).await;

        assert_eq!(
            nearest_ids(&db, rel.coach_id, window(rel.id, 1, 1)).await?,
            [current, earlier]
        );
        assert_eq!(
            nearest_ids(&db, rel.coach_id, window(rel.id, 1, 0)).await?,
            [earlier]
        );
        assert_eq!(
            nearest_ids(&db, rel.coach_id, window(rel.id, 0, 1)).await?,
            [current]
        );
        Ok::<(), Error>(())
    })
    .await
}

/// No distance cap: sessions months away still fill the window.
#[tokio::test]
async fn sessions_far_from_around_still_count() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let rel = seed_relationship(&db).await;
        let long_ago = seed_session(&db, rel.id, days_from_around(-240)).await;
        let far_ahead = seed_session(&db, rel.id, days_from_around(180)).await;

        let ids = nearest_ids(&db, rel.coach_id, window(rel.id, 2, 2)).await?;

        assert_eq!(ids, [far_ahead, long_ago]);
        Ok::<(), Error>(())
    })
    .await
}

/// Other relationships' sessions never leak in, even ones the same user is part of.
#[tokio::test]
async fn only_the_requested_relationship_is_read() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let rel = seed_relationship(&db).await;
        let mine = seed_session(&db, rel.id, days_from_around(1)).await;
        let other = seed_relationship(&db).await;
        seed_session(&db, other.id, days_from_around(-1)).await;
        seed_session(&db, other.id, around()).await;

        assert_eq!(
            nearest_ids(&db, rel.coach_id, window(rel.id, 10, 10)).await?,
            [mine]
        );
        Ok::<(), Error>(())
    })
    .await
}

/// Coach and coachee read the same window; anyone else reads nothing.
#[tokio::test]
async fn only_participants_read_the_window() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let rel = seed_relationship(&db).await;
        let session = seed_session(&db, rel.id, days_from_around(1)).await;
        let outsider = seed_user(&db).await;

        assert_eq!(
            nearest_ids(&db, rel.coach_id, window(rel.id, 2, 2)).await?,
            [session]
        );
        assert_eq!(
            nearest_ids(&db, rel.coachee_id, window(rel.id, 2, 2)).await?,
            [session]
        );
        assert!(nearest_ids(&db, outsider, window(rel.id, 2, 2))
            .await?
            .is_empty());
        Ok::<(), Error>(())
    })
    .await
}

/// Zero on both sides is a valid, empty window.
#[tokio::test]
async fn zero_on_both_sides_returns_nothing() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let rel = seed_relationship(&db).await;
        seed_session(&db, rel.id, days_from_around(-1)).await;
        seed_session(&db, rel.id, days_from_around(1)).await;

        assert!(nearest_ids(&db, rel.coach_id, window(rel.id, 0, 0))
            .await?
            .is_empty());
        Ok::<(), Error>(())
    })
    .await
}

/// Sessions sharing a date are cut and ordered by id, so the window is stable.
#[tokio::test]
async fn equal_dates_are_ordered_by_id() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let rel = seed_relationship(&db).await;
        // Each pair is inserted against its side's order, so only the id sort can pass.
        let (low, high) = (Id::from_u128(1), Id::from_u128(2));
        seed_session_with_id(&db, rel.id, low, days_from_around(-1)).await;
        seed_session_with_id(&db, rel.id, high, days_from_around(-1)).await;
        let (after_low, after_high) = (Id::from_u128(3), Id::from_u128(4));
        seed_session_with_id(&db, rel.id, after_high, days_from_around(1)).await;
        seed_session_with_id(&db, rel.id, after_low, days_from_around(1)).await;

        // Each side keeps the tied session nearest in (date, id) order.
        assert_eq!(
            nearest_ids(&db, rel.coach_id, window(rel.id, 1, 1)).await?,
            [after_low, high]
        );
        assert_eq!(
            nearest_ids(&db, rel.coach_id, window(rel.id, 2, 2)).await?,
            [after_high, after_low, high, low]
        );
        Ok::<(), Error>(())
    })
    .await
}

/// `include=relationship` loads the relationship and both participants.
#[tokio::test]
async fn include_relationship_loads_coach_and_coachee() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let rel = seed_relationship(&db).await;
        seed_session(&db, rel.id, days_from_around(1)).await;
        let includes = IncludeOptions {
            relationship: true,
            ..Default::default()
        };

        let sessions =
            find_nearest_by_user_with_includes(&db, rel.coachee_id, window(rel.id, 2, 2), includes)
                .await?;

        let [session] = sessions.as_slice() else {
            panic!("expected one session, got {sessions:?}");
        };
        assert_eq!(session.relationship.as_ref().map(|r| r.id), Some(rel.id));
        assert_eq!(session.coach.as_ref().map(|u| u.id), Some(rel.coach_id));
        assert_eq!(session.coachee.as_ref().map(|u| u.id), Some(rel.coachee_id));
        Ok::<(), Error>(())
    })
    .await
}

/// The prior/next lookups share the window's queries but stay strict on both
/// sides: called with a session's own date, they never return that session.
#[tokio::test]
async fn prior_and_next_skip_a_session_at_the_given_date() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let rel = seed_relationship(&db).await;
        seed_session(&db, rel.id, days_from_around(-2)).await;
        let prior = seed_session(&db, rel.id, days_from_around(-1)).await;
        seed_session(&db, rel.id, around()).await;
        let next = seed_session(&db, rel.id, days_from_around(1)).await;
        seed_session(&db, rel.id, days_from_around(2)).await;

        let found_prior = coaching_session::find_prior_session(&db, rel.id, around()).await?;
        let found_next = find_next_session(&db, rel.id, around()).await?;

        assert_eq!(found_prior.map(|s| s.id), Some(prior));
        assert_eq!(found_next.map(|s| s.id), Some(next));
        Ok::<(), Error>(())
    })
    .await
}

/// Prior/next are scoped to the relationship and return nothing past its ends.
#[tokio::test]
async fn prior_and_next_are_none_past_the_relationships_ends() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let rel = seed_relationship(&db).await;
        let only = seed_session(&db, rel.id, around()).await;
        let other = seed_relationship(&db).await;
        seed_session(&db, other.id, days_from_around(-1)).await;
        seed_session(&db, other.id, days_from_around(1)).await;

        assert!(coaching_session::find_prior_session(&db, rel.id, around())
            .await?
            .is_none());
        assert!(find_next_session(&db, rel.id, around()).await?.is_none());
        assert_eq!(
            find_next_session(&db, rel.id, days_from_around(-1))
                .await?
                .map(|s| s.id),
            Some(only)
        );
        Ok::<(), Error>(())
    })
    .await
}
