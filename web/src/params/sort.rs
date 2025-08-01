use serde::Deserialize;
use utoipa::ToSchema;

/// Common sort order values used across all entities
#[derive(Debug, Deserialize, ToSchema, Clone)]
#[schema(example = "desc")]
pub enum SortOrder {
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}
