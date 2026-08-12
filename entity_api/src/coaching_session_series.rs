use super::error::{EntityApiErrorKind, Error};
pub use entity::coaching_session_series::Model;
use entity::coaching_session_series::{ActiveModel, Column, Entity};
use entity::Id;
use log::debug;
use sea_orm::{entity::prelude::*, ActiveValue::Set, ConnectionTrait, QueryOrder, TryIntoModel};

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
/// Used by the reschedule flow. Every rule replacement moves the calendar, so
/// `ical_sequence` (RFC 5545 SEQUENCE) is incremented in the same statement, as a
/// column expression rather than a read-then-write: two concurrent reschedules
/// must not land on the same SEQUENCE, or a calendar client drops the second
/// invite as a duplicate.
pub async fn update_rule(
    db: &impl ConnectionTrait,
    id: Id,
    rule: serde_json::Value,
) -> Result<Model, Error> {
    Entity::update_many()
        .col_expr(Column::Rule, Expr::value(rule))
        .col_expr(Column::IcalSequence, Expr::col(Column::IcalSequence).add(1))
        .col_expr(
            Column::UpdatedAt,
            Expr::value::<DateTimeWithTimeZone>(chrono::Utc::now().into()),
        )
        .filter(Column::Id.eq(id))
        .exec_with_returning(db)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        })
}

/// Bump a series' `ical_sequence` by 1 without touching its rule.
///
/// Used by the cancellation path, which needs a SEQUENCE strictly higher than any
/// concurrently-committed reschedule. Same column-expression increment as
/// [`update_rule`], so the row lock orders it against a competing update.
pub async fn increment_ical_sequence(db: &impl ConnectionTrait, id: Id) -> Result<Model, Error> {
    Entity::update_many()
        .col_expr(Column::IcalSequence, Expr::col(Column::IcalSequence).add(1))
        .filter(Column::Id.eq(id))
        .exec_with_returning(db)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        })
}

pub async fn delete(db: &impl ConnectionTrait, id: Id) -> Result<(), Error> {
    Entity::delete_by_id(id).exec(db).await?;
    Ok(())
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

    fn sample_model() -> Model {
        let now = chrono::Utc::now();
        Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            rule: serde_json::json!({"frequency": "weekly", "interval": 1}),
            ical_sequence: 0,
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
            .append_query_results(vec![vec![after.clone()]])
            .into_connection();

        let result = update_rule(&db, existing.id, new_rule.clone()).await?;
        assert_eq!(result.id, existing.id);
        assert_eq!(result.rule, new_rule);
        Ok(())
    }

    /// Collect the SQL of every UPDATE of coaching_session_series in the transaction log.
    fn series_update_sql(log: &[Transaction]) -> Vec<String> {
        log.iter()
            .flat_map(|txn| txn.statements())
            .filter(|stmt| {
                stmt.sql.contains("UPDATE") && stmt.sql.contains(r#""coaching_session_series""#)
            })
            .map(|stmt| stmt.sql.clone())
            .collect()
    }

    /// A rule replacement is a calendar reschedule, so the emitted UPDATE must bump
    /// `ical_sequence` relative to its own stored value rather than to a value read in
    /// an earlier statement. Two concurrent reschedules that both read N would both
    /// write N+1, and a calendar client drops the second invite as a duplicate.
    /// Asserting on the returned model would prove nothing: MockDatabase echoes
    /// whatever row the test appended.
    #[tokio::test]
    async fn update_rule_bumps_ical_sequence_atomically() -> Result<(), Error> {
        let existing = sample_model();
        let new_rule = serde_json::json!({"frequency": "monthly"});
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![existing.clone()]])
            .into_connection();

        update_rule(&db, existing.id, new_rule.clone()).await?;

        let statements = series_update_sql(&db.into_transaction_log());
        assert_eq!(
            statements.len(),
            1,
            "expected exactly one series UPDATE: {statements:?}"
        );
        assert!(
            statements[0].contains(r#""ical_sequence" = "ical_sequence" + "#),
            "SEQUENCE must be incremented in SQL, not read-then-written: {}",
            statements[0]
        );
        Ok(())
    }
}
