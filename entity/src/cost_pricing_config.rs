use crate::cost_metric;
use crate::cost_unit;
use crate::pipeline_provider;
use crate::Id;
use sea_orm::entity::prelude::*;
use sea_orm::prelude::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize, ToSchema)]
#[schema(as = entity::cost_pricing_config::Model)]
#[sea_orm(schema_name = "refactor_platform", table_name = "cost_pricing_config")]
pub struct Model {
    #[serde(skip_deserializing)]
    #[sea_orm(primary_key, auto_increment = false)]
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

/// The low/high/avg cost of a billable quantity at a given rate.
///
/// Mirrors the three `cost_per_unit_*` columns so one quantity yields the full
/// cost range in a single step.
pub struct CostRange {
    pub low: Decimal,
    pub high: Decimal,
    pub avg: Decimal,
}

impl Model {
    /// Cost of `quantity` units at this rate, as a low/high/avg range.
    ///
    /// The mirror image of [`cost_unit::Unit::quantity_from_seconds`]: that turns
    /// a duration into a quantity, this turns a quantity into a cost. Multiplying
    /// `Decimal`s keeps `NUMERIC` precision end-to-end with no lossy `f64` hop.
    pub fn cost_for(&self, quantity: Decimal) -> CostRange {
        CostRange {
            low: quantity * self.cost_per_unit_low,
            high: quantity * self.cost_per_unit_high,
            avg: quantity * self.cost_per_unit_avg,
        }
    }

    /// Exact cost of a token-priced call, with the same figure in low, high and avg.
    ///
    /// A token rate keeps the input price in `cost_per_unit_low` and the output price in
    /// `cost_per_unit_high`. The call's token counts are known, so no blended average is needed.
    pub fn cost_for_tokens(&self, input: Decimal, output: Decimal) -> CostRange {
        let exact = input * self.cost_per_unit_low + output * self.cost_per_unit_high;
        CostRange {
            low: exact,
            high: exact,
            avg: exact,
        }
    }
}

impl ActiveModelBehavior for ActiveModel {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_call_costs_input_at_the_low_rate_and_output_at_the_high_rate() {
        let rate = Model {
            id: Id::new_v4(),
            provider: pipeline_provider::Provider::LlmGateway,
            metric: cost_metric::Metric::LlmTokens,
            unit: cost_unit::Unit::Tokens,
            cost_per_unit_low: Decimal::new(33, 7),
            cost_per_unit_high: Decimal::new(165, 7),
            cost_per_unit_avg: Decimal::new(51857, 10),
            effective_from: chrono::Utc::now().fixed_offset(),
        };

        // 29,020 input and 400 output tokens, as billed by AssemblyAI for a 59-minute session.
        let cost = rate.cost_for_tokens(Decimal::from(29_020), Decimal::from(400));

        let billed = Decimal::new(102366, 6);
        assert_eq!((cost.low, cost.high, cost.avg), (billed, billed, billed));
    }
}
