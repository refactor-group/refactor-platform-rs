use crate::cost_metric;
use crate::cost_unit;
use crate::pipeline_provider;
use crate::Id;
use sea_orm::entity::prelude::*;
use sea_orm::prelude::Decimal;
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
    /// Per-unit rate. Stored as `NUMERIC(20, 10)` so per-token pricing
    /// (e.g. ~$0.0000025) keeps full fractional precision before multiplication.
    #[sea_orm(column_type = "Decimal(Some((20, 10)))")]
    #[schema(value_type = f64)]
    pub cost_per_unit_low: Decimal,
    #[sea_orm(column_type = "Decimal(Some((20, 10)))")]
    #[schema(value_type = f64)]
    pub cost_per_unit_high: Decimal,
    #[sea_orm(column_type = "Decimal(Some((20, 10)))")]
    #[schema(value_type = f64)]
    pub cost_per_unit_avg: Decimal,
    #[schema(value_type = String, format = DateTime)]
    pub effective_from: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
