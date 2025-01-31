use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use crate::gateway::tiptap::client as tip_tap_client;
use chrono::{DurationRound, TimeDelta};
use entity::coaching_sessions::Model;
use entity_api::{coaching_relationship, coaching_session, organization};
use log::*;
use sea_orm::DatabaseConnection;
use serde_json::json;
use service::config::Config;

pub async fn create(
    db: &DatabaseConnection,
    config: &Config,
    mut coaching_session_model: Model,
) -> Result<Model, Error> {
    let coaching_relationship =
        coaching_relationship::find_by_id(db, coaching_session_model.coaching_relationship_id)
            .await?;
    let organization = organization::find_by_id(db, coaching_relationship.organization_id).await?;
    // Remove seconds because all coaching_sessions will be scheduled by the minute
    // TODO: we might consider codifying this in the type system at some point.
    let date_time = coaching_session_model
        .date
        .duration_trunc(TimeDelta::minutes(1))
        .map_err(|err| {
            warn!("Failed to truncate date_time: {:?}", err);
            Error {
                source: Some(Box::new(err)),
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other),
            }
        })?;
    coaching_session_model.date = date_time;
    let document_name = format!(
        "{}.{}.{}-v0",
        organization.slug,
        coaching_relationship.slug,
        coaching_session_model.date.and_utc().timestamp()
    );
    info!(
        "Attempting to create Tiptap document with name: {}",
        document_name
    );
    let tip_tap_url = config.tip_tap_url().ok_or_else(|| {
        warn!("Failed to get Tiptap URL from config");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other),
        }
    })?;
    let full_url = format!(
        "{}/api/documents/{}?format=json",
        tip_tap_url, document_name
    );
    let client = tip_tap_client(config).await?;

    let request = client
        .post(full_url)
        .json(&json!({"type": "doc", "content": []}));
    let response = match request.send().await {
        Ok(response) => {
            info!("Tiptap response: {:?}", response);
            response
        }
        Err(e) => {
            warn!("Failed to send request: {:?}", e);
            return Err(e.into());
        }
    };

    // Tiptap's API will return a 200 for successful creation of a new document
    // and will return a 409 if the document already exists. We consider both "successful".
    if response.status().is_success() || response.status().as_u16() == 409 {
        // TODO: Save document_name to record
        Ok(coaching_session::create(db, coaching_session_model).await?)
    } else {
        warn!(
            "Failed to create Tiptap document: {}",
            response.text().await?
        );
        Err(Error {
            source: None,
            error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
        })
    }
}

pub async fn find_by_id(db: &DatabaseConnection, id: entity::Id) -> Result<Option<Model>, Error> {
    Ok(coaching_session::find_by_id(db, id).await?)
}

pub async fn find_by_id_with_coaching_relationship(
    db: &DatabaseConnection,
    id: entity::Id,
) -> Result<(Model, entity::coaching_relationships::Model), Error> {
    Ok(coaching_session::find_by_id_with_coaching_relationship(db, id).await?)
}

pub async fn find_by(
    db: &DatabaseConnection,
    params: std::collections::HashMap<String, String>,
) -> Result<Vec<Model>, Error> {
    Ok(coaching_session::find_by(db, params).await?)
}
