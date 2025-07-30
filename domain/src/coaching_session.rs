use crate::coaching_sessions::Model;
use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use crate::gateway::tiptap::TiptapDocument;
use crate::Id;
use chrono::{DurationRound, NaiveDateTime, TimeDelta};
use entity_api::{
    coaching_relationship, coaching_session, coaching_sessions, mutate, organization, query,
    query::IntoQueryFilterMap,
};
use sea_orm::Order;
use log::*;
use sea_orm::{DatabaseConnection, IntoActiveModel};
use service::config::Config;

pub use entity_api::coaching_session::{find_by_id, find_by_id_with_coaching_relationship};

#[derive(Debug, Clone)]
struct SessionDate(NaiveDateTime);

impl SessionDate {
    fn new(date: NaiveDateTime) -> Result<Self, Error> {
        let truncated = date.duration_trunc(TimeDelta::minutes(1)).map_err(|err| {
            warn!("Failed to truncate date_time: {err:?}");
            Error {
                source: Some(Box::new(err)),
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                    "Failed to truncate date_time".to_string(),
                )),
            }
        })?;
        Ok(Self(truncated))
    }

    fn into_inner(self) -> NaiveDateTime {
        self.0
    }
}

pub async fn create(
    db: &DatabaseConnection,
    config: &Config,
    mut coaching_session_model: Model,
) -> Result<Model, Error> {
    let coaching_relationship =
        coaching_relationship::find_by_id(db, coaching_session_model.coaching_relationship_id)
            .await?;
    let organization = organization::find_by_id(db, coaching_relationship.organization_id).await?;

    coaching_session_model.date = SessionDate::new(coaching_session_model.date)?.into_inner();

    let document_name = generate_document_name(&organization.slug, &coaching_relationship.slug);
    info!("Attempting to create Tiptap document with name: {document_name}");
    coaching_session_model.collab_document_name = Some(document_name.clone());

    let tiptap = TiptapDocument::new(config).await?;
    tiptap.create(&document_name).await?;

    Ok(coaching_session::create(db, coaching_session_model).await?)
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

pub async fn find_by_with_sort<P>(
    db: &DatabaseConnection,
    params: P,
) -> Result<Vec<Model>, Error>
where
    P: IntoQueryFilterMap + CoachingSessionSortParams,
{
    // Extract sort parameters before consuming params
    let sort_column = params.get_sort_column();
    let sort_order = params.get_sort_order();
    let query_filter_map = params.into_query_filter_map();

    let coaching_sessions = query::find_by_with_sort::<coaching_sessions::Entity, coaching_sessions::Column>(
        db,
        query_filter_map,
        sort_column,
        sort_order,
    )
    .await?;

    Ok(coaching_sessions)
}

pub trait CoachingSessionSortParams {
    fn get_sort_column(&self) -> Option<coaching_sessions::Column>;
    fn get_sort_order(&self) -> Option<Order>;
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

pub async fn delete(db: &DatabaseConnection, config: &Config, id: Id) -> Result<(), Error> {
    let coaching_session = find_by_id(db, id).await?;
    let document_name = coaching_session.collab_document_name.ok_or_else(|| {
        warn!("Failed to get document name from coaching session");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Failed to get document name from coaching session".to_string(),
            )),
        }
    })?;

    let tiptap = TiptapDocument::new(config).await?;
    tiptap.delete(&document_name).await?;

    coaching_session::delete(db, id).await?;
    Ok(())
}

fn generate_document_name(organization_slug: &str, relationship_slug: &str) -> String {
    format!(
        "{}.{}.{}-v0",
        organization_slug,
        relationship_slug,
        Id::new_v4()
    )
}
