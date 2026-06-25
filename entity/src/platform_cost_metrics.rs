use crate::cost_metric;
use crate::pipeline_provider;
use crate::Id;
use sea_orm::entity::prelude::*;
use sea_orm::prelude::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, ToSchema)]
#[sea_orm(
    schema_name = "refactor_platform",
    table_name = "platform_cost_metrics"
)]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key)]
    pub id: Id,
    pub provider: pipeline_provider::Provider,
    pub metric: cost_metric::Metric,
    pub coaching_session_id: Option<Id>,
    /// Logical FK to the source record (meeting_recordings.id or transcriptions.id).
    /// Not a DB-level FK because it can point to different tables.
    pub source_record_id: Id,
    /// Computed cost. Stored as `NUMERIC(14, 6)` — micro-dollar precision
    /// (standard for usage-based billing), with headroom to ~$100M per row.
    #[sea_orm(column_type = "Decimal(Some((14, 6)))")]
    #[schema(value_type = f64)]
    pub cost_low: Decimal,
    #[sea_orm(column_type = "Decimal(Some((14, 6)))")]
    #[schema(value_type = f64)]
    pub cost_high: Decimal,
    #[sea_orm(column_type = "Decimal(Some((14, 6)))")]
    #[schema(value_type = f64)]
    pub cost_avg: Decimal,
    #[serde(skip_deserializing)]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTimeWithTimeZone,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::coaching_sessions::Entity",
        from = "Column::CoachingSessionId",
        to = "super::coaching_sessions::Column::Id",
        on_update = "NoAction",
        on_delete = "SetNull"
    )]
    CoachingSessions,
}

impl Related<super::coaching_sessions::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::CoachingSessions.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
