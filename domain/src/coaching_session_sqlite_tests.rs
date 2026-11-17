//! Which sessions the nearest window, the prior/next lookups and a user's own session
//! lists select, and who a created session records as its scheduler and link creator,
//! against a real SQL engine.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{NaiveDate, Utc};
use entity::meeting_provider::Provider;
use entity::{coaching_relationships, coaching_sessions, oauth_connections, organizations, users};
use meeting_auth::oauth::token::encryption;
use mockito::{Matcher, Mock, Server, ServerGuard};
use sea_orm::{ActiveModelTrait, ActiveValue::Unchanged, DatabaseConnection, EntityTrait, Set};

use super::*;
use crate::coaching_relationship::Membership;
use crate::coaching_relationships::Visibility;
use crate::test_utils::sqlite::fixture::{make_colleagues, set_google_meeting_url};
use crate::test_utils::sqlite::{
    coach_of, database, mirror_migration_defaults, organization_of, seed_lapsed_membership,
    seed_membership, seed_organization, seed_user, within_time_limit, LapsedMembership,
};

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
    organization_id: Id,
    coach_id: Id,
    coachee_id: Id,
}

async fn seed_relationship(db: &DatabaseConnection) -> Relationship {
    let coach_id = seed_user(db).await;
    seed_relationship_coached_by(db, coach_id).await
}

/// A relationship with a new coachee, coached by `coach_id`.
async fn seed_relationship_coached_by(db: &DatabaseConnection, coach_id: Id) -> Relationship {
    let (id, coachee_id) = (Id::new_v4(), seed_user(db).await);
    let organization_id = seed_organization(db).await;
    let now = Utc::now();
    coaching_relationships::ActiveModel {
        id: Set(id),
        organization_id: Set(organization_id),
        coach_id: Set(coach_id),
        coachee_id: Set(coachee_id),
        kind: Set(crate::coaching_relationship_kind::Kind::Coaching),
        slug: Set(format!("relationship-{id}")),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the relationship is seeded");
    Relationship {
        id,
        organization_id,
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
        scheduled_by_user_id: Set(coach_of(db, coaching_relationship_id).await),
        meeting_url_created_by_user_id: Set(None),
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
    window_over(&[coaching_relationship_id], before, after)
}

fn window_over(coaching_relationship_ids: &[Id], before: u64, after: u64) -> NearestWindow {
    NearestWindow {
        coaching_relationship_ids: coaching_relationship_ids.to_vec(),
        around: around(),
        before,
        after,
    }
}

/// `user_id` as a SuperAdmin, so only participation decides; membership has its own tests.
fn participant(user_id: Id) -> Visibility {
    Visibility {
        user_id,
        is_super_admin: true,
        admin_org_ids: vec![],
        member_org_ids: vec![],
    }
}

async fn nearest_ids(
    db: &DatabaseConnection,
    user_id: Id,
    window: NearestWindow,
) -> Result<Vec<Id>, Error> {
    Ok(find_nearest_by_user_with_includes(
        db,
        &participant(user_id),
        window,
        IncludeOptions::default(),
    )
    .await?
    .into_iter()
    .map(|s| s.session.id)
    .collect())
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

/// With several relationships, each gets its own `before` and `after`, and all of them
/// come back in one list, latest first.
#[tokio::test]
async fn each_relationship_keeps_its_own_window() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let first = seed_relationship(&db).await;
        let second = seed_relationship_coached_by(&db, first.coach_id).await;
        let mut sessions = BTreeMap::new();
        for (rel, hour) in [(&first, 0), (&second, 1)] {
            for day in [-2, -1, 1, 2] {
                let date = days_from_around(day) + TimeDelta::hours(hour);
                sessions.insert((rel.id, day), seed_session(&db, rel.id, date).await);
            }
        }

        let ids = nearest_ids(
            &db,
            first.coach_id,
            window_over(&[first.id, second.id], 1, 1),
        )
        .await?;

        assert_eq!(
            ids,
            [
                sessions[&(second.id, 1)],
                sessions[&(first.id, 1)],
                sessions[&(second.id, -1)],
                sessions[&(first.id, -1)],
            ]
        );
        Ok::<(), Error>(())
    })
    .await
}

/// A relationship the caller isn't part of, or one that doesn't exist, adds nothing,
/// and the caller's own relationships still come back.
#[tokio::test]
async fn relationships_the_caller_is_not_in_add_nothing() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let mine = seed_relationship(&db).await;
        let mine_session = seed_session(&db, mine.id, days_from_around(1)).await;
        let theirs = seed_relationship(&db).await;
        seed_session(&db, theirs.id, days_from_around(1)).await;

        assert_eq!(
            nearest_ids(
                &db,
                mine.coach_id,
                window_over(&[theirs.id, mine.id, Id::new_v4()], 2, 2),
            )
            .await?,
            [mine_session]
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

        let sessions = find_nearest_by_user_with_includes(
            &db,
            &participant(rel.coachee_id),
            window(rel.id, 2, 2),
            includes,
        )
        .await?;

        let [session] = sessions.as_slice() else {
            panic!("expected one session, got {sessions:?}");
        };
        assert_eq!(
            session.relationship.as_ref().map(|r| r.relationship.id),
            Some(rel.id)
        );
        assert_eq!(session.coach.as_ref().map(|u| u.id), Some(rel.coach_id));
        assert_eq!(session.coachee.as_ref().map(|u| u.id), Some(rel.coachee_id));
        Ok::<(), Error>(())
    })
    .await
}

/// M5-02: the own list's included relationship marks a removed coachee.
#[tokio::test]
async fn m5_02_an_included_relationship_marks_a_removed_coachee() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let rel = seed_relationship(&db).await;
        let session = seed_session(&db, rel.id, around()).await;
        seed_membership(&db, rel.coach_id, organization_of(&db, session).await).await;

        let sessions = find_by_user_with_includes(
            &db,
            &participant(rel.coach_id),
            SessionQueryOptions::default(),
            IncludeOptions {
                relationship: true,
                ..Default::default()
            },
        )
        .await?;

        let [enriched] = sessions.as_slice() else {
            panic!("expected one session, got {sessions:?}");
        };
        assert_eq!(
            enriched.relationship.as_ref().map(|r| r.membership),
            Some(Membership {
                coach_is_member: true,
                coachee_is_member: false,
            })
        );
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

/// The ids of U's sessions `viewer` lists.
async fn own_session_ids(
    db: &DatabaseConnection,
    viewer: &Visibility,
) -> Result<BTreeSet<Id>, Error> {
    Ok(find_by_user_with_includes(
        db,
        viewer,
        SessionQueryOptions::default(),
        IncludeOptions::default(),
    )
    .await?
    .into_iter()
    .map(|s| s.session.id)
    .collect())
}

/// The ids of U's sessions `viewer` reads in a window over both relationships.
async fn own_nearest_ids(
    db: &DatabaseConnection,
    m: &LapsedMembership,
    viewer: &Visibility,
) -> Result<BTreeSet<Id>, Error> {
    Ok(find_nearest_by_user_with_includes(
        db,
        viewer,
        window_over(&m.relationships, 5, 5),
        IncludeOptions::default(),
    )
    .await?
    .into_iter()
    .map(|s| s.session.id)
    .collect())
}

/// M1-02
#[tokio::test]
async fn a_users_own_sessions_skip_an_org_they_left() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let m = seed_lapsed_membership(&db, around()).await;
        assert_eq!(
            own_session_ids(&db, &m.in_o1()).await?,
            BTreeSet::from([m.sessions[0]])
        );
        Ok::<(), Error>(())
    })
    .await
}

/// M1-03: both relationships requested, only R1's session comes back.
#[tokio::test]
async fn a_users_own_nearest_sessions_skip_an_org_they_left() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let m = seed_lapsed_membership(&db, around()).await;
        assert_eq!(
            own_nearest_ids(&db, &m, &m.in_o1()).await?,
            BTreeSet::from([m.sessions[0]])
        );
        Ok::<(), Error>(())
    })
    .await
}

/// M1-07, M1-08: a SuperAdmin with no roles, or a member of both orgs, sees both.
#[tokio::test]
async fn a_super_admin_or_a_member_of_both_orgs_sees_every_own_session() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let m = seed_lapsed_membership(&db, around()).await;
        let both = BTreeSet::from(m.sessions);
        for viewer in [m.super_admin(), m.in_both()] {
            assert_eq!(
                own_session_ids(&db, &viewer).await?,
                both,
                "list {viewer:?}"
            );
            assert_eq!(
                own_nearest_ids(&db, &m, &viewer).await?,
                both,
                "nearest {viewer:?}"
            );
        }
        Ok::<(), Error>(())
    })
    .await
}

/// A database with the migrations' defaults, and a relationship whose coach and coachee
/// both hold a role in its organization.
async fn scheduling_database() -> (DatabaseConnection, Relationship) {
    let db = database().await;
    mirror_migration_defaults(&db).await;
    let rel = seed_relationship(&db).await;
    for user_id in [rel.coach_id, rel.coachee_id] {
        seed_membership(&db, user_id, rel.organization_id).await;
    }
    (db, rel)
}

/// A Tiptap stand-in that accepts every new document, and the config that points at it.
async fn documents() -> (ServerGuard, Mock, Config) {
    let mut server = Server::new_async().await;
    let mock = server
        .mock("POST", Matcher::Regex("^/api/documents/".to_string()))
        .with_status(200)
        .create_async()
        .await;
    let config = Config::from_args([
        "test",
        "--tiptap-auth-key=test-auth-key",
        &format!("--tiptap-url={}", server.url()),
    ]);
    (server, mock, config)
}

/// A new session on `rel` as a create request carries it.
fn new_session(rel: &Relationship, provider: Option<Provider>) -> coaching_sessions::Model {
    let now = Utc::now();
    coaching_sessions::Model {
        id: Id::nil(),
        coaching_relationship_id: rel.id,
        coaching_session_series_id: None,
        ical_sequence: 0,
        ical_recurrence_id: None,
        collab_document_name: None,
        date: days_from_around(1),
        duration_minutes: 60,
        title: None,
        meeting_url: None,
        provider,
        created_at: now.into(),
        updated_at: now.into(),
        hydrated_at: None,
        scheduled_by_user_id: Id::nil(),
        meeting_url_created_by_user_id: None,
    }
}

async fn create_as(
    db: &DatabaseConnection,
    config: &Config,
    session: coaching_sessions::Model,
    acting_user_id: Id,
) -> coaching_sessions::Model {
    let created = create(
        db,
        config,
        &EventPublisher::new(),
        session,
        acting_user_id,
        None,
    )
    .await
    .expect("the session is created");
    stored(db, created.id).await
}

async fn stored(db: &DatabaseConnection, id: Id) -> coaching_sessions::Model {
    coaching_sessions::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the session reads")
        .expect("the session exists")
}

/// The scheduler is whoever made the request, never the coach by default nor a value the
/// caller put on the model.
#[tokio::test]
async fn create_records_the_acting_user_as_scheduler() {
    within_time_limit(async {
        let (db, rel) = scheduling_database().await;
        let (_server, _documents, config) = documents().await;
        let acting_user = seed_user(&db).await;
        let session = coaching_sessions::Model {
            scheduled_by_user_id: seed_user(&db).await,
            ..new_session(&rel, None)
        };

        let created = create_as(&db, &config, session, acting_user).await;

        assert_eq!(created.scheduled_by_user_id, acting_user);
    })
    .await;
}

/// Both participants' notice rows are stamped at the session's `created_at`.
#[tokio::test]
async fn create_stamps_both_participants_notice_at_the_sessions_creation() {
    within_time_limit(async {
        let (db, rel) = scheduling_database().await;
        let (_server, _documents, config) = documents().await;

        let created = create_as(&db, &config, new_session(&rel, None), rel.coach_id).await;

        let notices: BTreeMap<Id, _> =
            entity_api::coaching_session_notice::find_by_coaching_session(&db, created.id)
                .await
                .expect("the notices read")
                .into_iter()
                .map(|row| (row.user_id, row.notice_given_at))
                .collect();
        let at = created.created_at;
        assert_eq!(
            notices,
            BTreeMap::from([(rel.coach_id, at), (rel.coachee_id, at)])
        );
    })
    .await;
}

/// Every seeded session holds a notice row for its coach and its coachee, so none is reminded.
#[tokio::test]
async fn seeded_sessions_notice_both_participants() {
    within_time_limit(async {
        let db = database().await;
        mirror_migration_defaults(&db).await;
        // The initial-admin migration creates this organization; the seed reuses it.
        let now = Utc::now();
        organizations::ActiveModel {
            name: Set("Refactor Group".to_string()),
            slug: Set("refactor-group".to_string()),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
            ..Default::default()
        }
        .insert(&db)
        .await
        .expect("the organization is seeded");

        entity_api::seed_database(&db).await;

        let relationships: BTreeMap<Id, coaching_relationships::Model> =
            coaching_relationships::Entity::find()
                .all(&db)
                .await
                .expect("the relationships read")
                .into_iter()
                .map(|relationship| (relationship.id, relationship))
                .collect();
        let sessions = coaching_sessions::Entity::find()
            .all(&db)
            .await
            .expect("the sessions read");
        assert!(!sessions.is_empty());
        for session in sessions {
            let relationship = &relationships[&session.coaching_relationship_id];
            let noticed: BTreeSet<Id> =
                entity_api::coaching_session_notice::find_by_coaching_session(&db, session.id)
                    .await
                    .expect("the notices read")
                    .into_iter()
                    .map(|row| row.user_id)
                    .collect();
            assert_eq!(
                noticed,
                BTreeSet::from([relationship.coach_id, relationship.coachee_id]),
                "session {}",
                session.id
            );
        }
    })
    .await;
}

/// A reused link whose earlier session recorded no creator is credited to the coach.
#[tokio::test]
async fn create_reusing_a_meeting_link_with_no_recorded_creator_records_the_coach() {
    within_time_limit(async {
        let (db, rel) = scheduling_database().await;
        let (_server, _documents, config) = documents().await;
        let earlier = seed_session(&db, rel.id, days_from_around(-7)).await;
        set_google_meeting_url(&db, earlier, "https://meet.google.com/abc-defg-hij").await;

        let created = create_as(
            &db,
            &config,
            new_session(&rel, Some(Provider::Google)),
            rel.coachee_id,
        )
        .await;

        assert_eq!(
            created.meeting_url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(created.meeting_url_created_by_user_id, Some(rel.coach_id));
    })
    .await;
}

/// A reused link keeps the creator recorded on the session it came from.
#[tokio::test]
async fn create_reusing_a_meeting_link_keeps_its_original_creator() {
    within_time_limit(async {
        let (db, rel) = scheduling_database().await;
        let (_server, _documents, config) = documents().await;
        let link_creator = seed_user(&db).await;
        let earlier = seed_session(&db, rel.id, days_from_around(-7)).await;
        set_google_meeting_url(&db, earlier, "https://meet.google.com/abc-defg-hij").await;
        coaching_sessions::ActiveModel {
            id: Unchanged(earlier),
            meeting_url_created_by_user_id: Set(Some(link_creator)),
            ..Default::default()
        }
        .update(&db)
        .await
        .expect("the link creator is seeded");

        let created = create_as(
            &db,
            &config,
            new_session(&rel, Some(Provider::Google)),
            rel.coach_id,
        )
        .await;

        assert_eq!(
            created.meeting_url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(created.meeting_url_created_by_user_id, Some(link_creator));
    })
    .await;
}

/// No provider, no link, so no link creator, whatever the caller put on the model.
#[tokio::test]
async fn create_without_a_provider_records_no_link_creator() {
    within_time_limit(async {
        let (db, rel) = scheduling_database().await;
        let (_server, _documents, config) = documents().await;
        let session = coaching_sessions::Model {
            meeting_url_created_by_user_id: Some(rel.coach_id),
            ..new_session(&rel, None)
        };

        let created = create_as(&db, &config, session, rel.coach_id).await;

        assert_eq!(created.meeting_url, None);
        assert_eq!(created.meeting_url_created_by_user_id, None);
    })
    .await;
}

/// A link the request carries was set by hand, by whoever made the request.
#[tokio::test]
async fn create_with_a_pasted_meeting_link_records_the_acting_user_as_its_creator() {
    within_time_limit(async {
        let (db, rel) = scheduling_database().await;
        let (_server, _documents, config) = documents().await;
        let session = coaching_sessions::Model {
            meeting_url: Some("https://meet.example/pasted".to_string()),
            ..new_session(&rel, None)
        };

        let created = create_as(&db, &config, session, rel.coachee_id).await;

        assert_eq!(created.meeting_url_created_by_user_id, Some(rel.coachee_id));
    })
    .await;
}

/// Hydration stores the link's creator beside the link, and leaves the scheduler alone.
#[tokio::test]
async fn mark_hydrated_stores_the_link_creator_and_keeps_the_scheduler() {
    within_time_limit(async {
        let db = database().await;
        let rel = seed_relationship(&db).await;
        let scheduler = seed_user(&db).await;
        let id = seed_session(&db, rel.id, days_from_around(1)).await;
        coaching_sessions::ActiveModel {
            id: Unchanged(id),
            scheduled_by_user_id: Set(scheduler),
            ..Default::default()
        }
        .update(&db)
        .await
        .expect("the scheduler is seeded");
        let target = coaching_sessions::Model {
            meeting_url: Some("https://meet.google.com/abc-defg-hij".to_string()),
            provider: Some(Provider::Google),
            meeting_url_created_by_user_id: Some(rel.coach_id),
            scheduled_by_user_id: seed_user(&db).await,
            ..stored(&db, id).await
        };

        coaching_session::mark_hydrated(&db, &target)
            .await
            .expect("the session hydrates");

        let hydrated = stored(&db, id).await;
        assert_eq!(hydrated.meeting_url, target.meeting_url);
        assert_eq!(hydrated.meeting_url_created_by_user_id, Some(rel.coach_id));
        assert_eq!(hydrated.scheduled_by_user_id, scheduler);
    })
    .await;
}

/// SCHED-17: a colleague session created with no duration takes the default of the colleague
/// who scheduled it, not the coach seat's.
#[tokio::test]
async fn a_colleague_session_takes_the_schedulers_default_duration() {
    within_time_limit(async {
        let (db, rel) = scheduling_database().await;
        make_colleagues(&db, rel.id).await;
        for (user_id, minutes) in [(rel.coach_id, 60), (rel.coachee_id, 25)] {
            users::ActiveModel {
                id: Set(user_id),
                default_coaching_session_duration_minutes: Set(minutes),
                ..Default::default()
            }
            .update(&db)
            .await
            .expect("the default duration is set");
        }
        let (_server, _documents, config) = documents().await;

        let created = create_as(&db, &config, new_session(&rel, None), rel.coachee_id).await;

        assert_eq!(created.duration_minutes, 25);
    })
    .await;
}

/// 32 bytes hex, the shape `encryption::encrypt` expects.
const ENCRYPTION_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A Google connection for `user_id` whose unexpiring access token is `token`.
async fn connect_google(db: &DatabaseConnection, user_id: Id, token: &str) {
    let now = Utc::now();
    oauth_connections::ActiveModel {
        id: Set(Id::new_v4()),
        user_id: Set(user_id),
        provider: Set(Provider::Google),
        external_account_id: Set(None),
        external_email: Set(None),
        access_token: Set(encryption::encrypt(token, ENCRYPTION_KEY).expect("the token encrypts")),
        refresh_token: Set(None),
        token_expires_at: Set(None),
        token_type: Set("Bearer".to_string()),
        scopes: Set(String::new()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the connection is seeded");
}

const MEET_LINK: &str = "https://meet.google.com/new-link-abc";

/// A Tiptap and Google Meet stand-in, where a space is made only with the access token
/// `token`, and the config that points at it.
async fn documents_and_meet(token: &str) -> (ServerGuard, [Mock; 2], Config) {
    let mut server = Server::new_async().await;
    let documents = server
        .mock("POST", Matcher::Regex("^/api/documents/".to_string()))
        .with_status(200)
        .create_async()
        .await;
    let spaces = server
        .mock("POST", "/spaces")
        .match_header("authorization", format!("Bearer {token}").as_str())
        .with_status(200)
        .with_body(format!(
            r#"{{"name":"spaces/abc","meetingUri":"{MEET_LINK}","meetingCode":"new-link-abc"}}"#
        ))
        .create_async()
        .await;
    let config = Config::from_args([
        "test".to_string(),
        "--tiptap-auth-key=test-auth-key".to_string(),
        format!("--tiptap-url={}", server.url()),
        format!("--google-meet-api-url={}", server.url()),
        format!("--encryption-key={ENCRYPTION_KEY}"),
        "--google-client-id=client-id".to_string(),
        "--google-client-secret=client-secret".to_string(),
        "--google-redirect-uri=https://app.example.com/oauth/google/callback".to_string(),
    ]);
    (server, [documents, spaces], config)
}

/// SCHED-18: a colleague whose scheduler has no connected account gets a link made with the
/// other colleague's, which is recorded as its creator.
#[tokio::test]
async fn a_colleague_session_links_with_the_other_colleagues_account_when_the_scheduler_has_none() {
    within_time_limit(async {
        let (db, rel) = scheduling_database().await;
        make_colleagues(&db, rel.id).await;
        connect_google(&db, rel.coach_id, "other-token").await;
        let (_server, [_documents, spaces], config) = documents_and_meet("other-token").await;

        let created = create_as(
            &db,
            &config,
            new_session(&rel, Some(Provider::Google)),
            rel.coachee_id,
        )
        .await;

        spaces.assert_async().await;
        assert_eq!(created.meeting_url.as_deref(), Some(MEET_LINK));
        assert_eq!(created.meeting_url_created_by_user_id, Some(rel.coach_id));
    })
    .await;
}

/// SCHED-18: colleagues with no connected account between them get no link.
#[tokio::test]
async fn a_colleague_session_gets_no_link_when_neither_colleague_is_connected() {
    within_time_limit(async {
        let (db, rel) = scheduling_database().await;
        make_colleagues(&db, rel.id).await;
        let (_server, [_documents, spaces], config) = documents_and_meet("any-token").await;

        let created = create_as(
            &db,
            &config,
            new_session(&rel, Some(Provider::Google)),
            rel.coachee_id,
        )
        .await;

        spaces.expect(0).assert_async().await;
        assert_eq!(created.meeting_url, None);
        assert_eq!(created.meeting_url_created_by_user_id, None);
    })
    .await;
}

/// SCHED-19: a coaching session's link is only ever made with the coach's account, so a
/// coach with none gets no link even when the coachee has one.
#[tokio::test]
async fn a_coaching_session_never_links_with_the_coachees_account() {
    within_time_limit(async {
        let (db, rel) = scheduling_database().await;
        connect_google(&db, rel.coachee_id, "coachee-token").await;
        let (_server, [_documents, spaces], config) = documents_and_meet("coachee-token").await;

        let created = create_as(
            &db,
            &config,
            new_session(&rel, Some(Provider::Google)),
            rel.coach_id,
        )
        .await;

        spaces.expect(0).assert_async().await;
        assert_eq!(created.meeting_url, None);
        assert_eq!(created.meeting_url_created_by_user_id, None);
    })
    .await;
}

/// SCHED-22: a colleague session requested with no provider by a scheduler with no connection
/// gets a link from the other colleague's, with that connection's provider.
#[tokio::test]
async fn a_colleague_session_with_no_provider_links_with_the_other_colleagues_connection() {
    within_time_limit(async {
        let (db, rel) = scheduling_database().await;
        make_colleagues(&db, rel.id).await;
        connect_google(&db, rel.coach_id, "other-token").await;
        let (_server, [_documents, spaces], config) = documents_and_meet("other-token").await;

        let created = create_as(&db, &config, new_session(&rel, None), rel.coachee_id).await;

        spaces.assert_async().await;
        assert_eq!(created.meeting_url.as_deref(), Some(MEET_LINK));
        assert_eq!(created.provider, Some(Provider::Google));
        assert_eq!(created.meeting_url_created_by_user_id, Some(rel.coach_id));
    })
    .await;
}

/// SCHED-22: a connected scheduler who asks for no provider gets no link, whatever the other
/// colleague has.
#[tokio::test]
async fn a_connected_colleague_who_asks_for_no_provider_gets_no_link() {
    within_time_limit(async {
        let (db, rel) = scheduling_database().await;
        make_colleagues(&db, rel.id).await;
        connect_google(&db, rel.coachee_id, "scheduler-token").await;
        connect_google(&db, rel.coach_id, "other-token").await;
        let (_server, [_documents, spaces], config) = documents_and_meet("scheduler-token").await;

        let created = create_as(&db, &config, new_session(&rel, None), rel.coachee_id).await;

        spaces.expect(0).assert_async().await;
        assert_eq!(created.meeting_url, None);
        assert_eq!(created.provider, None);
        assert_eq!(created.meeting_url_created_by_user_id, None);
    })
    .await;
}

/// SCHED-22: a coaching session asked for with no provider never takes one from the coachee.
#[tokio::test]
async fn a_coaching_session_with_no_provider_never_links_with_the_coachees_connection() {
    within_time_limit(async {
        let (db, rel) = scheduling_database().await;
        connect_google(&db, rel.coachee_id, "coachee-token").await;
        let (_server, [_documents, spaces], config) = documents_and_meet("coachee-token").await;

        let created = create_as(&db, &config, new_session(&rel, None), rel.coach_id).await;

        spaces.expect(0).assert_async().await;
        assert_eq!(created.meeting_url, None);
        assert_eq!(created.provider, None);
        assert_eq!(created.meeting_url_created_by_user_id, None);
    })
    .await;
}
