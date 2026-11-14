//! Which users an email lookup finds for each requester, against the access matrix's fixture.

use sea_orm::DatabaseConnection;

use super::{can_administer_user, lookup_by_email_scoped};

use crate::test_utils::sqlite::fixture::record_removal;
use crate::test_utils::sqlite::{
    database, mirror_migration_defaults, seed_fixture, within_time_limit, Caller, Fixture,
};
use crate::{users, Id};

/// The fixture, seeded fresh.
async fn seeded() -> (DatabaseConnection, Fixture) {
    let db = database().await;
    mirror_migration_defaults(&db).await;
    let fixture = seed_fixture(&db).await;
    (db, fixture)
}

/// `requester`'s user row with its roles, as the signed-in user is loaded.
async fn requester(db: &DatabaseConnection, fixture: &Fixture, caller: Caller) -> users::Model {
    entity_api::user::find_by_id(db, fixture.user(caller))
        .await
        .expect("the requester reads")
}

/// The ids `requester` finds looking up `target`'s email.
async fn found(
    db: &DatabaseConnection,
    fixture: &Fixture,
    requester_caller: Caller,
    target: Caller,
) -> Vec<Id> {
    let requester = requester(db, fixture, requester_caller).await;
    lookup_by_email_scoped(db, &requester, &target.email())
        .await
        .expect("the lookup runs")
        .into_iter()
        .map(|user| user.id)
        .collect()
}

/// USR-07
#[tokio::test]
async fn an_admin_finds_a_member_of_their_org() {
    within_time_limit(async {
        let (db, fixture) = seeded().await;

        assert_eq!(
            found(&db, &fixture, Caller::Admin, Caller::Member).await,
            vec![fixture.user(Caller::Member)]
        );
    })
    .await;
}

/// USR-08
#[tokio::test]
async fn an_admin_cannot_find_a_member_of_another_org_even_with_history_there() {
    within_time_limit(async {
        let (db, fixture) = seeded().await;
        let outsider = fixture.user(Caller::Outsider);

        assert_eq!(
            found(&db, &fixture, Caller::Admin, Caller::Outsider).await,
            Vec::<Id>::new()
        );
        record_removal(&db, outsider, fixture.o2).await;
        assert_eq!(
            found(&db, &fixture, Caller::Admin, Caller::Outsider).await,
            Vec::<Id>::new()
        );
    })
    .await;
}

/// USR-09
#[tokio::test]
async fn an_admin_finds_and_may_administer_a_former_member_of_their_org() {
    within_time_limit(async {
        let (db, fixture) = seeded().await;
        let removed = fixture.user(Caller::Removed);
        let admin = requester(&db, &fixture, Caller::Admin).await;

        assert_eq!(
            found(&db, &fixture, Caller::Admin, Caller::Removed).await,
            Vec::<Id>::new(),
            "no history yet"
        );
        record_removal(&db, removed, fixture.o1).await;

        assert_eq!(
            found(&db, &fixture, Caller::Admin, Caller::Removed).await,
            vec![removed]
        );
        assert!(can_administer_user(&db, &admin, removed)
            .await
            .expect("the check runs"));
    })
    .await;
}

/// USR-10
#[tokio::test]
async fn a_super_admin_finds_anyone() {
    within_time_limit(async {
        let (db, fixture) = seeded().await;

        for target in Caller::ALL {
            assert_eq!(
                found(&db, &fixture, Caller::Super, target).await,
                vec![fixture.user(target)],
                "{target:?}"
            );
        }
    })
    .await;
}
