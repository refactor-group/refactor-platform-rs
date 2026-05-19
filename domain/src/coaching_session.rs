use crate::coaching_sessions::Model;
use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use crate::gateway::tiptap::TiptapDocument;
use crate::provider::MeetingProperties;
use crate::Id;
use chrono::{DurationRound, NaiveDateTime, TimeDelta};
use entity_api::{
    coaching_relationship, coaching_session, coaching_session_goal, coaching_sessions, mutate,
    organization, query,
    query::{IntoQueryFilterMap, QuerySort},
};
use log::*;
use sea_orm::{DatabaseConnection, IntoActiveModel, TransactionTrait};
use service::config::Config;

pub use entity_api::coaching_session::{
    find_by_id, find_by_user, find_by_user_with_includes, EnrichedSession, IncludeOptions,
    SessionQueryOptions,
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

    maybe_attach_meeting_url(
        db,
        config,
        &mut coaching_session_model,
        coaching_relationship.coach_id,
    )
    .await?;

    let tiptap = TiptapDocument::new(config).await?;
    tiptap.create(&document_name).await?;

    // Wrap all DB writes in a transaction so the session and its goal links
    // succeed or fail atomically. If the transaction fails, compensate by
    // deleting the Tiptap document we just created.
    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;
    let result: Result<Model, Error> = async {
        let session = coaching_session::create(&txn, coaching_session_model).await?;

        let linked = coaching_session_goal::link_in_progress_goals_to_session(
            &txn,
            session.coaching_relationship_id,
            session.id,
        )
        .await?;
        debug!(
            "Linked {linked} in-progress goal(s) to session {}",
            session.id
        );

        txn.commit().await.map_err(entity_api::error::Error::from)?;
        Ok(session)
    }
    .await;

    if result.is_err() {
        if let Err(e) = tiptap.delete(&document_name).await {
            warn!("Failed to clean up Tiptap document '{document_name}' after DB error: {e}");
        }
    }

    result
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

/// If a provider is specified on the session, attempt to attach a meeting URL. First checks
/// if an existing meeting URL can be reused (for providers with persistent URLs), then
/// falls back to creating a new meeting space via OAuth credentials.
async fn maybe_attach_meeting_url(
    db: &DatabaseConnection,
    config: &Config,
    coaching_session_model: &mut Model,
    coach_id: Id,
) -> Result<(), Error> {
    if let Some(provider) = &coaching_session_model.provider {
        if let Some(url) = find_reusable_meeting_url(
            db,
            coaching_session_model.coaching_relationship_id,
            provider,
        )
        .await?
        {
            coaching_session_model.meeting_url = Some(url);
            return Ok(());
        }

        let credentials =
            crate::oauth_connection::find_by_user_and_provider(db, coach_id, *provider).await?;

        if let Some(credentials) = credentials {
            let meeting_url = create_meeting_url(
                db,
                config,
                coach_id,
                provider,
                &coaching_session_model.date,
                credentials.external_account_id,
            )
            .await?;
            coaching_session_model.meeting_url = Some(meeting_url);
        }
    }
    Ok(())
}

/// For providers with persistent meeting URLs, look up an existing meeting URL from the
/// same coaching relationship. Returns `None` for providers with time-bound meetings.
async fn find_reusable_meeting_url(
    db: &DatabaseConnection,
    coaching_relationship_id: Id,
    provider: &crate::provider::Provider,
) -> Result<Option<String>, Error> {
    if !provider.has_persistent_meeting_urls() {
        return Ok(None);
    }

    let url = coaching_session::find_meeting_url_by_relationship_and_provider(
        db,
        coaching_relationship_id,
        *provider,
    )
    .await?;

    debug!(
        "Reusable {} meeting URL for coaching relationship {}: {:?}",
        provider, coaching_relationship_id, url
    );

    Ok(url)
}

/// Create a meeting URL for the given provider using the coach's OAuth connection.
async fn create_meeting_url(
    db: &DatabaseConnection,
    config: &Config,
    coach_id: Id,
    provider: &crate::provider::Provider,
    start_time: &NaiveDateTime,
    external_account_id: Option<String>,
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
        crate::provider::Provider::Zoom => {
            let external_account_id = external_account_id.ok_or_else(|| {
                warn!("Zoom oauth connection for does not have an external_account_id");
                Error {
                    source: None,
                    error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
                }
            })?;

            let client = crate::gateway::zoom::Client::new(&access_token, config.zoom_api_url())?;

            let meeting = client
                .create_meeting(start_time, &external_account_id)
                .await?;

            info!(
                "Created Zoom meeting {} for coaching session",
                meeting.join_url,
            );

            Ok(meeting.join_url)
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
        coaching_relationships, coaching_sessions, goals, oauth_connections, organizations,
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

        // Queries: relationship SELECT, organization SELECT, session INSERT,
        // in-progress goals SELECT (for link_in_progress_goals_to_session).
        // No oauth_connections query because provider is None.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![relationship.clone()]])
            .append_query_results(vec![vec![org.clone()]])
            .append_query_results(vec![vec![session.clone()]])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .into_connection();

        let config = test_config(&server.url());
        let result = create(&db, &config, session.clone()).await?;

        assert!(result.meeting_url.is_none());
        Ok(())
    }

    /// When provider is set, credentials exist, and a previous session in the same
    /// relationship already has a meeting URL for that provider, the existing URL is
    /// reused instead of creating a new Google Meet space.
    #[tokio::test]
    async fn create_with_provider_reuses_existing_meeting_url() -> Result<(), Error> {
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

        // An older session in the same relationship that already has a Google Meet URL
        let existing_session_with_url = coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship.id,
            collab_document_name: Some("old-doc".to_string()),
            date: chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap().into(),
            meeting_url: Some("https://meet.google.com/existing-url".to_string()),
            provider: Some(Provider::Google),
            created_at: chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .unwrap()
                .into(),
            updated_at: chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
                .unwrap()
                .into(),
        };

        // The session as the DB would return it after INSERT (with the reused meeting URL)
        let saved_session = coaching_sessions::Model {
            meeting_url: Some("https://meet.google.com/existing-url".to_string()),
            ..session.clone()
        };

        // Query sequence:
        // 1. relationship SELECT
        // 2. organization SELECT
        // 3. find_meeting_url_by_relationship_and_provider → returns existing session
        //    (no oauth lookup or Meet API call needed!)
        // 4. session INSERT → returns saved_session with the reused meeting_url
        // 5. in-progress goals SELECT
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![relationship.clone()]])
            .append_query_results(vec![vec![org.clone()]])
            .append_query_results(vec![vec![existing_session_with_url]])
            .append_query_results(vec![vec![saved_session]])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .into_connection();

        let config = test_config(&server.url());
        let result = create(&db, &config, session).await?;

        assert_eq!(
            result.meeting_url,
            Some("https://meet.google.com/existing-url".to_string())
        );
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

        // 6 queries: relationship, organization,
        // find_meeting_url (empty = no reusable URL),
        // oauth_connection (empty = no credentials),
        // session INSERT, in-progress goals SELECT.
        // No Google Meet API call should occur.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![relationship.clone()]])
            .append_query_results(vec![vec![org.clone()]])
            .append_query_results::<coaching_sessions::Model, Vec<coaching_sessions::Model>, _>(
                vec![vec![]],
            )
            .append_query_results::<oauth_connections::Model, Vec<oauth_connections::Model>, _>(
                vec![vec![]],
            )
            .append_query_results(vec![vec![session.clone()]])
            .append_query_results(vec![Vec::<goals::Model>::new()])
            .into_connection();

        let config = test_config(&server.url());
        let result = create(&db, &config, session.clone()).await?;

        assert!(result.meeting_url.is_none());
        Ok(())
    }
}
