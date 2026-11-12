//! A user's own goals read against a real SQL engine: only those in relationships the
//! user may still access.

use std::collections::BTreeSet;

use chrono::Utc;
use sea_orm::{DatabaseConnection, Value};

use super::*;
use crate::coaching_relationships::Visibility;
use crate::test_utils::sqlite::{database, seed_lapsed_membership, within_time_limit};
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
