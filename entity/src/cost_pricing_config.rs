use crate::cost_metric;
use crate::cost_unit;
use crate::pipeline_provider;
use crate::Id;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, ToSchema)]
#[sea_orm(schema_name = "refactor_platform", table_name = "cost_pricing_config")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    pub provider: pipeline_provider::Provider,
    pub metric: cost_metric::Metric,
    pub unit: cost_unit::Unit,
    pub cost_per_unit_low: f64,
    pub cost_per_unit_high: f64,
    pub cost_per_unit_avg: f64,
    #[schema(value_type = String, format = DateTime)]
    pub effective_from: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
