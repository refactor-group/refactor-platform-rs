//! Which sessions the batch session lookup selects, against a real SQL engine.

use std::collections::BTreeMap;

use super::*;
use crate::test_utils::sqlite::{database, seed_fixture, within_time_limit};

#[tokio::test]
async fn the_batch_lookup_returns_only_the_requested_sessions_with_their_relationships() {
    within_time_limit(async {
        let db = database().await;
        let w = seed_fixture(&db).await;

        let found = find_by_ids_with_coaching_relationship(&db, &[w.s1, w.s2])
            .await
            .expect("the lookup succeeds");

        let relationship_by_session: BTreeMap<Id, Id> = found
            .iter()
            .map(|(session, relationship)| {
                assert_eq!(session.coaching_relationship_id, relationship.id);
                (session.id, relationship.id)
            })
            .collect();
        assert_eq!(found.len(), 2);
        assert_eq!(
            relationship_by_session,
            BTreeMap::from([(w.s1, w.r1), (w.s2, w.r2)])
        );
    })
    .await;
}
