//! Exercises the audit log's chain against a real SQL engine: what `append` stores, and what
//! `verify_chain` finds after a stored event is edited, removed or spliced out.

use entity::audit_events::{Action, Column, Entity, Model};
use entity::Id;
use entity_api::audit_event::append;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, TransactionTrait,
};
use serde_json::json;

use super::{verify_chain, ChainCheck, DatabaseRecorder, NewEvent, Recorder};

use crate::test_utils::sqlite::{database, seed_organization, within_time_limit};
use crate::Actor;

/// A kind switch in `organization_id` by a fresh actor.
fn kind_switch(organization_id: Id) -> NewEvent {
    NewEvent {
        organization_id,
        actor: Some(Actor::new(Id::new_v4())),
        action: Action::RelationshipKindChanged,
        subject_id: Id::new_v4(),
        details: json!({ "from": "coaching", "to": "colleague" }),
    }
}

/// Appends `count` events to `organization_id`, each in its own committed transaction.
async fn append_committed(
    db: &DatabaseConnection,
    organization_id: Id,
    count: usize,
) -> Vec<Model> {
    let mut events = Vec::with_capacity(count);
    for _ in 0..count {
        let txn = db.begin().await.expect("a transaction begins");
        events.push(
            append(&txn, kind_switch(organization_id))
                .await
                .expect("the event appends"),
        );
        txn.commit().await.expect("the transaction commits");
    }
    events
}

/// How many events are stored for `organization_id`.
async fn stored(db: &DatabaseConnection, organization_id: Id) -> u64 {
    Entity::find()
        .filter(Column::OrganizationId.eq(organization_id))
        .count(db)
        .await
        .expect("the events count")
}

#[tokio::test]
async fn each_organization_chains_its_own_events() {
    within_time_limit(async {
        let db = database().await;
        let (org_a, org_b) = (seed_organization(&db).await, seed_organization(&db).await);

        // Interleaved so a chain that ignored the organization would link across them.
        let mut a = append_committed(&db, org_a, 1).await;
        let mut b = append_committed(&db, org_b, 1).await;
        a.extend(append_committed(&db, org_a, 1).await);
        b.extend(append_committed(&db, org_b, 1).await);
        a.extend(append_committed(&db, org_a, 1).await);

        for chain in [&a, &b] {
            assert_eq!(chain[0].prev_hash, None);
            for pair in chain.windows(2) {
                assert_eq!(pair[1].prev_hash.as_ref(), Some(&pair[0].hash));
            }
        }
        assert_eq!(
            verify_chain(&db, org_a).await.expect("the chain reads"),
            ChainCheck::Intact { count: 3 }
        );
        assert_eq!(
            verify_chain(&db, org_b).await.expect("the chain reads"),
            ChainCheck::Intact { count: 2 }
        );
    })
    .await;
}

#[tokio::test]
async fn an_edited_event_breaks_the_chain_at_that_event() {
    within_time_limit(async {
        let db = database().await;
        let org = seed_organization(&db).await;
        let events = append_committed(&db, org, 3).await;

        Entity::update_many()
            .col_expr(
                Column::Details,
                Expr::value(json!({ "from": "coaching", "to": "coaching" })),
            )
            .filter(Column::Id.eq(events[1].id))
            .exec(&db)
            .await
            .expect("the stored event is edited");

        assert_eq!(
            verify_chain(&db, org).await.expect("the chain reads"),
            ChainCheck::BrokenAt {
                event_id: events[1].id
            }
        );
    })
    .await;
}

#[tokio::test]
async fn a_removed_event_breaks_the_chain_at_the_next_event() {
    within_time_limit(async {
        let db = database().await;
        let org = seed_organization(&db).await;
        let events = append_committed(&db, org, 3).await;

        Entity::delete_by_id(events[1].id)
            .exec(&db)
            .await
            .expect("the stored event is removed");

        assert_eq!(
            verify_chain(&db, org).await.expect("the chain reads"),
            ChainCheck::BrokenAt {
                event_id: events[2].id
            }
        );
    })
    .await;
}

#[tokio::test]
async fn a_removed_event_breaks_the_chain_even_when_the_next_link_is_rewritten() {
    within_time_limit(async {
        let db = database().await;
        let org = seed_organization(&db).await;
        let events = append_committed(&db, org, 3).await;

        Entity::delete_by_id(events[1].id)
            .exec(&db)
            .await
            .expect("the stored event is removed");
        Entity::update_many()
            .col_expr(Column::PrevHash, Expr::value(events[0].hash.clone()))
            .filter(Column::Id.eq(events[2].id))
            .exec(&db)
            .await
            .expect("the next event is relinked");

        assert_eq!(
            verify_chain(&db, org).await.expect("the chain reads"),
            ChainCheck::BrokenAt {
                event_id: events[2].id
            }
        );
    })
    .await;
}

#[tokio::test]
async fn details_hash_the_same_whatever_order_their_keys_are_stored_in() {
    within_time_limit(async {
        let db = database().await;
        let org = seed_organization(&db).await;
        let events = append_committed(&db, org, 2).await;

        // Postgres `jsonb` reorders keys on storage; here the stored text puts `to` first.
        Entity::update_many()
            .col_expr(
                Column::Details,
                Expr::value(r#"{"to":"colleague","from":"coaching"}"#),
            )
            .filter(Column::Id.eq(events[0].id))
            .exec(&db)
            .await
            .expect("the stored details are reordered");

        assert_eq!(
            verify_chain(&db, org).await.expect("the chain reads"),
            ChainCheck::Intact { count: 2 }
        );
    })
    .await;
}

#[tokio::test]
async fn an_event_recorded_in_a_rolled_back_transaction_is_not_stored() {
    within_time_limit(async {
        let db = database().await;
        let org = seed_organization(&db).await;

        let committed = db.begin().await.expect("a transaction begins");
        DatabaseRecorder
            .record(&committed, kind_switch(org))
            .await
            .expect("the event records");
        committed.commit().await.expect("the transaction commits");

        let rolled_back = db.begin().await.expect("a transaction begins");
        DatabaseRecorder
            .record(&rolled_back, kind_switch(org))
            .await
            .expect("the event records");
        rolled_back
            .rollback()
            .await
            .expect("the transaction rolls back");

        assert_eq!(stored(&db, org).await, 1);
        assert_eq!(
            verify_chain(&db, org).await.expect("the chain reads"),
            ChainCheck::Intact { count: 1 }
        );
    })
    .await;
}
