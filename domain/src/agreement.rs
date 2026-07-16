use crate::agreements::Model;
use crate::coaching_session;
use crate::error::Error;
use crate::events::{DomainEvent, EventPublisher};
use crate::Id;
use entity_api::query::{IntoQueryFilterMap, QuerySort};
use entity_api::{agreements, query};
use log::*;
use sea_orm::DatabaseConnection;

// Mutations (create, update, delete_by_id) are wrapped below to emit SSE; reads re-export directly.
pub use entity_api::agreement::find_by_id;

pub async fn find_by<P>(db: &DatabaseConnection, params: P) -> Result<Vec<Model>, Error>
where
    P: IntoQueryFilterMap + QuerySort<agreements::Column>,
{
    let agreements =
        query::find_by::<agreements::Entity, agreements::Column, P>(db, params).await?;
    Ok(agreements)
}

/// Best-effort SSE notify set for an agreement: the participants of the agreement's session. The
/// DB write is the contract; a failed lookup must NOT fail the mutation, so log and return None.
async fn agreement_notify_user_ids(
    db: &DatabaseConnection,
    coaching_session_id: Id,
) -> Option<Vec<Id>> {
    match coaching_session::find_participant_ids(db, coaching_session_id).await {
        Ok(ids) => Some(ids),
        Err(e) => {
            error!("agreement SSE: failed to resolve participants for session {coaching_session_id}: {e:?}");
            None
        }
    }
}

/// Publishes `AgreementCreated` or `AgreementUpdated` carrying the full agreement entity.
async fn publish_agreement_changed(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    agreement: &Model,
    created: bool,
) {
    let coaching_session_id = agreement.coaching_session_id;
    let Some(notify_user_ids) = agreement_notify_user_ids(db, coaching_session_id).await else {
        return;
    };
    let payload = match serde_json::to_value(agreement) {
        Ok(payload) => payload,
        Err(e) => {
            error!("agreement SSE: failed to serialize agreement for session {coaching_session_id}: {e:?}");
            return;
        }
    };
    let event = if created {
        DomainEvent::AgreementCreated {
            coaching_session_id,
            agreement: payload,
            notify_user_ids,
        }
    } else {
        DomainEvent::AgreementUpdated {
            coaching_session_id,
            agreement: payload,
            notify_user_ids,
        }
    };
    event_publisher.publish(event).await;
}

/// Creates an agreement and publishes `AgreementCreated` to both session participants.
pub async fn create(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    agreement_model: Model,
    user_id: Id,
) -> Result<Model, Error> {
    let agreement = entity_api::agreement::create(db, agreement_model, user_id).await?;
    publish_agreement_changed(db, event_publisher, &agreement, true).await;
    Ok(agreement)
}

/// Updates an agreement and publishes `AgreementUpdated`.
pub async fn update(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
    model: Model,
) -> Result<Model, Error> {
    let agreement = entity_api::agreement::update(db, id, model).await?;
    publish_agreement_changed(db, event_publisher, &agreement, false).await;
    Ok(agreement)
}

/// Deletes an agreement and publishes `AgreementDeleted`. Captures the session id before deletion.
pub async fn delete_by_id(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    id: Id,
) -> Result<(), Error> {
    let coaching_session_id = entity_api::agreement::find_by_id(db, id)
        .await?
        .coaching_session_id;
    entity_api::agreement::delete_by_id(db, id).await?;
    if let Some(notify_user_ids) = agreement_notify_user_ids(db, coaching_session_id).await {
        event_publisher
            .publish(DomainEvent::AgreementDeleted {
                coaching_session_id,
                agreement_id: id,
                notify_user_ids,
            })
            .await;
    }
    Ok(())
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use crate::test_support::recording_publisher;
    use crate::{coaching_relationships, coaching_sessions};
    use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};

    fn agreement_model(coaching_session_id: Id) -> Model {
        let now = chrono::Utc::now().fixed_offset();
        Model {
            id: Id::new_v4(),
            coaching_session_id,
            body: Some("We agree to X".to_string()),
            user_id: Id::new_v4(),
            created_at: now,
            updated_at: now,
        }
    }

    fn session_with_relationship(
        coaching_session_id: Id,
    ) -> (coaching_sessions::Model, coaching_relationships::Model) {
        let now = chrono::Utc::now().fixed_offset();
        let relationship_id = Id::new_v4();
        let session = coaching_sessions::Model {
            id: coaching_session_id,
            coaching_relationship_id: relationship_id,
            coaching_session_series_id: None,
            collab_document_name: None,
            date: now.naive_utc(),
            duration_minutes: 60,
            title: None,
            meeting_url: None,
            provider: None,
            created_at: now,
            updated_at: now,
            hydrated_at: None,
        };
        let relationship = coaching_relationships::Model {
            id: relationship_id,
            organization_id: Id::new_v4(),
            coach_id: Id::new_v4(),
            coachee_id: Id::new_v4(),
            slug: "test-slug".to_string(),
            created_at: now,
            updated_at: now,
        };
        (session, relationship)
    }

    #[tokio::test]
    async fn create_publishes_agreement_created() {
        let session_id = Id::new_v4();
        let agreement = agreement_model(session_id);
        let (publisher, events) = recording_publisher();

        // create (INSERT RETURNING) → participant lookup.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![agreement.clone()]])
            .append_query_results(vec![vec![session_with_relationship(session_id)]])
            .into_connection();

        let result = create(&db, &publisher, agreement.clone(), agreement.user_id).await;

        assert!(result.is_ok());
        let recorded = events.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        match &recorded[0] {
            DomainEvent::AgreementCreated {
                coaching_session_id,
                notify_user_ids,
                ..
            } => {
                assert_eq!(*coaching_session_id, session_id);
                assert_eq!(notify_user_ids.len(), 2, "coach + coachee");
            }
            other => panic!("expected AgreementCreated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn update_publishes_agreement_updated() {
        let session_id = Id::new_v4();
        let agreement = agreement_model(session_id);
        let (publisher, events) = recording_publisher();

        // update: find_by_id → UPDATE RETURNING → participant lookup.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![agreement.clone()]])
            .append_query_results(vec![vec![agreement.clone()]])
            .append_query_results(vec![vec![session_with_relationship(session_id)]])
            .into_connection();

        let result = update(&db, &publisher, agreement.id, agreement.clone()).await;

        assert!(result.is_ok());
        let recorded = events.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert!(matches!(
            &recorded[0],
            DomainEvent::AgreementUpdated { coaching_session_id, .. } if *coaching_session_id == session_id
        ));
    }

    #[tokio::test]
    async fn delete_by_id_publishes_agreement_deleted() {
        let session_id = Id::new_v4();
        let agreement = agreement_model(session_id);
        let (publisher, events) = recording_publisher();

        // delete: wrapper find_by_id → entity delete_by_id (find_by_id + DELETE) → participant lookup.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![agreement.clone()]])
            .append_query_results(vec![vec![agreement.clone()]])
            .append_exec_results(vec![MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_query_results(vec![vec![session_with_relationship(session_id)]])
            .into_connection();

        let result = delete_by_id(&db, &publisher, agreement.id).await;

        assert!(result.is_ok());
        let recorded = events.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        match &recorded[0] {
            DomainEvent::AgreementDeleted {
                coaching_session_id,
                agreement_id,
                notify_user_ids,
            } => {
                assert_eq!(*coaching_session_id, session_id);
                assert_eq!(*agreement_id, agreement.id);
                assert_eq!(notify_user_ids.len(), 2);
            }
            other => panic!("expected AgreementDeleted, got {other:?}"),
        }
    }
}
