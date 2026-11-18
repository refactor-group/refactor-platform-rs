//! Exercises the audit log's chain against a real SQL engine: what `append` stores, and what
//! `verify_chain` finds after a stored event is edited, removed or spliced out.

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use entity::audit_events::{Action, Column, Entity, Model};
use entity::Id;
use entity_api::audit_event::append;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseConnection, DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter,
    TransactionTrait,
};
use serde_json::json;

use super::{verify_chain, ChainCheck, DatabaseRecorder, NewEvent, Recorder};

use crate::coaching_relationship::InviteeRelationship;
use crate::coaching_relationship_kind::Kind;
use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use crate::test_utils::audit_event::MemoryRecorder;
use crate::test_utils::sqlite::{
    database, mirror_migration_defaults, seed_membership, seed_organization, seed_user,
    within_time_limit,
};
use crate::user::create_in_organization;
use crate::{coaching_relationships, user_roles, users, Actor};

/// A role change in `organization_id` by a fresh actor.
fn role_change(organization_id: Id) -> NewEvent {
    NewEvent {
        organization_id,
        actor: Some(Actor::new(Id::new_v4())),
        action: Action::MemberRoleChanged,
        subject_id: Id::new_v4(),
        details: json!({ "from": "user", "to": "admin" }),
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
            append(&txn, role_change(organization_id))
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
                Expr::value(json!({ "from": "user", "to": "user" })),
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
                Expr::value(r#"{"to":"admin","from":"user"}"#),
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
            .record(&committed, role_change(org))
            .await
            .expect("the event records");
        committed.commit().await.expect("the transaction commits");

        let rolled_back = db.begin().await.expect("a transaction begins");
        DatabaseRecorder
            .record(&rolled_back, role_change(org))
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

/// Stores the first `stored` events it is given, then fails every later one.
struct FailsAfter {
    stored: usize,
    attempts: AtomicUsize,
}

#[async_trait]
impl Recorder for FailsAfter {
    async fn record(&self, txn: &DatabaseTransaction, event: NewEvent) -> Result<(), Error> {
        if self.attempts.fetch_add(1, Ordering::SeqCst) < self.stored {
            return DatabaseRecorder.record(txn, event).await;
        }
        Err(Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "the audit write fails".to_string(),
            )),
        })
    }
}

/// A new account to invite, not yet stored.
fn invitee() -> users::Model {
    let id = Id::new_v4();
    let now = chrono::Utc::now();
    users::Model {
        id,
        email: format!("{id}@example.com"),
        first_name: "New".to_string(),
        last_name: "Member".to_string(),
        display_name: None,
        password: None,
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
        working_hours_start: crate::working_hours::default_start(),
        working_hours_end: crate::working_hours::default_end(),
        created_at: now.into(),
        updated_at: now.into(),
        roles: vec![],
        invite_status: None,
    }
}

#[tokio::test]
async fn a_failed_audit_write_undoes_the_member_and_relationship_it_records() {
    within_time_limit(async {
        let db = database().await;
        mirror_migration_defaults(&db).await;
        let org = seed_organization(&db).await;
        let coach = seed_user(&db).await;
        seed_membership(&db, coach, org).await;
        let new_user = invitee();
        let recorder = FailsAfter {
            stored: 1,
            attempts: AtomicUsize::new(0),
        };

        create_in_organization(
            &db,
            &recorder,
            Actor::new(Id::new_v4()),
            org,
            new_user.clone(),
            Some(InviteeRelationship {
                with_user_id: coach,
                kind: Kind::Coaching,
                role: None,
            }),
        )
        .await
        .expect_err("the failed audit write fails the call");

        // The member's event was stored before the relationship's failed.
        assert_eq!(recorder.attempts.load(Ordering::SeqCst), 2);
        assert_eq!(
            users::Entity::find_by_id(new_user.id)
                .one(&db)
                .await
                .expect("the user reads"),
            None
        );
        assert_eq!(
            user_roles::Entity::find()
                .filter(user_roles::Column::UserId.eq(new_user.id))
                .count(&db)
                .await
                .expect("the roles count"),
            0
        );
        assert_eq!(
            coaching_relationships::Entity::find()
                .filter(coaching_relationships::Column::OrganizationId.eq(org))
                .count(&db)
                .await
                .expect("the relationships count"),
            0
        );
        assert_eq!(stored(&db, org).await, 0);
    })
    .await;
}

#[tokio::test]
async fn record_all_records_organization_by_organization_keeping_each_ones_order() {
    within_time_limit(async {
        let db = database().await;
        let (low, high) = {
            let (a, b) = (Id::new_v4(), Id::new_v4());
            (a.min(b), a.max(b))
        };
        let [first_high, low_event, second_high] = [high, low, high].map(role_change);
        let recorder = MemoryRecorder::default();

        let txn = db.begin().await.expect("a transaction begins");
        recorder
            .record_all(
                &txn,
                vec![first_high.clone(), low_event.clone(), second_high.clone()],
            )
            .await
            .expect("the events record");

        assert_eq!(recorder.events(), vec![low_event, first_high, second_high]);
    })
    .await;
}
