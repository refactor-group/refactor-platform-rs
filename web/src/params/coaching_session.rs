use chrono::NaiveDate;
use domain::Id;
use domain::{IntoQueryFilterMap, QueryFilterMap};
use sea_orm::Value;
use serde::Deserialize;
use utoipa::IntoParams;

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    pub(crate) coaching_relationship_id: Id,
    pub(crate) from_date: NaiveDate,
    pub(crate) to_date: NaiveDate,
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
