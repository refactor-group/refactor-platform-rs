use crate::coaching_sessions::Model;
use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use crate::gateway::tiptap::client as tiptap_client;
use crate::Id;
use chrono::{DurationRound, TimeDelta};
use entity_api::{
    coaching_relationship, coaching_session, coaching_sessions, mutate, organization, query,
    query::IntoQueryFilterMap,
};
use log::*;
use sea_orm::{DatabaseConnection, IntoActiveModel};
use serde_json::json;
use service::config::Config;

pub use entity_api::coaching_session::{find_by_id, find_by_id_with_coaching_relationship};

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
    coaching_session_model.collab_document_name = Some(document_name.clone());
    let tiptap_url = config.tiptap_url().ok_or_else(|| {
        warn!("Failed to get Tiptap URL from config");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other),
        }
    })?;
    let full_url = format!("{}/api/documents/{}?format=json", tiptap_url, document_name);
    let client = tiptap_client(config).await?;

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

pub async fn find_by(
    db: &DatabaseConnection,
    params: impl IntoQueryFilterMap,
) -> Result<Vec<Model>, Error> {
    let coaching_sessions = query::find_by::<coaching_sessions::Entity, coaching_sessions::Column>(
        db,
        params.into_query_filter_map(),
    )
    .await?;

    Ok(coaching_sessions)
}

pub async fn update(
    db: &DatabaseConnection,
    id: Id,
    params: impl mutate::IntoUpdateMap + std::fmt::Debug,
) -> Result<Model, Error> {
    let coaching_session = coaching_session::find_by_id(db, id).await?;
    let active_model = coaching_session.into_active_model();
    Ok(
        mutate::update::<coaching_sessions::ActiveModel, coaching_sessions::Column>(
            db,
            active_model,
            params.into_update_map(),
        )
        .await?,
    )
}
