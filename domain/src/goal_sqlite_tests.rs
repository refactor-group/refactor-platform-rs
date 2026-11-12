//! Goals against a real SQL engine: a user's own goals only in relationships the user may
//! still access, and the timestamps a status change leaves in the row.

use std::collections::BTreeSet;

use chrono::{DateTime, TimeZone, Utc};
use entity::{coaching_sessions, status::Status};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set, Value};

use super::*;
use crate::coaching_relationships::Visibility;
use crate::test_utils::sqlite::{
    database, seed_coaching_session, seed_lapsed_membership, within_time_limit,
};
use crate::{FilterOnly, QueryFilterMap};

/// Goals written by one user, as `GET /users/{id}/goals` filters them.
struct ByUser(Id);

impl IntoQueryFilterMap for ByUser {
    fn into_query_filter_map(self) -> QueryFilterMap {
        let mut map = QueryFilterMap::new();
        map.insert("user_id".to_string(), Some(Value::Uuid(Some(self.0))));
        map
    }
}

/// The ids of `user_id`'s goals `viewer` reads.
async fn own_goal_ids(
    db: &DatabaseConnection,
    viewer: &Visibility,
    user_id: Id,
) -> Result<BTreeSet<Id>, Error> {
    Ok(find_accessible_by(db, viewer, FilterOnly(ByUser(user_id)))
        .await?
        .into_iter()
        .map(|g| g.id)
        .collect())
}

/// M1-09
#[tokio::test]
async fn a_users_own_goals_skip_an_org_they_left() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let m = seed_lapsed_membership(&db, Utc::now().naive_utc()).await;
        assert_eq!(
            own_goal_ids(&db, &m.in_o1(), m.user).await?,
            BTreeSet::from([m.goals[0]])
        );
        Ok::<(), Error>(())
    })
    .await
}

/// M1-07, M1-08: a SuperAdmin with no roles, or a member of both orgs, sees both.
#[tokio::test]
async fn a_super_admin_or_a_member_of_both_orgs_sees_every_own_goal() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let m = seed_lapsed_membership(&db, Utc::now().naive_utc()).await;
        for viewer in [m.super_admin(), m.in_both()] {
            assert_eq!(
                own_goal_ids(&db, &viewer, m.user).await?,
                BTreeSet::from(m.goals),
                "{viewer:?}"
            );
        }
        Ok::<(), Error>(())
    })
    .await
}

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
        .single()
        .expect("T0 is a valid instant")
}

fn t1() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0)
        .single()
        .expect("T1 is a valid instant")
}

/// Seeds a goal with the given status and timestamps, moves it to `next` through
/// `update_status`, and reads the row back.
async fn after_status_change(
    status: Status,
    status_changed_at: Option<DateTime<Utc>>,
    completed_at: Option<DateTime<Utc>>,
    next: Status,
) -> Result<Model, Error> {
    let db = database().await;
    let (session_id, user_id) = seed_coaching_session(&db).await;
    let session = coaching_sessions::Entity::find_by_id(session_id)
        .one(&db)
        .await
        .expect("the session is read")
        .expect("the session exists");
    let goal = goals::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_relationship_id: Set(session.coaching_relationship_id),
        created_in_session_id: Set(None),
        user_id: Set(user_id),
        title: Set(None),
        body: Set(None),
        status: Set(status),
        status_changed_at: Set(status_changed_at.map(Into::into)),
        completed_at: Set(completed_at.map(Into::into)),
        target_date: Set(None),
        created_at: Set(t0().into()),
        updated_at: Set(t0().into()),
    }
    .insert(&db)
    .await
    .expect("the goal is seeded");

    update_status(&db, &EventPublisher::new(), goal.id, next).await?;
    find_by_id(&db, goal.id).await.map_err(Error::from)
}

/// GS-01
#[tokio::test]
async fn completing_a_goal_stamps_both_timestamps() -> Result<(), Error> {
    within_time_limit(async {
        let goal =
            after_status_change(Status::InProgress, Some(t0()), None, Status::Completed).await?;
        assert!(goal.status_changed_at.expect("status_changed_at is set") > t0());
        assert!(goal.completed_at.is_some());
        Ok::<(), Error>(())
    })
    .await
}

/// GS-02
#[tokio::test]
async fn reopening_a_goal_clears_completed_at() -> Result<(), Error> {
    within_time_limit(async {
        let goal = after_status_change(
            Status::Completed,
            Some(t0()),
            Some(t1()),
            Status::InProgress,
        )
        .await?;
        assert_eq!(goal.completed_at, None);
        assert!(goal.status_changed_at.expect("status_changed_at is set") > t0());
        Ok::<(), Error>(())
    })
    .await
}

/// GS-03
#[tokio::test]
async fn keeping_in_progress_leaves_the_timestamps_alone() -> Result<(), Error> {
    within_time_limit(async {
        let goal =
            after_status_change(Status::InProgress, Some(t0()), None, Status::InProgress).await?;
        assert_eq!(goal.status_changed_at, Some(t0().into()));
        assert_eq!(goal.completed_at, None);
        assert!(goal.updated_at > t0());
        Ok::<(), Error>(())
    })
    .await
}

/// GS-04
#[tokio::test]
async fn keeping_completed_leaves_the_timestamps_alone() -> Result<(), Error> {
    within_time_limit(async {
        let goal =
            after_status_change(Status::Completed, Some(t0()), Some(t1()), Status::Completed)
                .await?;
        assert_eq!(goal.status_changed_at, Some(t0().into()));
        assert_eq!(goal.completed_at, Some(t1().into()));
        Ok::<(), Error>(())
    })
    .await
}

/// GS-05: `WontDo` is not completion.
#[tokio::test]
async fn wont_do_is_not_completion() -> Result<(), Error> {
    within_time_limit(async {
        let goal =
            after_status_change(Status::InProgress, Some(t0()), None, Status::WontDo).await?;
        assert_eq!(goal.completed_at, None);
        assert!(goal.status_changed_at.expect("status_changed_at is set") > t0());
        Ok::<(), Error>(())
    })
    .await
}
