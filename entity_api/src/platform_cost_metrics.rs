use crate::error::Error;
use entity::{
    platform_cost_metrics::{Column, Entity, Model},
    Id,
};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, Order, QueryFilter, QueryOrder,
};

pub async fn create(db: &DatabaseConnection, model: Model) -> Result<Model, Error> {
    let mut active_model = model.into_active_model();
    // Override server-generated fields; the rest carry over from the caller's model.
    active_model.id = Set(Id::new_v4());
    active_model.created_at = Set(chrono::Utc::now().fixed_offset());

    Ok(active_model.insert(db).await?)
}

pub async fn find_by_session(
    db: &DatabaseConnection,
    coaching_session_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::CoachingSessionId.eq(coaching_session_id))
        .order_by(Column::CreatedAt, Order::Asc)
        .all(db)
        .await?)
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use sea_orm::prelude::Decimal;
    use sea_orm::{DatabaseBackend, MockDatabase};

    fn test_metric(session_id: Id) -> Model {
        Model {
            id: Id::new_v4(),
            provider: entity::pipeline_provider::Provider::RecallAi,
            metric: entity::cost_metric::Metric::BotMinutes,
            coaching_session_id: Some(session_id),
            source_record_id: Id::new_v4(),
            cost_low: Decimal::new(10, 2),
            cost_high: Decimal::new(50, 2),
            cost_avg: Decimal::new(30, 2),
            created_at: chrono::Utc::now().fixed_offset(),
        }
    }

    #[tokio::test]
    async fn create_returns_new_cost_metric() -> Result<(), Error> {
        let session_id = Id::new_v4();
        let metric = test_metric(session_id);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![metric.clone()]])
            .into_connection();

        let result = create(&db, metric.clone()).await?;

        assert_eq!(result.coaching_session_id, metric.coaching_session_id);
        assert_eq!(result.provider, metric.provider);
        assert_eq!(result.metric, metric.metric);
        Ok(())
    }

    #[tokio::test]
    async fn find_by_session_returns_metrics_for_session() -> Result<(), Error> {
        let session_id = Id::new_v4();
        let m1 = test_metric(session_id);
        let m2 = test_metric(session_id);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![m1.clone(), m2.clone()]])
            .into_connection();

        let result = find_by_session(&db, session_id).await?;

        assert_eq!(result.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn find_by_session_returns_empty_when_none_exist() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = find_by_session(&db, Id::new_v4()).await?;

        assert!(result.is_empty());
        Ok(())
    }
}
