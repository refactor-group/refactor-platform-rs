use crate::error::Error;
use entity::{
    cost_metric::Metric,
    cost_pricing_config::{ActiveModel, Column, Entity, Model},
    pipeline_provider::Provider,
    Id,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, Order,
    QueryFilter, QueryOrder, QuerySelect,
};

/// Finds the most-recently effective rate for the given provider and metric as of now.
///
/// Returns `None` if no rate has ever been configured for this provider+metric combination.
pub async fn find_current_rate(
    db: &DatabaseConnection,
    provider: Provider,
    metric: Metric,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::Provider.eq(provider))
        .filter(Column::Metric.eq(metric))
        .filter(Column::EffectiveFrom.lte(chrono::Utc::now().fixed_offset()))
        .order_by(Column::EffectiveFrom, Order::Desc)
        .limit(1)
        .one(db)
        .await?)
}

pub async fn create(db: &DatabaseConnection, model: Model) -> Result<Model, Error> {
    let active_model = ActiveModel {
        id: Set(Id::new_v4()),
        provider: Set(model.provider),
        metric: Set(model.metric),
        unit: Set(model.unit),
        cost_per_unit_low: Set(model.cost_per_unit_low),
        cost_per_unit_high: Set(model.cost_per_unit_high),
        cost_per_unit_avg: Set(model.cost_per_unit_avg),
        effective_from: Set(model.effective_from),
    };

    Ok(active_model.insert(db).await?)
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::cost_unit::Unit;
    use sea_orm::{DatabaseBackend, MockDatabase};

    fn test_rate() -> Model {
        Model {
            id: Id::new_v4(),
            provider: Provider::RecallAi,
            metric: Metric::BotMinutes,
            unit: Unit::Minutes,
            cost_per_unit_low: 0.001,
            cost_per_unit_high: 0.005,
            cost_per_unit_avg: 0.003,
            effective_from: chrono::Utc::now().fixed_offset(),
        }
    }

    #[tokio::test]
    async fn find_current_rate_returns_some_when_configured() -> Result<(), Error> {
        let rate = test_rate();
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![rate.clone()]])
            .into_connection();

        let result = find_current_rate(&db, Provider::RecallAi, Metric::BotMinutes).await?;

        assert!(result.is_some());
        let found = result.unwrap();
        assert_eq!(found.provider, Provider::RecallAi);
        assert_eq!(found.metric, Metric::BotMinutes);
        Ok(())
    }

    #[tokio::test]
    async fn find_current_rate_returns_none_when_not_configured() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = find_current_rate(&db, Provider::RecallAi, Metric::BotMinutes).await?;

        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn create_returns_new_rate() -> Result<(), Error> {
        let rate = test_rate();
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![rate.clone()]])
            .into_connection();

        let result = create(&db, rate.clone()).await?;

        assert_eq!(result.provider, rate.provider);
        assert_eq!(result.metric, rate.metric);
        assert_eq!(result.cost_per_unit_avg, rate.cost_per_unit_avg);
        Ok(())
    }
}
