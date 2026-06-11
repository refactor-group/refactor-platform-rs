pub(crate) mod recurrence;

use crate::coaching_relationships;
use crate::coaching_sessions::Model;
use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use crate::events::{DomainEvent, EventPublisher};
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
    find_by_id, find_by_series_id, find_by_user_with_includes, find_counts_by_month_for_user,
    find_participant_ids, CountByMonth, EnrichedSession, IncludeOptions, SessionQueryOptions,
};

use crate::duration::Duration;
pub use recurrence::{expand_recurrence, Frequency, Recurrence, RecurrenceError};

/// Validate a wire-supplied `Option<i16>` duration and return `Option<Duration>`.
/// Out-of-range values propagate through `entity_api::Error` to
/// `DomainErrorKind::Validation` → 422. Lives in domain so the web layer
/// can call it without depending on `entity_api` directly.
pub fn parse_duration_minutes(minutes: Option<i16>) -> Result<Option<Duration>, Error> {
    minutes
        .map(Duration::try_from)
        .transpose()
        .map_err(|out| entity_api::error::Error::from(out).into())
}

/// Wraps the entity_api function to convert `entity_api::Error` into `domain::Error`,
/// keeping the web layer from depending on entity_api error types directly.
pub async fn find_by_id_with_coaching_relationship(
    db: &DatabaseConnection,
    id: Id,
) -> Result<(Model, coaching_relationships::Model), Error> {
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
    event_publisher: &EventPublisher,
    mut coaching_session_model: Model,
    requested_duration: Option<Duration>,
) -> Result<Model, Error> {
    let coaching_relationship =
        coaching_relationship::find_by_id(db, coaching_session_model.coaching_relationship_id)
            .await?;
    let organization = organization::find_by_id(db, coaching_relationship.organization_id).await?;
    let coach_id = coaching_relationship.coach_id;

    coaching_session_model.date = SessionDate::new(coaching_session_model.date)?.into_inner();

    let document_name = generate_document_name(&organization.slug, &coaching_relationship.slug);
    info!("Attempting to create Tiptap document with name: {document_name}");
    coaching_session_model.collab_document_name = Some(document_name.clone());
    coaching_session_model.hydrated_at = Some(chrono::Utc::now().into());

    maybe_attach_meeting_url(db, config, &mut coaching_session_model, coach_id).await?;

    let tiptap = TiptapDocument::new(config).await?;
    tiptap.create(&document_name).await?;

    // Wrap all DB writes in a transaction so the session and its goal links
    // succeed or fail atomically. If the transaction fails, compensate by
    // deleting the Tiptap document we just created.
    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;
    let result: Result<(Model, Vec<Id>), Error> = async {
        let session =
            coaching_session::create(&txn, coaching_session_model, coach_id, requested_duration)
                .await?;

        let linked_goal_ids = coaching_session_goal::link_in_progress_goals_to_session(
            &txn,
            session.coaching_relationship_id,
            session.id,
        )
        .await?;
        debug!(
            "Linked {} in-progress goal(s) to session {}",
            linked_goal_ids.len(),
            session.id
        );

        txn.commit().await.map_err(entity_api::error::Error::from)?;
        Ok((session, linked_goal_ids))
    }
    .await;

    // Publish after commit so subscribers never refetch against goal links
    // that a rolled-back transaction never persisted.
    match result {
        Ok((session, linked_goal_ids)) => {
            publish_goals_linked(
                event_publisher,
                &coaching_relationship,
                session.id,
                &linked_goal_ids,
            )
            .await;
            Ok(session)
        }
        Err(e) => {
            if let Err(cleanup_err) = tiptap.delete(&document_name).await {
                warn!("Failed to clean up Tiptap document '{document_name}' after DB error: {cleanup_err}");
            }
            Err(e)
        }
    }
}

/// Bulk-creates a series of coaching sessions. Provider is intentionally not
/// captured here — every row's `provider` stays NULL until [`ensure_hydrated`]
/// fills it in on first read, using the coach's then-current OAuth connection.
///
/// Every materialized recurring session belongs to a parent
/// `coaching_session_series` row (`series_id`); the [`crate::coaching_session_series`]
/// orchestration module is the only call-site that should reach this helper
/// directly.
pub async fn bulk_create_recurring(
    db: &impl sea_orm::ConnectionTrait,
    coaching_relationship_id: Id,
    coach_id: Id,
    series_id: Id,
    dates: Vec<NaiveDateTime>,
    requested_duration: Option<Duration>,
) -> Result<Vec<Model>, Error> {
    let truncated = dates
        .into_iter()
        .map(|d| SessionDate::new(d).map(SessionDate::into_inner))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(coaching_session::bulk_create_recurring(
        db,
        coaching_relationship_id,
        coach_id,
        series_id,
        truncated,
        requested_duration,
    )
    .await?)
}

/// Runs deferred side-effects (Tiptap doc, meeting URL, in-progress-goal links)
/// on first read, stamping `hydrated_at` so subsequent reads short-circuit.
///
/// Provider is resolved at hydration time from the coach's most-recently-updated
/// OAuth connection — not at session-create time. For recurring series created
/// via [`bulk_create_recurring`], this means changing or reconnecting providers
/// between bulk-create and first read will change which provider the session
/// uses. This is the intended behavior: recurring sessions defer the choice
/// until the coach actually opens them.
pub async fn ensure_hydrated(
    db: &DatabaseConnection,
    config: &Config,
    event_publisher: &EventPublisher,
    session: Model,
) -> Result<Model, Error> {
    if session.hydrated_at.is_some() {
        return Ok(session);
    }

    let session_id = session.id;
    let txn = db.begin().await.map_err(entity_api::error::Error::from)?;

    coaching_session::acquire_advisory_lock(&txn, session_id).await?;

    let mut session = coaching_session::find_by_id(&txn, session_id).await?;
    if session.hydrated_at.is_some() {
        txn.commit().await.map_err(entity_api::error::Error::from)?;
        return Ok(session);
    }

    // Read-only lookups on rows; running them on `db` rather than
    // `&txn` is intentional — the txn is only needed to scope the
    // advisory lock and the final UPDATE.
    let coaching_relationship =
        coaching_relationship::find_by_id(db, session.coaching_relationship_id).await?;
    let organization = organization::find_by_id(db, coaching_relationship.organization_id).await?;

    let document_name = generate_document_name(&organization.slug, &coaching_relationship.slug);
    let tiptap = TiptapDocument::new(config).await?;

    let coach_id = coaching_relationship.coach_id;
    let result: Result<(Model, Vec<Id>), Error> = async {
        tiptap.create(&document_name).await?;
        session.collab_document_name = Some(document_name.clone());

        if let Some(connection) = crate::oauth_connection::find_by_user(db, coach_id).await? {
            session.provider = Some(connection.provider);
            maybe_attach_meeting_url(db, config, &mut session, coach_id).await?;
        }

        let linked_goal_ids = coaching_session_goal::link_in_progress_goals_to_session(
            &txn,
            session.coaching_relationship_id,
            session.id,
        )
        .await?;

        debug!(
            "Linked {} in-progress goal(s) to session {}",
            linked_goal_ids.len(),
            session.id
        );

        let updated = coaching_session::mark_hydrated(&txn, &session).await?;
        txn.commit().await.map_err(entity_api::error::Error::from)?;
        Ok((updated, linked_goal_ids))
    }
    .await;

    // Publish after commit so subscribers never refetch against goal links
    // that a rolled-back transaction never persisted.
    match result {
        Ok((updated, linked_goal_ids)) => {
            publish_goals_linked(
                event_publisher,
                &coaching_relationship,
                updated.id,
                &linked_goal_ids,
            )
            .await;
            Ok(updated)
        }
        Err(e) => {
            if let Err(cleanup_err) = tiptap.delete(&document_name).await {
                warn!(
                    "Failed to clean up Tiptap document '{document_name}' after hydration error: {cleanup_err}"
                );
            }
            Err(e)
        }
    }
}

/// Publishes a `CoachingSessionGoalCreated` event for each goal newly linked to
/// a session during create/hydration, so connected clients refresh the session's
/// goal list. Mirrors the manual link path's notification contract; a no-op when
/// no new links were inserted.
async fn publish_goals_linked(
    event_publisher: &EventPublisher,
    relationship: &coaching_relationships::Model,
    coaching_session_id: Id,
    goal_ids: &[Id],
) {
    if goal_ids.is_empty() {
        return;
    }

    let notify_user_ids = vec![relationship.coach_id, relationship.coachee_id];
    for goal_id in goal_ids {
        event_publisher
            .publish(DomainEvent::CoachingSessionGoalCreated {
                coaching_relationship_id: relationship.id,
                coaching_session_id,
                goal_id: *goal_id,
                notify_user_ids: notify_user_ids.clone(),
            })
            .await;
    }

    debug!(
        "Published {} CoachingSessionGoalCreated event(s) for session {coaching_session_id}",
        goal_ids.len()
    );
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
    let update_map = params.into_update_map();
    // Validate `duration_minutes` if it appears in the patch. The
    // IntoUpdateMap pattern erased its type, so the entity_api boundary is
    // the right place to re-check (1..=480).
    coaching_session::validate_duration_in_update_map(&update_map, "duration_minutes")?;

    let coaching_session = coaching_session::find_by_id(db, id).await?;
    debug!(
        "Domain update coaching_session id={id} relationship_id={} update_map={update_map:?}",
        coaching_session.coaching_relationship_id
    );
    let active_model = coaching_session.into_active_model();
    Ok(
        mutate::update::<coaching_sessions::ActiveModel, coaching_sessions::Column>(
            db,
            active_model,
            update_map,
        )
        .await?,
    )
}

pub async fn delete(db: &DatabaseConnection, config: &Config, id: Id) -> Result<(), Error> {
    let coaching_session = find_by_id(db, id).await?;
    debug!(
        "Domain delete coaching_session id={id} relationship_id={} tiptap_doc={:?}",
        coaching_session.coaching_relationship_id, coaching_session.collab_document_name,
    );
    if let Some(document_name) = coaching_session.collab_document_name {
        let tiptap = TiptapDocument::new(config).await?;
        tiptap.delete(&document_name).await?;
    }

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
    use crate::test_support::recording_publisher;
    use crate::{coaching_sessions, goals, oauth_connections, organizations, provider::Provider};
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
            coaching_session_series_id: None,
            collab_document_name: None,
            date: chrono::Local::now().naive_utc(),
            duration_minutes: crate::duration::Duration::default_minutes(),
            meeting_url: None,
            provider,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: Some(now.into()),
        }
    }

    fn test_config(tiptap_url: &str) -> Config {
        Config::from_args([
            "test",
            "--tiptap-auth-key=test-auth-key",
            &format!("--tiptap-url={tiptap_url}"),
        ])
    }

    /// Creating a session links the relationship's in-progress goals and
    /// publishes one `CoachingSessionGoalCreated` per newly-linked goal,
    /// scoped to the coach and coachee. This is the signal the carry-forward
    /// path previously omitted, leaving clients unaware of auto-linked goals.
    #[tokio::test]
    async fn create_publishes_link_event_for_each_carried_forward_goal() -> Result<(), Error> {
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

        let now = chrono::Utc::now();
        let goal = goals::Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship.id,
            created_in_session_id: None,
            user_id: Id::new_v4(),
            title: Some("Carried-forward goal".to_string()),
            body: None,
            status: entity_api::status::Status::InProgress,
            status_changed_at: None,
            completed_at: None,
            target_date: None,
            created_at: now.into(),
            updated_at: now.into(),
        };
        let link = entity_api::coaching_sessions_goals::Model {
            id: Id::new_v4(),
            coaching_session_id: session.id,
            goal_id: goal.id,
            created_at: now.into(),
            updated_at: now.into(),
        };

        // Queries: relationship, organization, session INSERT, in-progress goals
        // SELECT (one goal), join INSERT...RETURNING (one freshly-linked row).
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![relationship.clone()]])
            .append_query_results(vec![vec![org.clone()]])
            .append_query_results(vec![vec![session.clone()]])
            .append_query_results(vec![vec![goal.clone()]])
            .append_query_results(vec![vec![link.clone()]])
            .into_connection();

        let config = test_config(&server.url());
        let (publisher, recorded) = recording_publisher();
        create(
            &db,
            &config,
            &publisher,
            session.clone(),
            Some(Duration::default()),
        )
        .await?;

        let events = recorded.lock().unwrap();
        assert_eq!(events.len(), 1, "expected one link event, got {events:?}");
        match &events[0] {
            DomainEvent::CoachingSessionGoalCreated {
                coaching_relationship_id,
                coaching_session_id,
                goal_id,
                notify_user_ids,
            } => {
                assert_eq!(*coaching_relationship_id, relationship.id);
                assert_eq!(*coaching_session_id, session.id);
                assert_eq!(*goal_id, goal.id);
                assert_eq!(notify_user_ids, &vec![coach_id, relationship.coachee_id]);
            }
            other => panic!("expected CoachingSessionGoalCreated, got {other:?}"),
        }

        Ok(())
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
        let result = create(
            &db,
            &config,
            &EventPublisher::new(),
            session.clone(),
            Some(Duration::default()),
        )
        .await?;

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
            coaching_session_series_id: None,
            collab_document_name: Some("old-doc".to_string()),
            date: chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap().into(),
            duration_minutes: crate::duration::Duration::default_minutes(),
            meeting_url: Some("https://meet.google.com/existing-url".to_string()),
            provider: Some(Provider::Google),
            created_at: chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap(),
            updated_at: chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap(),
            hydrated_at: Some(
                chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap(),
            ),
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
        let result = create(
            &db,
            &config,
            &EventPublisher::new(),
            session,
            Some(Duration::default()),
        )
        .await?;

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
        let result = create(
            &db,
            &config,
            &EventPublisher::new(),
            session.clone(),
            Some(Duration::default()),
        )
        .await?;

        assert!(result.meeting_url.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn ensure_hydrated_short_circuits_after_lock_when_row_hydrated_concurrently(
    ) -> Result<(), Error> {
        let mut server = Server::new_async().await;
        let config = test_config(&server.url());
        let _tiptap_mock = server
            .mock("POST", mockito::Matcher::Any)
            .with_status(500)
            .expect(0)
            .create_async()
            .await;

        let input = coaching_sessions::Model {
            hydrated_at: None,
            ..test_session(Id::new_v4(), None)
        };
        let hydrated_row = coaching_sessions::Model {
            hydrated_at: Some(chrono::Utc::now().into()),
            ..input.clone()
        };

        // Sequence under the lock: pg_advisory_xact_lock exec → SELECT
        // (re-fetch returns the already-hydrated row) → COMMIT. No further
        // DB calls and no Tiptap traffic.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![sea_orm::MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_query_results(vec![vec![hydrated_row.clone()]])
            .into_connection();

        let result = ensure_hydrated(&db, &config, &EventPublisher::new(), input.clone()).await?;
        assert_eq!(result.id, input.id);
        assert!(result.hydrated_at.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn ensure_hydrated_deletes_tiptap_doc_when_post_create_step_fails() {
        let mut server = Server::new_async().await;
        let config = test_config(&server.url());

        let doc_path = mockito::Matcher::Regex(r"^/api/documents/.+".to_string());
        let _create_mock = server
            .mock("POST", doc_path.clone())
            .with_status(200)
            .expect(1)
            .create_async()
            .await;
        let delete_mock = server
            .mock("DELETE", doc_path)
            .with_status(204)
            .expect(1)
            .create_async()
            .await;

        let org = test_organization();
        let relationship = test_coaching_relationship(Id::new_v4(), org.id);
        let input = coaching_sessions::Model {
            hydrated_at: None,
            ..test_session(relationship.id, None)
        };
        let refetched = input.clone();

        // Sequence: advisory_lock exec → re-fetch (not yet hydrated) → relationship
        // → organization → tiptap POST (200) → oauth_connection::find_by_user
        // returns an error → cleanup DELETE fires.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_exec_results(vec![sea_orm::MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_query_results(vec![vec![refetched]])
            .append_query_results(vec![vec![relationship.clone()]])
            .append_query_results(vec![vec![org]])
            .append_query_errors(vec![sea_orm::DbErr::Custom(
                "simulated oauth lookup failure".to_string(),
            )])
            .into_connection();

        let result = ensure_hydrated(&db, &config, &EventPublisher::new(), input).await;
        assert!(
            result.is_err(),
            "expected ensure_hydrated to surface the error"
        );

        delete_mock.assert_async().await;
    }

    /// Already-hydrated input short-circuits before any DB or HTTP call: the
    /// MockDatabase has no appended results and the function still returns the
    /// input model unchanged.
    #[tokio::test]
    async fn ensure_hydrated_short_circuits_for_already_hydrated_session() -> Result<(), Error> {
        let mut server = Server::new_async().await;
        let config = test_config(&server.url());
        // Asserts Tiptap is never hit — any HTTP call to mockito server would 404.
        let _tiptap_mock = server
            .mock("POST", mockito::Matcher::Any)
            .with_status(500)
            .expect(0)
            .create_async()
            .await;

        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let session = test_session(Id::new_v4(), None);
        assert!(session.hydrated_at.is_some());

        let result = ensure_hydrated(&db, &config, &EventPublisher::new(), session.clone()).await?;
        assert_eq!(result.id, session.id);
        Ok(())
    }

    #[tokio::test]
    async fn bulk_create_recurring_inserts_rows_with_all_lazy_fields_null() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let dates = vec![
            chrono::NaiveDate::from_ymd_opt(2026, 6, 1)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap(),
            chrono::NaiveDate::from_ymd_opt(2026, 6, 8)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap(),
        ];

        let row_template = coaching_sessions::Model {
            hydrated_at: None,
            ..test_session(relationship_id, None)
        };
        let row1 = coaching_sessions::Model {
            id: Id::new_v4(),
            date: dates[0],
            ..row_template.clone()
        };
        let row2 = coaching_sessions::Model {
            id: Id::new_v4(),
            date: dates[1],
            ..row_template
        };

        // Query sequence: 1 bulk INSERT...RETURNING. Existence of the
        // relationship is the caller's (web layer's) responsibility now.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![row1.clone(), row2.clone()]])
            .into_connection();

        let inserted = bulk_create_recurring(
            &db,
            relationship_id,
            Id::new_v4(),
            Id::new_v4(),
            dates,
            Some(Duration::default()),
        )
        .await?;
        assert_eq!(inserted.len(), 2);
        assert!(inserted.iter().all(|s| s.provider.is_none()));
        assert!(inserted.iter().all(|s| s.collab_document_name.is_none()));
        assert!(inserted.iter().all(|s| s.meeting_url.is_none()));
        assert!(inserted.iter().all(|s| s.hydrated_at.is_none()));
        Ok(())
    }

    #[tokio::test]
    async fn delete_unhydrated_session_skips_tiptap_and_succeeds() -> Result<(), Error> {
        let mut server = Server::new_async().await;
        let config = test_config(&server.url());

        let tiptap_mock = server
            .mock("DELETE", mockito::Matcher::Any)
            .with_status(204)
            .expect(0)
            .create_async()
            .await;

        let session = coaching_sessions::Model {
            collab_document_name: None,
            hydrated_at: None,
            ..test_session(Id::new_v4(), None)
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![session.clone()]])
            .append_exec_results(vec![sea_orm::MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .into_connection();

        delete(&db, &config, session.id).await?;

        tiptap_mock.assert_async().await;
        Ok(())
    }
}
