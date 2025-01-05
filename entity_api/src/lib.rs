use chrono::{Days, Utc};
use password_auth::generate_hash;
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};

use entity::{coaching_relationships, coaching_sessions, organizations, users, Id};

pub mod action;
pub mod agreement;
pub mod coaching_relationship;
pub mod coaching_session;
pub mod error;
pub mod note;
pub mod organization;
pub mod overarching_goal;
pub mod user;

pub(crate) fn uuid_parse_str(uuid_str: &str) -> Result<Id, error::Error> {
    Id::parse_str(uuid_str).map_err(|_| error::Error {
        inner: None,
        error_code: error::EntityApiErrorCode::InvalidQueryTerm,
    })
}

pub(crate) fn naive_date_parse_str(date_str: &str) -> Result<chrono::NaiveDate, error::Error> {
    chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").map_err(|_| error::Error {
        inner: None,
        error_code: error::EntityApiErrorCode::InvalidQueryTerm,
    })
}

pub async fn seed_database(db: &DatabaseConnection) {
    let now = Utc::now();

    let _admin_user: users::ActiveModel = users::ActiveModel {
        email: Set("admin@refactorcoach.com".to_owned()),
        first_name: Set(Some("Admin".to_owned())),
        last_name: Set(Some("User".to_owned())),
        display_name: Set(Some("Admin User".to_owned())),
        password: Set(generate_hash("dLxNxnjn&b!2sqkwFbb4s8jX")),
        github_username: Set(None),
        github_profile_url: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    let jim_hodapp: users::ActiveModel = users::ActiveModel {
        email: Set("james.hodapp@gmail.com".to_owned()),
        first_name: Set(Some("Jim".to_owned())),
        last_name: Set(Some("Hodapp".to_owned())),
        display_name: Set(Some("Jim H".to_owned())),
        password: Set(generate_hash("password")),
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
        first_name: Set(Some("Caleb".to_owned())),
        last_name: Set(Some("Bourg".to_owned())),
        display_name: Set(Some("cbourg2".to_owned())),
        password: Set(generate_hash("password")),
        github_username: Set(Some("calebbourg".to_owned())),
        github_profile_url: Set(Some("https://github.com/calebbourg".to_owned())),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    let other_user: users::ActiveModel = users::ActiveModel {
        email: Set("other_user@gmail.com".to_owned()),
        first_name: Set(Some("Other".to_owned())),
        last_name: Set(Some("User".to_owned())),
        display_name: Set(Some("Other U.".to_owned())),
        password: Set(generate_hash("password")),
        github_username: Set(None),
        github_profile_url: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    let refactor_coaching = organizations::ActiveModel {
        name: Set("Refactor Coaching".to_owned()),
        slug: Set("refactor-coaching".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

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

    let jim_caleb_coaching_relationship = coaching_relationships::ActiveModel {
        coach_id: Set(jim_hodapp.id.clone().unwrap()),
        coachee_id: Set(caleb_bourg.id.clone().unwrap()),
        organization_id: Set(refactor_coaching.id.unwrap()),
        slug: Set("jim-caleb".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    coaching_relationships::ActiveModel {
        coach_id: Set(jim_hodapp.id.clone().unwrap()),
        coachee_id: Set(other_user.id.clone().unwrap()),
        organization_id: Set(acme_corp.id.unwrap()),
        slug: Set("jim-other".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    coaching_sessions::ActiveModel {
        coaching_relationship_id: Set(jim_caleb_coaching_relationship.id.clone().unwrap()),
        date: Set(now.naive_local()),
        timezone: Set("America/Chicago".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    coaching_sessions::ActiveModel {
        coaching_relationship_id: Set(jim_caleb_coaching_relationship.id.clone().unwrap()),
        date: Set(now.naive_local().checked_add_days(Days::new(7)).unwrap()),
        timezone: Set("America/Chicago".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    coaching_sessions::ActiveModel {
        coaching_relationship_id: Set(jim_caleb_coaching_relationship.id.clone().unwrap()),
        date: Set(now.naive_local().checked_add_days(Days::new(14)).unwrap()),
        timezone: Set("America/Chicago".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    coaching_sessions::ActiveModel {
        coaching_relationship_id: Set(jim_caleb_coaching_relationship.id.clone().unwrap()),
        date: Set(now.naive_local().checked_add_days(Days::new(21)).unwrap()),
        timezone: Set("America/Chicago".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    coaching_sessions::ActiveModel {
        coaching_relationship_id: Set(jim_caleb_coaching_relationship.id.clone().unwrap()),
        date: Set(now.naive_local().checked_add_days(Days::new(28)).unwrap()),
        timezone: Set("America/Chicago".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    coaching_sessions::ActiveModel {
        coaching_relationship_id: Set(jim_caleb_coaching_relationship.id.clone().unwrap()),
        date: Set(now.naive_local().checked_sub_days(Days::new(7)).unwrap()),
        timezone: Set("America/Chicago".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    coaching_sessions::ActiveModel {
        coaching_relationship_id: Set(jim_caleb_coaching_relationship.id.clone().unwrap()),
        date: Set(now.naive_local().checked_sub_days(Days::new(14)).unwrap()),
        timezone: Set("America/Chicago".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    coaching_sessions::ActiveModel {
        coaching_relationship_id: Set(jim_caleb_coaching_relationship.id.clone().unwrap()),
        date: Set(now.naive_local().checked_sub_days(Days::new(21)).unwrap()),
        timezone: Set("America/Chicago".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    coaching_sessions::ActiveModel {
        coaching_relationship_id: Set(jim_caleb_coaching_relationship.id.clone().unwrap()),
        date: Set(now.naive_local().checked_sub_days(Days::new(28)).unwrap()),
        timezone: Set("America/Chicago".to_owned()),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();
}

#[cfg(test)]
// We need to gate seaORM's mock feature behind conditional compilation because
// the feature removes the Clone trait implementation from seaORM's DatabaseConnection.
// see https://github.com/SeaQL/sea-orm/issues/830
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

    #[tokio::test]
    async fn naive_date_parse_str_parses_valid_date() {
        let date_str = "2021-08-01";
        let date = naive_date_parse_str(date_str).unwrap();
        assert_eq!(date.to_string(), date_str);
    }

    #[tokio::test]
    async fn naive_date_parse_str_returns_error_for_invalid_date() {
        let date_str = "invalid";
        let result = naive_date_parse_str(date_str);
        assert!(result.is_err());
    }
}
