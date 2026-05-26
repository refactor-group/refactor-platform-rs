pub(crate) mod goal;
pub(crate) mod recurring;

use chrono::{NaiveDate, NaiveDateTime};
use domain::provider::Provider;
use sea_orm::{ActiveEnum, Order, Value};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use super::sort::SortOrder;
use super::WithSortDefaults;
use domain::{
    coaching_sessions, Id, IntoQueryFilterMap, IntoUpdateMap, QueryFilterMap, QuerySort, UpdateMap,
};

/// Sortable fields for coaching sessions
#[derive(Debug, Deserialize, ToSchema)]
#[schema(example = "date")]
pub(crate) enum SortField {
    #[serde(rename = "date")]
    Date,
    #[serde(rename = "created_at")]
    CreatedAt,
    #[serde(rename = "updated_at")]
    UpdatedAt,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    pub(crate) coaching_relationship_id: Id,
    pub(crate) from_date: NaiveDate,
    pub(crate) to_date: NaiveDate,
    pub(crate) sort_by: Option<SortField>,
    pub(crate) sort_order: Option<SortOrder>,
}

impl IntoQueryFilterMap for IndexParams {
    fn into_query_filter_map(self) -> QueryFilterMap {
        let mut query_filter_map = QueryFilterMap::new();
        query_filter_map.insert(
            "coaching_relationship_id".to_string(),
            Some(Value::Uuid(Some(Box::new(self.coaching_relationship_id)))),
        );
        query_filter_map.insert(
            "from_date".to_string(),
            Some(Value::ChronoDate(Some(Box::new(self.from_date)))),
        );
        query_filter_map.insert(
            "to_date".to_string(),
            Some(Value::ChronoDate(Some(Box::new(self.to_date)))),
        );
        query_filter_map
    }
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub(crate) struct UpdateParams {
    pub(crate) date: NaiveDateTime,
    /// Session duration in minutes (1..=480). Validated in entity_api via
    /// the `Duration` newtype. `i16` matches both the storage type
    /// (PG `SMALLINT` ↔ sqlx-postgres `i16`) and the newtype's inner
    /// representation, so no conversion is needed at any layer.
    pub(crate) duration_minutes: Option<i16>,
    pub(crate) meeting_url: Option<String>,
    pub(crate) provider: Option<Provider>,
}

impl IntoUpdateMap for UpdateParams {
    fn into_update_map(self) -> UpdateMap {
        let mut update_map = UpdateMap::new();
        update_map.insert(
            "date".to_string(),
            Some(Value::ChronoDateTime(Some(Box::new(self.date)))),
        );
        if let Some(duration_minutes) = self.duration_minutes {
            update_map.insert(
                "duration_minutes".to_string(),
                Some(Value::SmallInt(Some(duration_minutes))),
            );
        }
        if let Some(meeting_url) = self.meeting_url {
            update_map.insert(
                "meeting_url".to_string(),
                Some(Value::String(Some(Box::new(meeting_url)))),
            );
        }
        if let Some(provider) = self.provider {
            update_map.insert(
                "provider".to_string(),
                Some(Value::String(Some(Box::new(provider.to_value())))),
            );
        }
        update_map
    }
}

/// Request body for `POST /coaching_sessions`. Decoupled from the entity
/// `Model` so `duration_minutes` can be optional on the wire (omission
/// triggers the BE defaulting cascade — see
/// `entity_api::coaching_session::resolve_duration`).
#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct CreateParams {
    pub(crate) coaching_relationship_id: Id,
    pub(crate) date: NaiveDateTime,
    /// Session duration in minutes (1..=480). Omit to use the coach's stored
    /// `default_coaching_session_duration_minutes`.
    pub(crate) duration_minutes: Option<i16>,
    pub(crate) meeting_url: Option<String>,
    pub(crate) provider: Option<Provider>,
}

impl CreateParams {
    /// Build the entity `Model` that the domain layer expects. The
    /// `duration_minutes` placeholder here is irrelevant — the value is
    /// resolved (and overridden) inside `entity_api::coaching_session::create`
    /// via the defaulting cascade. Other field defaults are also overwritten
    /// by the domain layer (id on insert, hydrated_at via the hydration
    /// machinery, timestamps on save).
    pub(crate) fn into_model(self) -> coaching_sessions::Model {
        let now = chrono::Utc::now();
        coaching_sessions::Model {
            id: Id::nil(),
            coaching_relationship_id: self.coaching_relationship_id,
            collab_document_name: None,
            date: self.date,
            duration_minutes: domain::duration::Duration::default_minutes(),
            meeting_url: self.meeting_url,
            provider: self.provider,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
        }
    }
}

impl QuerySort<coaching_sessions::Column> for IndexParams {
    fn get_sort_column(&self) -> Option<coaching_sessions::Column> {
        self.sort_by.as_ref().map(|field| match field {
            SortField::Date => coaching_sessions::Column::Date,
            SortField::CreatedAt => coaching_sessions::Column::CreatedAt,
            SortField::UpdatedAt => coaching_sessions::Column::UpdatedAt,
        })
    }

    fn get_sort_order(&self) -> Option<Order> {
        self.sort_order.as_ref().map(|order| match order {
            SortOrder::Asc => Order::Asc,
            SortOrder::Desc => Order::Desc,
        })
    }
}

impl WithSortDefaults for IndexParams {
    type SortField = SortField;
}
