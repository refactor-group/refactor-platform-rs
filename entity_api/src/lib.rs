use chrono::{DateTime, Days, Utc};
use password_auth::generate_hash;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

pub use entity::{
    actions, actions_users, agreements, audit_events, coachees, coaches,
    coaching_relationship_kind, coaching_relationships, coaching_session_images,
    coaching_session_notices, coaching_session_pending_reschedules, coaching_session_topics,
    coaching_session_views, coaching_sessions, coaching_sessions_goals, cost_metric, cost_unit,
    duration, goals, jwts, magic_link_tokens, meeting_provider, oauth_connections, organizations,
    password_reset_attempts, pipeline_provider, status, token_purpose, topic_status,
    transcript_suggestion_target, user_invite_status, user_lookup_attempts, user_role_changes,
    user_roles, users, users::Role, working_hours, Id,
};

pub mod action;
pub mod actions_user;
pub mod actor;
mod advisory_lock;
pub mod agreement;
pub mod audit_event;
pub mod coaching_relationship;
pub mod coaching_session;
pub mod coaching_session_display_title;
pub mod coaching_session_goal;
pub mod coaching_session_image;
pub mod coaching_session_notice;
pub mod coaching_session_pending_reschedule;
pub mod coaching_session_series;
pub mod coaching_session_topic;
pub mod coaching_session_view;
pub mod cost_pricing_config;
pub mod error;
pub mod goal;
pub mod goal_progress;
pub mod magic_link_token;
pub mod meeting_recording;
pub mod mutate;
pub mod oauth_connection;
pub mod organization;
pub mod password_reset_attempt;
pub mod platform_cost_metrics;
pub mod query;
pub mod search;
pub mod tiptap_metrics;
pub mod transcript_analysis;
pub mod transcript_participant;
pub mod transcript_segment;
pub mod transcript_suggestion;
pub mod transcription;
pub mod user;
pub mod user_lookup_attempt;
pub mod user_role;
pub mod user_role_change;

pub(crate) fn uuid_parse_str(uuid_str: &str) -> Result<Id, error::Error> {
    Id::parse_str(uuid_str).map_err(|_| error::Error {
        source: None,
        error_kind: error::EntityApiErrorKind::InvalidQueryTerm,
    })
}

pub async fn seed_database(db: &DatabaseConnection) {
    let now = Utc::now();

    // let _admin_user: users::ActiveModel = users::ActiveModel {
    //     email: Set("admin@refactorcoach.com".to_owned()),
    //     first_name: Set("Admin".to_owned()),
    //     last_name: Set("User".to_owned()),
    //     display_name: Set(Some("Admin User".to_owned())),
    //     password: Set(Some(generate_hash("dLxNxnjn&b!2sqkwFbb4s8jX"))),
    //     github_username: Set(None),
    //     github_profile_url: Set(None),
    //     created_at: Set(now.into()),
    //     updated_at: Set(now.into()),
    //     ..Default::default()
    // }
    // .save(db)
    // .await
    // .unwrap();

    let jim_hodapp: users::ActiveModel = users::ActiveModel {
        email: Set("james.hodapp@gmail.com".to_owned()),
        first_name: Set("Jim".to_owned()),
        last_name: Set("Hodapp".to_owned()),
        display_name: Set(Some("Jim H".to_owned())),
        password: Set(Some(generate_hash("password"))),
        github_username: Set(Some("jhodapp".to_owned())),
        github_profile_url: Set(Some("https://github.com/jhodapp".to_owned())),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    let caleb_bourg: users::ActiveModel = users::ActiveModel {
        email: Set("calebbourg2@gmail.com".to_owned()),
        first_name: Set("Caleb".to_owned()),
        last_name: Set("Bourg".to_owned()),
        display_name: Set(Some("cbourg2".to_owned())),
        password: Set(Some(generate_hash("password"))),
        github_username: Set(Some("calebbourg".to_owned())),
        github_profile_url: Set(Some("https://github.com/calebbourg".to_owned())),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    let dinah_manley: users::ActiveModel = users::ActiveModel {
        email: Set("dmanley@hotmail.com".to_owned()),
        first_name: Set("Dinah".to_owned()),
        last_name: Set("Manley".to_owned()),
        display_name: Set(Some("Dinah M.".to_owned())),
        password: Set(Some(generate_hash("password"))),
        github_username: Set(None),
        github_profile_url: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    let other_user: users::ActiveModel = users::ActiveModel {
        email: Set("other_user@gmail.com".to_owned()),
        first_name: Set("Other".to_owned()),
        last_name: Set("User".to_owned()),
        display_name: Set(Some("Other U.".to_owned())),
        password: Set(Some(generate_hash("password"))),
        github_username: Set(None),
        github_profile_url: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    let jim_refactor_group: users::ActiveModel = users::ActiveModel {
        email: Set("jim@refactorgroup.com".to_owned()),
        first_name: Set("Jim".to_owned()),
        last_name: Set("Hodapp (Refactor Group)".to_owned()),
        display_name: Set(Some("Jim (Refactor Group)".to_owned())),
        password: Set(Some(generate_hash("password"))),
        github_username: Set(None),
        github_profile_url: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    // "Refactor Group" is created by migration m20250509_164646_add_initial_user; reuse it
    // as the primary org instead of creating a separate "Refactor Coaching".
    let refactor_group_id = organizations::Entity::find()
        .filter(organizations::Column::Slug.eq("refactor-group"))
        .one(db)
        .await
        .unwrap()
        .expect("Refactor Group organization (from initial-admin migration) must exist")
        .id;

    let acme_corp = organizations::ActiveModel {
        name: Set("Acme Corp".to_owned()),
        slug: Set("acme-corp".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    // In Refactor Group, Jim (james.hodapp) is coaching Caleb.
    let jim_caleb_coaching_relationship = coaching_relationships::ActiveModel {
        coach_id: Set(jim_hodapp.id.clone().unwrap()),
        coachee_id: Set(caleb_bourg.id.clone().unwrap()),
        organization_id: Set(refactor_group_id),
        slug: Set("jim-caleb".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    // In Refactor Group, jim@refactorgroup.com is the default coach of james.hodapp@gmail.com.
    let jimrg_james_coaching_relationship = coaching_relationships::ActiveModel {
        coach_id: Set(jim_refactor_group.id.clone().unwrap()),
        coachee_id: Set(jim_hodapp.id.clone().unwrap()),
        organization_id: Set(refactor_group_id),
        slug: Set("jimrg-james".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    // In the Acme Corp organization, Caleb is coaching Jim.
    let caleb_jim_coaching_relationship = coaching_relationships::ActiveModel {
        coach_id: Set(caleb_bourg.id.clone().unwrap()),
        coachee_id: Set(jim_hodapp.id.clone().unwrap()),
        organization_id: Set(acme_corp.id.clone().unwrap()),
        slug: Set("jim-caleb".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    // In the Acme Corp organization, Caleb is coaching Dinah.
    let caleb_dinah_coaching_relationship = coaching_relationships::ActiveModel {
        coach_id: Set(caleb_bourg.id.clone().unwrap()),
        coachee_id: Set(dinah_manley.id.clone().unwrap()),
        organization_id: Set(acme_corp.id.clone().unwrap()),
        slug: Set("caleb-dinah".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    let jim_other_coaching_relationship = coaching_relationships::ActiveModel {
        coach_id: Set(jim_hodapp.id.clone().unwrap()),
        coachee_id: Set(other_user.id.clone().unwrap()),
        organization_id: Set(acme_corp.id.clone().unwrap()),
        slug: Set("jim-other".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    // jim@refactorgroup.com coaches james.hodapp@gmail.com (Refactor Group)
    seed_session(
        db,
        &jimrg_james_coaching_relationship,
        now.naive_local(),
        now,
    )
    .await;

    seed_session(
        db,
        &jimrg_james_coaching_relationship,
        now.naive_local().checked_sub_days(Days::new(7)).unwrap(),
        now,
    )
    .await;

    // Caleb is coach
    seed_session(db, &caleb_jim_coaching_relationship, now.naive_local(), now).await;

    seed_session(db, &caleb_jim_coaching_relationship, now.naive_local(), now).await;

    // Caleb coaches Dinah
    seed_session(
        db,
        &caleb_dinah_coaching_relationship,
        now.naive_local(),
        now,
    )
    .await;

    seed_session(
        db,
        &caleb_dinah_coaching_relationship,
        now.naive_local().checked_add_days(Days::new(7)).unwrap(),
        now,
    )
    .await;

    // Jim is coach
    seed_session(db, &jim_caleb_coaching_relationship, now.naive_local(), now).await;

    seed_session(
        db,
        &jim_caleb_coaching_relationship,
        now.naive_local().checked_add_days(Days::new(7)).unwrap(),
        now,
    )
    .await;

    seed_session(
        db,
        &jim_caleb_coaching_relationship,
        now.naive_local().checked_add_days(Days::new(14)).unwrap(),
        now,
    )
    .await;

    seed_session(
        db,
        &jim_caleb_coaching_relationship,
        now.naive_local().checked_add_days(Days::new(21)).unwrap(),
        now,
    )
    .await;

    seed_session(
        db,
        &jim_caleb_coaching_relationship,
        now.naive_local().checked_sub_days(Days::new(7)).unwrap(),
        now,
    )
    .await;

    seed_session(
        db,
        &jim_caleb_coaching_relationship,
        now.naive_local().checked_sub_days(Days::new(14)).unwrap(),
        now,
    )
    .await;

    // Jim coaches Other User (Acme): one past and one upcoming 1-off.
    seed_session(
        db,
        &jim_other_coaching_relationship,
        now.naive_local().checked_sub_days(Days::new(3)).unwrap(),
        now,
    )
    .await;

    seed_session(
        db,
        &jim_other_coaching_relationship,
        now.naive_local().checked_add_days(Days::new(5)).unwrap(),
        now,
    )
    .await;

    // Recurring series (sessions linked via coaching_session_series_id).
    // Jim -> Caleb (Refactor Group): weekly for 4 weeks.
    seed_recurring_series(
        db,
        &jim_caleb_coaching_relationship,
        now,
        now.naive_local().checked_add_days(Days::new(1)).unwrap(),
        "weekly",
        4,
    )
    .await;

    // Caleb -> Dinah (Acme): bi-weekly for 3 occurrences (~6 weeks).
    seed_recurring_series(
        db,
        &caleb_dinah_coaching_relationship,
        now,
        now.naive_local().checked_add_days(Days::new(2)).unwrap(),
        "biweekly",
        3,
    )
    .await;

    // jim@refactorgroup.com -> james.hodapp (Refactor Group): weekly for 4 weeks.
    seed_recurring_series(
        db,
        &jimrg_james_coaching_relationship,
        now,
        now.naive_local().checked_add_days(Days::new(3)).unwrap(),
        "weekly",
        4,
    )
    .await;

    // Jim (james.hodapp): User in Refactor Group and Acme Corp
    user_roles::ActiveModel {
        role: Set(Role::User),
        organization_id: Set(Some(refactor_group_id)),
        user_id: Set(jim_hodapp.id.clone().unwrap()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    user_roles::ActiveModel {
        role: Set(Role::User),
        organization_id: Set(Some(acme_corp.id.clone().unwrap())),
        user_id: Set(jim_hodapp.id.clone().unwrap()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    // Caleb: User in Refactor Group, Admin of Acme Corp
    user_roles::ActiveModel {
        role: Set(Role::User),
        organization_id: Set(Some(refactor_group_id)),
        user_id: Set(caleb_bourg.id.clone().unwrap()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    user_roles::ActiveModel {
        role: Set(Role::Admin),
        organization_id: Set(Some(acme_corp.id.clone().unwrap())),
        user_id: Set(caleb_bourg.id.clone().unwrap()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    // jim@refactorgroup.com: Admin of Refactor Group
    user_roles::ActiveModel {
        role: Set(Role::Admin),
        organization_id: Set(Some(refactor_group_id)),
        user_id: Set(jim_refactor_group.id.clone().unwrap()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    // Dinah: User in Acme Corp
    user_roles::ActiveModel {
        role: Set(Role::User),
        organization_id: Set(Some(acme_corp.id.clone().unwrap())),
        user_id: Set(dinah_manley.id.clone().unwrap()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    // Other User: User in Acme Corp
    user_roles::ActiveModel {
        role: Set(Role::User),
        organization_id: Set(Some(acme_corp.id.clone().unwrap())),
        user_id: Set(other_user.id.clone().unwrap()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();
}

/// Seeds a session on `relationship` scheduled by its coach, with both participants noticed
/// at `now` so the reminder sweep leaves it alone.
async fn seed_session(
    db: &DatabaseConnection,
    relationship: &coaching_relationships::ActiveModel,
    date: chrono::NaiveDateTime,
    now: DateTime<Utc>,
) {
    let session = coaching_sessions::ActiveModel {
        coaching_relationship_id: Set(relationship.id.clone().unwrap()),
        scheduled_by_user_id: Set(relationship.coach_id.clone().unwrap()),
        date: Set(date),
        collab_document_name: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .insert(db)
    .await
    .unwrap();

    coaching_session_notice::stamp(db, session.id, &participants(relationship), now.into())
        .await
        .unwrap();
}

/// The coach and coachee of a seeded `relationship`.
fn participants(relationship: &coaching_relationships::ActiveModel) -> [Id; 2] {
    [
        relationship.coach_id.clone().unwrap(),
        relationship.coachee_id.clone().unwrap(),
    ]
}

/// Seed one recurring series: create the `coaching_session_series` row (with a
/// rule matching what the API would persist), then materialize its
/// weekly/bi-weekly sessions via the entity_api helper so each links back to the
/// series. `frequency` is "weekly" or "biweekly"; duration follows the coach's
/// default so the rule and the materialized rows agree. Both participants are noticed at `now`.
async fn seed_recurring_series(
    db: &DatabaseConnection,
    relationship: &coaching_relationships::ActiveModel,
    now: DateTime<Utc>,
    start: chrono::NaiveDateTime,
    frequency: &str,
    count: u32,
) {
    let relationship_id = relationship.id.clone().unwrap();
    let coach_id = relationship.coach_id.clone().unwrap();
    let duration = coaching_session::resolve_duration(db, coach_id, None)
        .await
        .unwrap();
    let step_days: u64 = if frequency == "biweekly" { 14 } else { 7 };

    let rule = serde_json::json!({
        "start_at": start,
        "recurrence": { "frequency": frequency, "interval": 1, "count": count },
        "duration_minutes": duration.minutes(),
    });

    let series = coaching_session_series::create(
        db,
        coaching_session_series::Model {
            id: Id::default(),
            coaching_relationship_id: relationship_id,
            rule,
            created_by_user_id: coach_id,
            ical_sequence: 0,
            created_at: now.into(),
            updated_at: now.into(),
        },
    )
    .await
    .unwrap();

    let dates: Vec<chrono::NaiveDateTime> = (0..count)
        .map(|i| {
            start
                .checked_add_days(Days::new(step_days * u64::from(i)))
                .unwrap()
        })
        .collect();

    let sessions = coaching_session::bulk_create_recurring(
        db,
        relationship_id,
        coach_id,
        coach_id,
        series.id,
        dates,
        Some(duration),
    )
    .await
    .unwrap();

    let notices: Vec<_> = sessions
        .iter()
        .map(|session| (session.id, now.into()))
        .collect();
    coaching_session_notice::stamp_many(db, &notices, &participants(relationship))
        .await
        .unwrap();
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;

    #[tokio::test]
    async fn uuid_parse_str_parses_valid_uuid() {
        let uuid_str = "a98c3295-0933-44cb-89db-7db0f7250fb1";
        let uuid = uuid_parse_str(uuid_str).unwrap();
        assert_eq!(uuid.to_string(), uuid_str);
    }

    #[tokio::test]
    async fn uuid_parse_str_returns_error_for_invalid_uuid() {
        let uuid_str = "invalid";
        let result = uuid_parse_str(uuid_str);
        assert!(result.is_err());
    }
}
