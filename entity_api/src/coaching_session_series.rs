use super::error::{EntityApiErrorKind, Error};
use entity::coaching_session_series::{ActiveModel, Column, Entity, Model};
use entity::Id;
use log::debug;
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    ConnectionTrait, QueryOrder, TryIntoModel,
};

/// Inserts a new coaching_session_series row. The `id`, `created_at`, and
/// `updated_at` fields on `model` are ignored — the DB assigns them.
pub async fn create(db: &impl ConnectionTrait, model: Model) -> Result<Model, Error> {
    debug!(
        "Creating coaching_session_series for relationship {}",
        model.coaching_relationship_id
    );

    let now = chrono::Utc::now();
    let active_model = ActiveModel {
        coaching_relationship_id: Set(model.coaching_relationship_id),
        rule: Set(model.rule),
        created_by_user_id: Set(model.created_by_user_id),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.save(db).await?.try_into_model()?)
}

pub async fn find_by_id(db: &impl ConnectionTrait, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Returns every series owned by the given coaching relationship, most-recently
/// created first.
pub async fn find_by_relationship(
    db: &impl ConnectionTrait,
    coaching_relationship_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::CoachingRelationshipId.eq(coaching_relationship_id))
        .order_by_desc(Column::CreatedAt)
        .order_by_desc(Column::Id)
        .all(db)
        .await?)
}

/// Replaces the JSONB `rule` on an existing series and bumps `updated_at`.
/// Used by the reschedule flow.
pub async fn update_rule(
    db: &impl ConnectionTrait,
    id: Id,
    rule: serde_json::Value,
) -> Result<Model, Error> {
    let existing = find_by_id(db, id).await?;
    let active_model = ActiveModel {
        id: Unchanged(existing.id),
        coaching_relationship_id: Unchanged(existing.coaching_relationship_id),
        rule: Set(rule),
        created_by_user_id: Unchanged(existing.created_by_user_id),
        created_at: Unchanged(existing.created_at),
        updated_at: Set(chrono::Utc::now().into()),
    };
    Ok(active_model.update(db).await?.try_into_model()?)
}

pub async fn delete(db: &impl ConnectionTrait, id: Id) -> Result<(), Error> {
    Entity::delete_by_id(id).exec(db).await?;
    Ok(())
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, MockDatabase};

    fn sample_model() -> Model {
        let now = chrono::Utc::now();
        Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            rule: serde_json::json!({"frequency": "weekly", "interval": 1}),
            created_by_user_id: Id::new_v4(),
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    #[tokio::test]
    async fn create_returns_inserted_row() -> Result<(), Error> {
        let returned = sample_model();
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![returned.clone()]])
            .into_connection();

        let result = create(&db, returned.clone()).await?;
        assert_eq!(result.id, returned.id);
        assert_eq!(result.rule, returned.rule);
        Ok(())
    }

    #[tokio::test]
    async fn find_by_id_returns_record() -> Result<(), Error> {
        let returned = sample_model();
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![returned.clone()]])
            .into_connection();

        let result = find_by_id(&db, returned.id).await?;
        assert_eq!(result.id, returned.id);
        Ok(())
    }

    #[tokio::test]
    async fn find_by_id_missing_row_returns_record_not_found() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, _, _>(vec![vec![]])
            .into_connection();

        let err = find_by_id(&db, Id::new_v4()).await.unwrap_err();
        assert!(matches!(err.error_kind, EntityApiErrorKind::RecordNotFound));
    }

    #[tokio::test]
    async fn find_by_relationship_returns_all_for_relationship() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let row1 = Model {
            coaching_relationship_id: relationship_id,
            ..sample_model()
        };
        let row2 = Model {
            coaching_relationship_id: relationship_id,
            ..sample_model()
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![row1.clone(), row2.clone()]])
            .into_connection();

        let result = find_by_relationship(&db, relationship_id).await?;
        assert_eq!(result.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn update_rule_writes_new_rule_and_bumps_updated_at() -> Result<(), Error> {
        let existing = sample_model();
        let new_rule = serde_json::json!({"frequency": "monthly"});
        let after = Model {
            rule: new_rule.clone(),
            ..existing.clone()
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![existing.clone()]])
            .append_query_results(vec![vec![after.clone()]])
            .into_connection();

        let result = update_rule(&db, existing.id, new_rule.clone()).await?;
        assert_eq!(result.id, existing.id);
        assert_eq!(result.rule, new_rule);
        Ok(())
    }
}
