use chrono::{NaiveDate, NaiveDateTime};
use domain::coaching_session::CoachingSessionSortParams;
use domain::coaching_sessions;
use domain::Id;
use domain::{IntoQueryFilterMap, IntoUpdateMap, QueryFilterMap, UpdateMap};
use sea_orm::{Order, Value};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize)]
pub(crate) enum CoachingSessionSortField {
    #[serde(rename = "date")]
    Date,
    #[serde(rename = "created_at")]
    CreatedAt,
    #[serde(rename = "updated_at")]
    UpdatedAt,
}

#[derive(Debug, Deserialize)]
pub(crate) enum SortOrder {
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    pub(crate) coaching_relationship_id: Id,
    pub(crate) from_date: NaiveDate,
    pub(crate) to_date: NaiveDate,
    pub(crate) sort_by: Option<CoachingSessionSortField>,
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
}

impl IntoUpdateMap for UpdateParams {
    fn into_update_map(self) -> UpdateMap {
        let mut update_map = UpdateMap::new();
        update_map.insert(
            "date".to_string(),
            Some(Value::ChronoDateTime(Some(Box::new(self.date)))),
        );
        update_map
    }
}

impl CoachingSessionSortParams for IndexParams {
    fn get_sort_column(&self) -> Option<coaching_sessions::Column> {
        self.sort_by.as_ref().map(|field| match field {
            CoachingSessionSortField::Date => coaching_sessions::Column::Date,
            CoachingSessionSortField::CreatedAt => coaching_sessions::Column::CreatedAt,
            CoachingSessionSortField::UpdatedAt => coaching_sessions::Column::UpdatedAt,
        })
    }

    fn get_sort_order(&self) -> Option<Order> {
        self.sort_order.as_ref().map(|order| match order {
            SortOrder::Asc => Order::Asc,
            SortOrder::Desc => Order::Desc,
        })
    }
}
