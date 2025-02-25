use chrono::{Days, Utc};
use password_auth::generate_hash;
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set, Value};
use std::collections::HashMap;

pub use entity::{
    actions, agreements, coachees, coaches, coaching_relationships, coaching_sessions, jwts, notes,
    organizations, overarching_goals, users, Id,
};

pub mod action;
pub mod agreement;
pub mod coaching_relationship;
pub mod coaching_session;
pub mod error;
pub mod note;
pub mod organization;
pub mod overarching_goal;
pub mod query;
pub mod user;

pub(crate) fn uuid_parse_str(uuid_str: &str) -> Result<Id, error::Error> {
    Id::parse_str(uuid_str).map_err(|_| error::Error {
        source: None,
        error_kind: error::EntityApiErrorKind::InvalidQueryTerm,
    })
}

pub(crate) fn naive_date_parse_str(date_str: &str) -> Result<chrono::NaiveDate, error::Error> {
    chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d").map_err(|_| error::Error {
        source: None,
        error_kind: error::EntityApiErrorKind::InvalidQueryTerm,
    })
}

/// `QueryFilterMap` is a data structure that serves as a bridge for translating filter parameters
/// between different layers of the application. It is essentially a wrapper around a `HashMap`
/// where the keys are filter parameter names (as `String`) and the values are optional `Value` types
/// from `sea_orm`.
///
/// This structure is particularly useful in scenarios where you need to pass filter parameters
/// from a web request down to the database query layer in a type-safe and organized manner.
///
/// # Example
///
/// ```
/// use sea_orm::Value;
/// use entity_api::QueryFilterMap;
///
/// let mut query_filter_map = QueryFilterMap::new();
/// query_filter_map.insert("coaching_session_id".to_string(), Some(Value::String(Some(Box::new("a_coaching_session_id".to_string())))));
/// let filter_value = query_filter_map.get("coaching_session_id");
/// ```
pub struct QueryFilterMap {
    map: HashMap<String, Option<Value>>,
}

impl QueryFilterMap {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        // HashMap.get returns an Option and so we need to "flatten" this to a single Option
        self.map
            .get(key)
            .and_then(|inner_option| inner_option.clone())
    }

    pub fn insert(&mut self, key: String, value: Option<Value>) {
        self.map.insert(key, value);
    }
}

impl Default for QueryFilterMap {
    fn default() -> Self {
        Self::new()
    }
}

/// `IntoQueryFilterMap` is a trait that provides a method for converting a struct into a `QueryFilterMap`.
/// This is particularly useful for translating data between different layers of the application,
/// such as from web request parameters to database query filters.
///
/// Implementing this trait for a struct allows you to define how the fields of the struct should be
/// mapped to the keys and values of the `QueryFilterMap`. This ensures that the data is passed
/// in a type-safe and organized manner.
///
/// # Example
///
/// ```
/// use entity_api::QueryFilterMap;
/// use entity_api::IntoQueryFilterMap;
///
/// #[derive(Debug)]
/// struct MyParams {
///     coaching_session_id: String,
/// }
///
/// impl IntoQueryFilterMap for MyParams {
///     fn into_query_filter_map(self) -> QueryFilterMap {
///         let mut query_filter_map = QueryFilterMap::new();
///         query_filter_map.insert(
///             "coaching_session_id".to_string(),
///             Some(sea_orm::Value::String(Some(Box::new(self.coaching_session_id)))),
///         );
///         query_filter_map
///     }
/// }
/// ```
pub trait IntoQueryFilterMap {
    fn into_query_filter_map(self) -> QueryFilterMap;
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

    // In refactor_coaching, Jim is coaching Caleb.
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

    coaching_relationships::ActiveModel {
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

    // caleb is Coach
    coaching_sessions::ActiveModel {
        coaching_relationship_id: Set(caleb_jim_coaching_relationship.id.clone().unwrap()),
        date: Set(now.naive_local()),
        collab_document_name: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    coaching_sessions::ActiveModel {
        coaching_relationship_id: Set(caleb_jim_coaching_relationship.id.clone().unwrap()),
        date: Set(now.naive_local()),
        collab_document_name: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    }
    .save(db)
    .await
    .unwrap();

    // Jim is coach
    coaching_sessions::ActiveModel {
        coaching_relationship_id: Set(jim_caleb_coaching_relationship.id.clone().unwrap()),
        date: Set(now.naive_local()),
        collab_document_name: Set(None),
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
        collab_document_name: Set(None),
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
        collab_document_name: Set(None),
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
        collab_document_name: Set(None),
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
        collab_document_name: Set(None),
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
        collab_document_name: Set(None),
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
        collab_document_name: Set(None),
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
        collab_document_name: Set(None),
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
        collab_document_name: Set(None),
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
