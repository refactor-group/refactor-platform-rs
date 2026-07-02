use sea_orm::entity::prelude::*;
use sea_orm::prelude::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(
    Debug,
    Clone,
    Copy,
    Eq,
    PartialEq,
    EnumIter,
    Deserialize,
    Serialize,
    DeriveActiveEnum,
    Default,
    ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "cost_unit")]
#[schema(as = domain::cost_unit::Unit)]
pub enum Unit {
    #[sea_orm(string_value = "minutes")]
    #[default]
    Minutes,

    #[sea_orm(string_value = "hours")]
    Hours,

    #[sea_orm(string_value = "tokens")]
    Tokens,
}

impl Unit {
    /// Billable quantity for a recording duration, expressed in this unit.
    ///
    /// Returns `None` when there is nothing to bill: a missing or non-positive
    /// duration, or a unit not derivable from a wall-clock duration (e.g.
    /// `Tokens`). Callers treat `None` as "skip recording" rather than writing a
    /// misleading zero-cost row. Returns `Decimal` so the quantity flows into the
    /// `NUMERIC` cost columns without a lossy `f64` hop.
    pub fn quantity_from_seconds(&self, seconds: Option<i32>) -> Option<Decimal> {
        let seconds = seconds.filter(|s| *s > 0)?;
        let seconds_per_unit = match self {
            Unit::Minutes => 60,
            Unit::Hours => 3600,
            Unit::Tokens => return None,
        };
        Some(Decimal::from(seconds) / Decimal::from(seconds_per_unit))
    }
}
