use domain::Id;
use sea_orm::Value;
use serde::Deserialize;
use utoipa::IntoParams;

use domain::{IntoQueryFilterMap, QueryFilterMap};

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    pub(crate) coaching_session_id: Id,
}

impl IntoQueryFilterMap for IndexParams {
    fn into_query_filter_map(self) -> QueryFilterMap {
        let mut query_filter_map = QueryFilterMap::new();
        query_filter_map.insert(
            "coaching_session_id".to_string(),
            Some(Value::Uuid(Some(Box::new(self.coaching_session_id)))),
        );

        query_filter_map
    }
}
