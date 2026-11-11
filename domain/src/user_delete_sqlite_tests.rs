//! What deleting an account leaves behind when it has, or lacks, coaching history.

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, Set,
};

use super::delete;
use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::test_utils::sqlite::{
    database, seed_membership, seed_organization, seed_user, within_time_limit,
};
use crate::{
    coaching_relationships, coaching_sessions, goals, status, user_roles, users, Actor, Id,
};

/// A database whose `user_role_changes` fills `id` and `changed_at` itself, as the
/// migration's column defaults do; schema sync leaves them without defaults.
async fn database_with_role_audit() -> DatabaseConnection {
    let db = database().await;
    for statement in [
        "DROP TABLE refactor_platform.user_role_changes",
        "CREATE TABLE refactor_platform.user_role_changes ( \
             id blob NOT NULL PRIMARY KEY DEFAULT (randomblob(16)), \
             actor_user_id blob, \
             target_user_id blob NOT NULL, \
             organization_id blob, \
             previous_role text, \
             new_role text, \
             changed_at text NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) )",
    ] {
        db.execute_unprepared(statement)
            .await
            .expect("the audit table is rebuilt");
    }
    db
}

/// A user holding a User role in a new organization.
async fn seed_member(db: &DatabaseConnection) -> (Id, Id) {
    let (user_id, organization_id) = (seed_user(db).await, seed_organization(db).await);
    seed_membership(db, user_id, organization_id).await;
    (user_id, organization_id)
}

async fn seed_relationship(
    db: &DatabaseConnection,
    organization_id: Id,
    coach_id: Id,
    coachee_id: Id,
) -> Id {
    let id = Id::new_v4();
    let now = Utc::now();
    coaching_relationships::ActiveModel {
        id: Set(id),
        organization_id: Set(organization_id),
        coach_id: Set(coach_id),
        coachee_id: Set(coachee_id),
        slug: Set(format!("relationship-{id}")),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the relationship is seeded")
    .id
}

async fn seed_session(db: &DatabaseConnection, coaching_relationship_id: Id) -> Id {
    let now = Utc::now();
    coaching_sessions::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_relationship_id: Set(coaching_relationship_id),
        coaching_session_series_id: Set(None),
        ical_sequence: Set(0),
        ical_recurrence_id: Set(None),
        collab_document_name: Set(None),
        date: Set(now.naive_utc()),
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
    .expect("the session is seeded")
    .id
}

async fn seed_goal(db: &DatabaseConnection, coaching_relationship_id: Id, user_id: Id) -> Id {
    let now = Utc::now();
    goals::ActiveModel {
        id: Set(Id::new_v4()),
        coaching_relationship_id: Set(coaching_relationship_id),
        created_in_session_id: Set(None),
        user_id: Set(user_id),
        title: Set(None),
        body: Set(None),
        status: Set(status::Status::InProgress),
        status_changed_at: Set(None),
        completed_at: Set(None),
        target_date: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the goal is seeded")
    .id
}

async fn user_exists(db: &DatabaseConnection, id: Id) -> bool {
    users::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the user reads")
        .is_some()
}

async fn relationship_exists(db: &DatabaseConnection, id: Id) -> bool {
    coaching_relationships::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the relationship reads")
        .is_some()
}

async fn session_exists(db: &DatabaseConnection, id: Id) -> bool {
    coaching_sessions::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the session reads")
        .is_some()
}

async fn goal_exists(db: &DatabaseConnection, id: Id) -> bool {
    goals::Entity::find_by_id(id)
        .one(db)
        .await
        .expect("the goal reads")
        .is_some()
}

async fn role_count(db: &DatabaseConnection, user_id: Id) -> u64 {
    user_roles::Entity::find()
        .filter(user_roles::Column::UserId.eq(user_id))
        .count(db)
        .await
        .expect("the roles count")
}

/// The `details` of a 409, or `None` for any other error.
fn conflict_details(error: &Error) -> Option<&serde_json::Value> {
    match &error.error_kind {
        DomainErrorKind::Internal(InternalErrorKind::Entity(EntityErrorKind::Conflict {
            details,
            ..
        })) => details.as_ref(),
        _ => None,
    }
}

fn admin() -> Actor {
    Actor::new(Id::new_v4())
}

#[tokio::test]
async fn a_coachee_with_a_session_is_refused_and_nothing_is_deleted() {
    within_time_limit(async {
        let db = database_with_role_audit().await;
        let (user_id, organization_id) = seed_member(&db).await;
        let coach_id = seed_user(&db).await;
        let relationship_id = seed_relationship(&db, organization_id, coach_id, user_id).await;
        let session_id = seed_session(&db, relationship_id).await;

        let refused = delete(&db, admin(), user_id)
            .await
            .expect_err("the delete is refused");

        let details = conflict_details(&refused).expect("a 409 with details");
        assert_eq!(details["reason"], "user_has_coaching_history");
        assert_eq!(details["user_id"], serde_json::json!(user_id));
        assert_eq!(details["relationship_count"], 1);
        assert!(user_exists(&db, user_id).await);
        assert_eq!(role_count(&db, user_id).await, 1);
        assert!(relationship_exists(&db, relationship_id).await);
        assert!(session_exists(&db, session_id).await);
    })
    .await;
}

#[tokio::test]
async fn a_coach_and_coachee_with_sessions_counts_both_relationships() {
    within_time_limit(async {
        let db = database_with_role_audit().await;
        let (user_id, organization_id) = seed_member(&db).await;
        let (coachee_id, coach_id) = (seed_user(&db).await, seed_user(&db).await);
        let coaching = seed_relationship(&db, organization_id, user_id, coachee_id).await;
        let coached = seed_relationship(&db, organization_id, coach_id, user_id).await;
        let sessions = [
            seed_session(&db, coaching).await,
            seed_session(&db, coached).await,
        ];

        let refused = delete(&db, admin(), user_id)
            .await
            .expect_err("the delete is refused");

        let details = conflict_details(&refused).expect("a 409 with details");
        assert_eq!(details["reason"], "user_has_coaching_history");
        assert_eq!(details["relationship_count"], 2);
        assert!(user_exists(&db, user_id).await);
        assert_eq!(role_count(&db, user_id).await, 1);
        for relationship_id in [coaching, coached] {
            assert!(relationship_exists(&db, relationship_id).await);
        }
        for session_id in sessions {
            assert!(session_exists(&db, session_id).await);
        }
    })
    .await;
}

#[tokio::test]
async fn a_relationship_without_sessions_is_deleted_with_its_goal() {
    within_time_limit(async {
        let db = database_with_role_audit().await;
        let (user_id, organization_id) = seed_member(&db).await;
        let coach_id = seed_user(&db).await;
        let relationship_id = seed_relationship(&db, organization_id, coach_id, user_id).await;
        let goal_id = seed_goal(&db, relationship_id, coach_id).await;

        let destroyed = delete(&db, admin(), user_id).await?;

        assert_eq!(destroyed.len(), 1);
        assert!(!user_exists(&db, user_id).await);
        assert_eq!(role_count(&db, user_id).await, 0);
        assert!(!relationship_exists(&db, relationship_id).await);
        assert!(!goal_exists(&db, goal_id).await);
        Ok::<(), Error>(())
    })
    .await
    .expect("the delete succeeds");
}

#[tokio::test]
async fn a_user_without_relationships_is_deleted() {
    within_time_limit(async {
        let db = database_with_role_audit().await;
        let (user_id, _) = seed_member(&db).await;

        delete(&db, admin(), user_id).await?;

        assert!(!user_exists(&db, user_id).await);
        assert_eq!(role_count(&db, user_id).await, 0);
        Ok::<(), Error>(())
    })
    .await
    .expect("the delete succeeds");
}

#[tokio::test]
async fn a_session_created_after_the_check_is_refused_as_history_not_a_fault() {
    within_time_limit(async {
        let db = database_with_role_audit().await;
        let (user_id, organization_id) = seed_member(&db).await;
        let coach_id = seed_user(&db).await;
        let relationship_id = seed_relationship(&db, organization_id, coach_id, user_id).await;
        // A session elsewhere, which the trigger moves into the relationship as it is deleted.
        let elsewhere =
            seed_relationship(&db, organization_id, coach_id, seed_user(&db).await).await;
        let session_id = seed_session(&db, elsewhere).await;
        db.execute_unprepared(
            "CREATE TEMP TRIGGER a_session_lands_mid_delete \
             BEFORE DELETE ON refactor_platform.coaching_relationships \
             BEGIN UPDATE coaching_sessions SET coaching_relationship_id = OLD.id; END",
        )
        .await
        .expect("the trigger is created");

        let refused = delete(&db, admin(), user_id)
            .await
            .expect_err("the delete is refused");

        let details = conflict_details(&refused).expect("a 409 with details");
        assert_eq!(details["reason"], "user_has_coaching_history");
        assert_eq!(details["user_id"], serde_json::json!(user_id));
        assert_eq!(details.get("relationship_count"), None);
        assert!(user_exists(&db, user_id).await);
        assert_eq!(role_count(&db, user_id).await, 1);
        assert!(relationship_exists(&db, relationship_id).await);
        assert!(session_exists(&db, session_id).await);
    })
    .await;
}
