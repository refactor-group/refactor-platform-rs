use crate::coaching_sessions::Model;
use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use crate::gateway::tiptap::TiptapDocument;
use crate::Id;
use chrono::{DurationRound, NaiveDateTime, TimeDelta};
use entity_api::{
    coaching_relationship, coaching_session, coaching_sessions, mutate, organization, query,
    query::{IntoQueryFilterMap, QuerySort},
};
use log::*;
use sea_orm::{DatabaseConnection, IntoActiveModel};
use service::config::Config;

pub use entity_api::coaching_session::{
    find_by_id, find_by_user_with_includes, EnrichedSession, IncludeOptions, SessionQueryOptions,
};

/// Wraps the entity_api function to convert `entity_api::Error` into `domain::Error`,
/// keeping the web layer from depending on entity_api error types directly.
pub async fn find_by_id_with_coaching_relationship(
    db: &DatabaseConnection,
    id: Id,
) -> Result<(Model, crate::coaching_relationships::Model), Error> {
    Ok(coaching_session::find_by_id_with_coaching_relationship(db, id).await?)
}

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

    maybe_attach_meeting_url(db, config, &mut coaching_session_model, coaching_relationship.coach_id).await?;

    let tiptap = TiptapDocument::new(config).await?;
    tiptap.create(&document_name).await?;

    Ok(coaching_session::create(db, coaching_session_model).await?)
}

pub async fn find_by<P>(db: &DatabaseConnection, params: P) -> Result<Vec<Model>, Error>
where
    P: IntoQueryFilterMap + QuerySort<coaching_sessions::Column>,
{
    let coaching_sessions =
        query::find_by::<coaching_sessions::Entity, coaching_sessions::Column, P>(db, params)
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

/// If a provider is specified on the session, attempt to create a meeting space if the coach has
/// OAuth credentials. If no credentials exist, skip meeting creation and proceed normally.
async fn maybe_attach_meeting_url(
    db: &DatabaseConnection,
    config: &Config,
    coaching_session_model: &mut Model,
    coach_id: Id,
) -> Result<(), Error> {
    if let Some(provider) = &coaching_session_model.provider {
        let has_credentials = crate::oauth_connection::find_by_user_and_provider(
            db,
            coach_id,
            *provider,
        )
        .await?
        .is_some();

        if has_credentials {
            let meeting_url = create_meeting_url(db, config, coach_id, provider).await?;
            coaching_session_model.meeting_url = Some(meeting_url);
        }
    }
    Ok(())
}

/// Create a meeting URL for the given provider using the coach's OAuth connection.
async fn create_meeting_url(
    db: &DatabaseConnection,
    config: &Config,
    coach_id: Id,
    provider: &crate::provider::Provider,
) -> Result<String, Error> {
    let access_token =
        crate::oauth_connection::get_valid_access_token(db, config, coach_id, *provider).await?;

    match provider {
        crate::provider::Provider::Google => {
            let client = crate::gateway::google_meet::Client::new(
                &access_token,
                config.google_meet_api_url(),
            )?;
            let space = client.create_space().await?;

            info!(
                "Created Google Meet {} for coaching session",
                space.meeting_code,
            );

            Ok(space.meeting_uri)
        }
    }
}

fn generate_document_name(organization_slug: &str, relationship_slug: &str) -> String {
    format!(
        "{}.{}.{}-v0",
        organization_slug,
        relationship_slug,
        Id::new_v4()
    )
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use crate::{
        coaching_relationships, coaching_sessions, oauth_connections, organizations,
        provider::Provider,
    };
    use mockito::Server;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use service::config::Config;

    fn test_organization() -> organizations::Model {
        let now = chrono::Utc::now();
        organizations::Model {
            id: Id::new_v4(),
            name: "Test Org".to_string(),
            logo: None,
            slug: "test-org".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn test_coaching_relationship(
        coach_id: Id,
        organization_id: Id,
    ) -> coaching_relationships::Model {
        let now = chrono::Utc::now();
        coaching_relationships::Model {
            id: Id::new_v4(),
            organization_id,
            coach_id,
            coachee_id: Id::new_v4(),
            slug: "test-slug".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn test_session(relationship_id: Id, provider: Option<Provider>) -> coaching_sessions::Model {
        let now = chrono::Utc::now();
        coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            collab_document_name: None,
            date: chrono::Local::now().naive_utc(),
            meeting_url: None,
            provider,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn test_config(tiptap_url: &str) -> Config {
        Config::from_args([
            "test",
            "--tiptap-auth-key=test-auth-key",
            &format!("--tiptap-url={tiptap_url}"),
        ])
    }

    /// When no provider is set, the oauth credentials lookup is skipped entirely.
    #[tokio::test]
    async fn create_without_provider_skips_oauth_lookup() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        let _tiptap_mock = server
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .create_async()
            .await;

        let org = test_organization();
        let coach_id = Id::new_v4();
        let relationship = test_coaching_relationship(coach_id, org.id);
        let session = test_session(relationship.id, None);

        // Only 3 queries: relationship SELECT, organization SELECT, session INSERT.
        // No oauth_connections query because provider is None.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![relationship.clone()]])
            .append_query_results(vec![vec![org.clone()]])
            .append_query_results(vec![vec![session.clone()]])
            .into_connection();

        let config = test_config(&server.url());
        let result = create(&db, &config, session.clone()).await?;

        assert!(result.meeting_url.is_none());
        Ok(())
    }

    /// When provider is set but the coach has no OAuth credentials, meeting creation
    /// is skipped and the session is created successfully without a meeting URL.
    #[tokio::test]
    async fn create_with_provider_but_no_credentials_skips_meeting_creation() -> Result<(), Error> {
        let mut server = Server::new_async().await;

        let _tiptap_mock = server
            .mock("POST", mockito::Matcher::Any)
            .with_status(200)
            .create_async()
            .await;

        let org = test_organization();
        let coach_id = Id::new_v4();
        let relationship = test_coaching_relationship(coach_id, org.id);
        let session = test_session(relationship.id, Some(Provider::Google));

        // 4 queries: relationship, organization, oauth_connection (empty = no credentials), session INSERT.
        // No Google Meet API call should occur.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![relationship.clone()]])
            .append_query_results(vec![vec![org.clone()]])
            .append_query_results::<oauth_connections::Model, Vec<oauth_connections::Model>, _>(
                vec![vec![]],
            )
            .append_query_results(vec![vec![session.clone()]])
            .into_connection();

        let config = test_config(&server.url());
        let result = create(&db, &config, session.clone()).await?;

        assert!(result.meeting_url.is_none());
        Ok(())
    }
}
