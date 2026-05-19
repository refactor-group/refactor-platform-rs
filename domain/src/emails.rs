use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use log::*;
use sea_orm::DatabaseConnection;
use service::config::Config;

use crate::{
    actions, coaching_relationship, coaching_session, coaching_sessions,
    error::Error,
    error::{DomainErrorKind, InternalErrorKind},
    gateway::resend::{Client as ResendClient, SendEmailRequestBuilder},
    goal, organization, organizations, user, users, Id,
};

/// Trait for email notifications that need common config prerequisites.
///
/// Implementors declare which template ID to use and a human-readable name
/// for log messages. The trait provides default implementations for resolving
/// the template ID and frontend base URL from config with consistent error handling.
trait EmailNotification {
    /// Return the template ID from config for this notification type.
    fn template_id(config: &Config) -> Option<String>;

    /// Human-readable name used in log/error messages (e.g. "session scheduled").
    fn notification_name() -> &'static str;

    /// Resolve the template ID from config, or return a config error.
    fn resolve_template_id(config: &Config) -> Result<String, Error> {
        Self::template_id(config).ok_or_else(|| {
            error!(
                "{} email template ID not configured",
                Self::notification_name()
            );
            Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
            }
        })
    }

    /// Return the URL path template from config for this notification type, if any.
    /// The template may contain `{session_id}` as a placeholder.
    fn url_path_template(_config: &Config) -> Option<String> {
        None
    }

    /// Resolve the frontend base URL from config, or return a config error.
    fn resolve_base_url(config: &Config) -> Result<String, Error> {
        config.frontend_base_url().ok_or_else(|| {
            error!(
                "Frontend base URL not configured, cannot send {} notification",
                Self::notification_name()
            );
            Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
            }
        })
    }
}

struct SessionScheduled;
impl EmailNotification for SessionScheduled {
    fn template_id(config: &Config) -> Option<String> {
        config.session_scheduled_email_template_id()
    }
    fn notification_name() -> &'static str {
        "session scheduled"
    }
    fn url_path_template(config: &Config) -> Option<String> {
        Some(config.session_scheduled_email_url_path().to_owned())
    }
}

struct RecurringSessionsScheduled;
impl EmailNotification for RecurringSessionsScheduled {
    fn template_id(config: &Config) -> Option<String> {
        config.recurring_sessions_scheduled_email_template_id()
    }
    fn notification_name() -> &'static str {
        "recurring sessions scheduled"
    }
    fn url_path_template(config: &Config) -> Option<String> {
        Some(config.session_scheduled_email_url_path().to_owned())
    }
}

struct ActionAssigned;
impl EmailNotification for ActionAssigned {
    fn template_id(config: &Config) -> Option<String> {
        config.action_assigned_email_template_id()
    }
    fn notification_name() -> &'static str {
        "action assigned"
    }
    fn url_path_template(config: &Config) -> Option<String> {
        Some(config.action_assigned_email_url_path().to_owned())
    }
}

struct WelcomeEmail;
impl EmailNotification for WelcomeEmail {
    fn template_id(config: &Config) -> Option<String> {
        config.welcome_email_template_id()
    }
    fn notification_name() -> &'static str {
        "welcome"
    }
    fn url_path_template(config: &Config) -> Option<String> {
        Some(config.magic_link_email_url_path().to_owned())
    }
}

/// Create a magic link token and send a welcome email to a user.
///
/// `inviter` is the user who triggered the invite (typically the coach or
/// org admin); their name is interpolated into the email body so the
/// recipient sees who added them.
///
/// Returns an error if token creation or email delivery fails.
pub async fn create_and_send_welcome_email(
    db: &sea_orm::DatabaseConnection,
    config: &Config,
    user: &users::Model,
    inviter: &users::Model,
) -> Result<(), Error> {
    let raw_token = crate::magic_link_token::create_magic_link(db, user.id, config).await?;
    send_welcome_email(config, user, inviter, &raw_token).await
}

/// Create a magic link token and send a best-effort welcome email to a newly created user.
///
/// `inviter` is the user who triggered the invite (typically the coach or
/// org admin); their name is interpolated into the email body.
///
/// Both token creation and email delivery are best-effort — errors are logged
/// internally and never propagate to the caller.
pub async fn notify_welcome_email(
    db: &sea_orm::DatabaseConnection,
    config: &Config,
    user: &users::Model,
    inviter: &users::Model,
) {
    match crate::magic_link_token::create_magic_link(db, user.id, config).await {
        Ok(raw_token) => {
            if let Err(e) = send_welcome_email(config, user, inviter, &raw_token).await {
                warn!("Failed to send welcome email to {}: {e:?}", user.email);
            }
        }
        Err(e) => {
            warn!(
                "Failed to create magic link token for user {}: {e:?}",
                user.id
            );
        }
    }
}

/// Build and send the welcome email to a single user.
async fn send_welcome_email(
    config: &Config,
    user: &users::Model,
    inviter: &users::Model,
    magic_link_token: &str,
) -> Result<(), Error> {
    info!(
        "Initiating welcome email for user: {} ({})",
        user.email, user.id
    );

    let email_config = ResolvedEmailConfig::new::<WelcomeEmail>(config).await?;
    info!("Using template ID: {}", email_config.template_id);

    let magic_link_url = email_config
        .session_url_builder
        .as_ref()
        .map(|b| b.build(TOKEN_PLACEHOLDER, magic_link_token))
        .unwrap_or_default();

    let coach_full_name = format!("{} {}", inviter.first_name, inviter.last_name);

    debug!("Preparing template variables for {}", user.email);

    let email_request = SendEmailRequestBuilder::new()
        .from(FROM_ADDRESS)
        .to_with_name(
            &user.email,
            format!("{} {}", user.first_name, user.last_name),
        )
        .template_id(&email_config.template_id)
        .add_variable("first_name", &user.first_name)
        .add_variable("last_name", &user.last_name)
        .add_variable("coach_first_name", &inviter.first_name)
        .add_variable("coach_full_name", &coach_full_name)
        .add_variable("magic_link_url", &magic_link_url)
        .build()
        .await?;
    debug!("Email request created for {}", user.email);

    email_config.client.send_email(email_request).await
}

/// Format a NaiveDateTime (assumed UTC) in the recipient's timezone.
/// Falls back to UTC formatting if the timezone string is invalid.
fn format_session_date_time(date: NaiveDateTime, timezone: &str) -> (String, String) {
    let utc_dt = Utc.from_utc_datetime(&date);

    match timezone.parse::<Tz>() {
        Ok(tz) => {
            let local_dt = utc_dt.with_timezone(&tz);
            let date_str = local_dt.format("%A, %B %-d, %Y").to_string();
            let time_str = local_dt.format("%-I:%M %p").to_string();
            (date_str, time_str)
        }
        Err(_) => {
            warn!("Invalid timezone '{timezone}', falling back to UTC");
            let date_str = utc_dt.format("%A, %B %-d, %Y").to_string();
            let time_str = format!("{} UTC", utc_dt.format("%-I:%M %p"));
            (date_str, time_str)
        }
    }
}

const TOKEN_PLACEHOLDER: &str = "{token}";
const SESSION_ID_PLACEHOLDER: &str = "{session_id}";

/// The `From:` address used for every transactional email sent through this module.
/// Kept on the `mail.` subdomain so production DMARC/SPF/DKIM records for the
/// `myrefactor.com` apex aren't affected by Resend's sending infrastructure.
const FROM_ADDRESS: &str = "hello@mail.myrefactor.com";

/// Groups the base URL and path template for building session links in emails.
struct SessionUrlBuilder {
    base_url: String,
    path_template: String,
}

impl SessionUrlBuilder {
    fn build(&self, placeholder: &str, value: &str) -> String {
        let path = self.path_template.replace(placeholder, value);
        format!("{}{}", self.base_url, path)
    }
}

/// Pre-resolved Resend configuration, created once per notification
/// so that config errors propagate before per-recipient sends begin.
struct ResolvedEmailConfig {
    client: ResendClient,
    template_id: String,
    /// `None` for notification types that don't include app links (e.g. welcome emails).
    session_url_builder: Option<SessionUrlBuilder>,
}

impl ResolvedEmailConfig {
    /// Resolve all Resend configuration for the given notification type.
    ///
    /// Creates the HTTP client and resolves the template ID via the
    /// `EmailNotification` trait. URL support is derived from the trait:
    /// if `url_path_template` returns `Some`, the base URL is also resolved.
    async fn new<N: EmailNotification>(config: &Config) -> Result<Self, Error> {
        let client = ResendClient::new(config).await?;
        let template_id = N::resolve_template_id(config)?;

        let session_url_builder = match N::url_path_template(config) {
            Some(path_template) => Some(SessionUrlBuilder {
                base_url: N::resolve_base_url(config)?,
                path_template,
            }),
            None => None,
        };

        Ok(Self {
            client,
            template_id,
            session_url_builder,
        })
    }

    /// Build a full session URL from the resolved base URL and path template.
    ///
    /// Returns a config error if this notification type does not support
    /// session URLs (i.e., its `url_path_template` returned `None`).
    fn build_session_url(&self, session_id: &Id) -> Result<String, Error> {
        self.session_url_builder
            .as_ref()
            .map(|b| b.build(SESSION_ID_PLACEHOLDER, &session_id.to_string()))
            .ok_or_else(|| {
                error!("Cannot build session URL: notification type has no URL template");
                Error {
                    source: None,
                    error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
                }
            })
    }
}

/// Send a session-scheduled notification email to a single recipient.
/// This is called once per recipient (coach and coachee each get their own email).
async fn send_session_email_to_recipient(
    email_config: &ResolvedEmailConfig,
    recipient: &users::Model,
    other_user: &users::Model,
    other_user_role: &str,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
) -> Result<(), Error> {
    let (session_date, session_time) = format_session_date_time(session.date, &recipient.timezone);
    let session_url = email_config.build_session_url(&session.id)?;

    let email_request = SendEmailRequestBuilder::new()
        .from(FROM_ADDRESS)
        .to_with_name(
            &recipient.email,
            format!("{} {}", recipient.first_name, recipient.last_name),
        )
        .template_id(&email_config.template_id)
        .add_variable("first_name", &recipient.first_name)
        .add_variable("other_user_first_name", &other_user.first_name)
        .add_variable("other_user_last_name", &other_user.last_name)
        .add_variable("other_user_role", other_user_role)
        .add_variable("organization_name", &organization.name)
        .add_variable("session_date", &session_date)
        .add_variable("session_time", &session_time)
        .add_variable("session_url", &session_url)
        .build()
        .await?;

    email_config.client.send_email(email_request).await
}

/// Send session-scheduled notification emails to both coach and coachee.
async fn send_session_scheduled_email(
    config: &Config,
    coach: &users::Model,
    coachee: &users::Model,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
) -> Result<(), Error> {
    info!(
        "Initiating session scheduled emails for session: {} (coach: {}, coachee: {})",
        session.id, coach.email, coachee.email
    );

    let email_config = ResolvedEmailConfig::new::<SessionScheduled>(config).await?;

    // Email to coachee: "Your coach, ... has a session with you"
    if let Err(e) = send_session_email_to_recipient(
        &email_config,
        coachee,
        coach,
        "coach",
        session,
        organization,
    )
    .await
    {
        warn!(
            "Failed to send session scheduled email to coachee {}: {e:?}",
            coachee.email
        );
    }

    // Email to coach: "Your coachee, ... has a session with you"
    if let Err(e) = send_session_email_to_recipient(
        &email_config,
        coach,
        coachee,
        "coachee",
        session,
        organization,
    )
    .await
    {
        warn!(
            "Failed to send session scheduled email to coach {}: {e:?}",
            coach.email
        );
    }

    Ok(())
}

/// Context for an action-assigned email, bundling the action-specific data
/// that the email needs for template variables.
struct ActionEmailContext<'a> {
    action_body: &'a str,
    due_by: Option<DateTime<FixedOffset>>,
    session_id: Id,
    organization: &'a organizations::Model,
    goal: Option<&'a str>,
}

/// Send action-assigned notification emails to all assignees.
async fn send_action_assigned_email(
    config: &Config,
    assignees: &[users::Model],
    assigner: &users::Model,
    ctx: &ActionEmailContext<'_>,
) -> Result<(), Error> {
    info!(
        "Initiating action assigned emails for {} assignee(s) (assigner: {})",
        assignees.len(),
        assigner.email
    );

    let email_config = ResolvedEmailConfig::new::<ActionAssigned>(config).await?;
    let session_url = email_config.build_session_url(&ctx.session_id)?;

    for assignee in assignees {
        let due_date_str = match ctx.due_by {
            Some(dt) => {
                let (date_str, _) = format_session_date_time(dt.naive_utc(), &assignee.timezone);
                date_str
            }
            None => "No due date set".to_string(),
        };

        let email_request = SendEmailRequestBuilder::new()
            .from(FROM_ADDRESS)
            .to_with_name(
                &assignee.email,
                format!("{} {}", assignee.first_name, assignee.last_name),
            )
            .template_id(&email_config.template_id)
            .add_variable("first_name", &assignee.first_name)
            .add_variable("action_body", ctx.action_body)
            .add_variable("due_date", &due_date_str)
            .add_variable("assigner_first_name", &assigner.first_name)
            .add_variable("assigner_last_name", &assigner.last_name)
            .add_variable("organization_name", &ctx.organization.name)
            .add_optional_variable("goal", ctx.goal)
            .add_variable("session_url", &session_url)
            .build()
            .await;

        match email_request {
            Ok(request) => {
                if let Err(e) = email_config.client.send_email(request).await {
                    warn!(
                        "Failed to send action assigned email for {}: {e:?}",
                        assignee.email
                    );
                }
            }
            Err(e) => warn!(
                "Failed to build action assigned email for {}: {e:?}",
                assignee.email
            ),
        }
    }

    Ok(())
}

/// Orchestrate sending session-scheduled emails (best-effort).
///
/// Looks up the coaching relationship, both users, and the organization,
/// then sends notification emails to both coach and coachee.
/// Errors are logged internally — email delivery must never block or fail
/// the calling operation.
pub async fn notify_session_scheduled(
    db: &DatabaseConnection,
    config: &Config,
    session: &coaching_sessions::Model,
) {
    let result: Result<(), Error> = async {
        let relationship =
            coaching_relationship::find_by_id(db, session.coaching_relationship_id).await?;
        let coach = user::find_by_id(db, relationship.coach_id).await?;
        let coachee = user::find_by_id(db, relationship.coachee_id).await?;
        let org = organization::find_by_id(db, relationship.organization_id).await?;

        send_session_scheduled_email(config, &coach, &coachee, session, &org).await
    }
    .await;

    if let Err(e) = result {
        warn!(
            "Failed to send session scheduled emails for session {}: {e:?}",
            session.id
        );
    }
}

/// Send a recurring-sessions-scheduled notification email to a single recipient.
/// One email per recipient — coach and coachee each get their own summarizing
/// the freshly scheduled series.
async fn send_recurring_series_email_to_recipient(
    email_config: &ResolvedEmailConfig,
    recipient: &users::Model,
    other_user: &users::Model,
    other_user_role: &str,
    sessions: &[coaching_sessions::Model],
    organization: &organizations::Model,
) -> Result<(), Error> {
    let first = sessions.first().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
            "Cannot send recurring sessions email: sessions slice is empty".to_string(),
        )),
    })?;
    let last = sessions.last().expect("non-empty slice already checked");

    let (first_session_date, first_session_time) =
        format_session_date_time(first.date, &recipient.timezone);
    let (last_session_date, _last_session_time) =
        format_session_date_time(last.date, &recipient.timezone);
    let session_url = email_config.build_session_url(&first.id)?;
    let session_count = sessions.len().to_string();

    let email_request = SendEmailRequestBuilder::new()
        .from(FROM_ADDRESS)
        .to_with_name(
            &recipient.email,
            format!("{} {}", recipient.first_name, recipient.last_name),
        )
        .template_id(&email_config.template_id)
        .add_variable("first_name", &recipient.first_name)
        .add_variable("other_user_first_name", &other_user.first_name)
        .add_variable("other_user_last_name", &other_user.last_name)
        .add_variable("other_user_role", other_user_role)
        .add_variable("organization_name", &organization.name)
        .add_variable("session_count", &session_count)
        .add_variable("first_session_date", &first_session_date)
        .add_variable("first_session_time", &first_session_time)
        .add_variable("last_session_date", &last_session_date)
        .add_variable("session_url", &session_url)
        .build()
        .await?;

    email_config.client.send_email(email_request).await
}

/// Send recurring-sessions-scheduled notification emails to both coach and coachee.
async fn send_recurring_sessions_scheduled_email(
    config: &Config,
    coach: &users::Model,
    coachee: &users::Model,
    sessions: &[coaching_sessions::Model],
    organization: &organizations::Model,
) -> Result<(), Error> {
    info!(
        "Initiating recurring sessions scheduled emails for {} sessions (coach: {}, coachee: {})",
        sessions.len(),
        coach.email,
        coachee.email
    );

    let email_config = ResolvedEmailConfig::new::<RecurringSessionsScheduled>(config).await?;

    if let Err(e) = send_recurring_series_email_to_recipient(
        &email_config,
        coachee,
        coach,
        "coach",
        sessions,
        organization,
    )
    .await
    {
        warn!(
            "Failed to send recurring sessions scheduled email to coachee {}: {e:?}",
            coachee.email
        );
    }

    if let Err(e) = send_recurring_series_email_to_recipient(
        &email_config,
        coach,
        coachee,
        "coachee",
        sessions,
        organization,
    )
    .await
    {
        warn!(
            "Failed to send recurring sessions scheduled email to coach {}: {e:?}",
            coach.email
        );
    }

    Ok(())
}

/// Orchestrate sending recurring-sessions-scheduled emails (best-effort).
///
/// Looks up the coaching relationship, both users, and the organization,
/// then sends a single summary email per recipient covering the whole series — count.
///
/// Errors are logged internally — email delivery must never block or fail
/// the calling operation.
pub async fn notify_recurring_sessions_scheduled(
    db: &DatabaseConnection,
    config: &Config,
    sessions: &[coaching_sessions::Model],
) {
    if sessions.is_empty() {
        return;
    }

    let result: Result<(), Error> = async {
        let relationship_id = sessions[0].coaching_relationship_id;
        let relationship = coaching_relationship::find_by_id(db, relationship_id).await?;
        let coach = user::find_by_id(db, relationship.coach_id).await?;
        let coachee = user::find_by_id(db, relationship.coachee_id).await?;
        let org = organization::find_by_id(db, relationship.organization_id).await?;

        send_recurring_sessions_scheduled_email(config, &coach, &coachee, sessions, &org).await
    }
    .await;

    if let Err(e) = result {
        warn!(
            "Failed to send recurring sessions scheduled emails for {} sessions: {e:?}",
            sessions.len()
        );
    }
}

/// Returns the title of the goal linked to an action, if any.
///
/// `None` when the action has no `goal_id`, when the linked goal has no title,
/// or when the lookup fails (best-effort: email delivery is never blocked).
async fn get_action_goal_title(db: &DatabaseConnection, action: &actions::Model) -> Option<String> {
    let goal_id = action.goal_id?;
    goal::find_by_id(db, goal_id)
        .await
        .ok()
        .and_then(|g| g.title)
}

/// Orchestrate sending action-assigned emails (best-effort).
///
/// Looks up assignee users, the coaching session, relationship, organization,
/// and goals, then sends notification emails to all assignees.
/// Errors are logged internally — email delivery must never block or fail
/// the calling operation.
pub async fn notify_action_assigned(
    db: &DatabaseConnection,
    config: &Config,
    assignee_ids: &[Id],
    assigner: &users::Model,
    action: &actions::Model,
) {
    let result: Result<(), Error> = async {
        // Look up assignee user models
        let assignees = user::find_by_ids(db, assignee_ids).await?;

        // Look up session → relationship → organization
        let (_, relationship) =
            coaching_session::find_by_id_with_coaching_relationship(db, action.coaching_session_id)
                .await?;
        let org = organization::find_by_id(db, relationship.organization_id).await?;

        let goal_title = get_action_goal_title(db, action).await;

        let ctx = ActionEmailContext {
            action_body: action.body.as_deref().unwrap_or(""),
            due_by: action.due_by,
            session_id: action.coaching_session_id,
            organization: &org,
            goal: goal_title.as_deref(),
        };

        send_action_assigned_email(config, &assignees, assigner, &ctx).await
    }
    .await;

    if let Err(e) = result {
        warn!(
            "Failed to send action assigned emails for action {}: {e:?}",
            action.id
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{coaching_sessions, organizations, users, Id};
    use chrono::NaiveDate;
    use mockito::{Server, ServerGuard};
    use service::config::Config;

    async fn setup_test_server() -> ServerGuard {
        Server::new_async().await
    }

    fn create_test_user() -> users::Model {
        users::Model {
            id: Id::new_v4(),
            first_name: "John".to_string(),
            last_name: "Doe".to_string(),
            email: "john.doe@example.com".to_string(),
            display_name: Some("John Doe".to_string()),
            password: Some("hashed_password".to_string()),
            github_username: None,
            github_profile_url: None,
            timezone: "UTC".to_string(),
            role: users::Role::User,
            roles: vec![],
            invite_status: None,
            created_at: chrono::Utc::now().fixed_offset(),
            updated_at: chrono::Utc::now().fixed_offset(),
        }
    }

    fn create_test_user_with(
        first_name: &str,
        last_name: &str,
        email: &str,
        timezone: &str,
    ) -> users::Model {
        users::Model {
            id: Id::new_v4(),
            first_name: first_name.to_string(),
            last_name: last_name.to_string(),
            email: email.to_string(),
            display_name: Some(format!("{first_name} {last_name}")),
            password: Some("hashed_password".to_string()),
            github_username: None,
            github_profile_url: None,
            timezone: timezone.to_string(),
            role: users::Role::User,
            roles: vec![],
            invite_status: None,
            created_at: chrono::Utc::now().fixed_offset(),
            updated_at: chrono::Utc::now().fixed_offset(),
        }
    }

    fn create_test_session() -> coaching_sessions::Model {
        coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            collab_document_name: None,
            date: NaiveDate::from_ymd_opt(2026, 3, 4)
                .unwrap()
                .and_hms_opt(15, 0, 0)
                .unwrap(),
            meeting_url: None,
            provider: None,
            created_at: chrono::Utc::now().fixed_offset(),
            updated_at: chrono::Utc::now().fixed_offset(),
            hydrated_at: Some(chrono::Utc::now().fixed_offset()),
        }
    }

    fn create_test_organization() -> organizations::Model {
        organizations::Model {
            id: Id::new_v4(),
            name: "Acme Corp".to_string(),
            logo: None,
            slug: "acme-corp".to_string(),
            created_at: chrono::Utc::now().fixed_offset(),
            updated_at: chrono::Utc::now().fixed_offset(),
        }
    }

    fn create_config_with_mock(server_url: &str) -> Config {
        Config::from_args([
            "test",
            "--resend-api-key=test_api_key_123",
            "--welcome-email-template-id=template_123",
            "--frontend-base-url=https://app.example.com",
            &format!("--resend-base-url={server_url}"),
        ])
    }

    fn create_full_config_with_mock(server_url: &str) -> Config {
        Config::from_args([
            "test",
            "--resend-api-key=test_api_key_123",
            "--welcome-email-template-id=template_123",
            "--session-scheduled-email-template-id=session_template_456",
            "--recurring-sessions-scheduled-email-template-id=recurring_template_xyz",
            "--action-assigned-email-template-id=action_template_789",
            "--frontend-base-url=https://app.example.com",
            &format!("--resend-base-url={server_url}"),
        ])
    }

    #[tokio::test]
    async fn test_send_welcome_email_success() {
        let mut server = setup_test_server().await;
        let user = create_test_user();
        let inviter = create_test_user_with("Sarah", "Coach", "sarah.coach@example.com", "UTC");
        let config = create_config_with_mock(&server.url());

        let _mock = server
            .mock("POST", "/emails")
            .match_header("authorization", "Bearer test_api_key_123")
            .match_header("content-type", "application/json")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"John Doe\" <john.doe@example.com>"],
                "template": {
                    "id": "template_123",
                    "variables": {
                        "first_name": "John",
                        "last_name": "Doe",
                        "coach_first_name": "Sarah",
                        "coach_full_name": "Sarah Coach",
                        "magic_link_url": "https://app.example.com/setup/test-magic-link-token"
                    }
                }
            })))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"id":"email_msg_123456789"}"#)
            .expect(1)
            .create_async()
            .await;

        let result = send_welcome_email(&config, &user, &inviter, "test-magic-link-token").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_welcome_email_missing_api_key() {
        let config = Config::from_args(["test", "--welcome-email-template-id=template_123"]);
        assert!(config.resend_api_key().is_none(), "API key should be None");

        let user = create_test_user();
        let inviter = create_test_user();

        let result = send_welcome_email(&config, &user, &inviter, "test-magic-link-token").await;
        assert!(result.is_err());

        if let Err(e) = result {
            match e.error_kind {
                DomainErrorKind::Internal(InternalErrorKind::Config) => {}
                _ => panic!("Expected Config error, got: {:?}", e.error_kind),
            }
        }
    }

    #[tokio::test]
    async fn test_send_welcome_email_missing_template_id() {
        let config = Config::from_args(["test", "--resend-api-key=test_api_key_123"]);
        assert!(
            config.resend_api_key().is_some(),
            "API key should be present"
        );
        assert!(
            config.welcome_email_template_id().is_none(),
            "Template ID should be None"
        );

        let user = create_test_user();
        let inviter = create_test_user();

        let result = send_welcome_email(&config, &user, &inviter, "test-magic-link-token").await;
        assert!(result.is_err());

        if let Err(e) = result {
            match e.error_kind {
                DomainErrorKind::Internal(InternalErrorKind::Config) => {}
                _ => panic!("Expected Config error, got: {:?}", e.error_kind),
            }
        }
    }

    #[tokio::test]
    async fn test_send_welcome_email_http_error() {
        let mut server = setup_test_server().await;
        let user = create_test_user();
        let inviter = create_test_user();
        let config = create_config_with_mock(&server.url());

        let _mock = server
            .mock("POST", "/emails")
            .with_status(400)
            .with_body(r#"{"message": "Invalid request"}"#)
            .expect(1)
            .create_async()
            .await;

        // HTTP 400 from Resend should propagate as an error that carries the
        // response body — that body is the caller's only diagnostic.
        let result = send_welcome_email(&config, &user, &inviter, "test-magic-link-token").await;
        let err = result.unwrap_err();
        match err.error_kind {
            DomainErrorKind::Internal(InternalErrorKind::Other(text)) => assert!(
                text.contains("Invalid request"),
                "response body not propagated into error, got: {text}"
            ),
            other => panic!("expected Internal(Other), got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_send_welcome_email_escapes_name_with_specials() {
        // Integration-level counterpart to gateway::resend's
        // `test_format_mailbox_quotes_and_escapes_specials`: a user whose
        // assembled name contains a comma must land in the `to` field as a
        // quoted-string, not as two malformed mailboxes.
        let mut server = setup_test_server().await;
        let user = create_test_user_with("Jane", "Doe, Jr.", "jane.jr@example.com", "UTC");
        let inviter = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let config = create_config_with_mock(&server.url());

        let _mock = server
            .mock("POST", "/emails")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Jane Doe, Jr.\" <jane.jr@example.com>"],
                "template": {
                    "id": "template_123",
                    "variables": {
                        "first_name": "Jane",
                        "last_name": "Doe, Jr.",
                        "coach_first_name": "Alex",
                        "coach_full_name": "Alex Smith",
                        "magic_link_url": "https://app.example.com/setup/test-magic-link-token"
                    }
                }
            })))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let result = send_welcome_email(&config, &user, &inviter, "test-magic-link-token").await;
        assert!(result.is_ok());
    }

    // ── Session Scheduled Email Tests ──────────────────────────────────

    #[tokio::test]
    async fn test_send_session_scheduled_email_variables() {
        let mut server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());

        // Coach and coachee in different timezones so a single body-match per
        // recipient proves BOTH the role swap (coach <-> coachee) AND that each
        // recipient's own timezone is used. Session is 2026-03-04 15:00 UTC:
        //   - coachee (America/New_York, EST): 10:00 AM, Wed March 4
        //   - coach   (Asia/Tokyo):            12:00 AM, Thu March 5 (date rolls)
        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "Asia/Tokyo");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "America/New_York");
        let session = create_test_session();
        let org = create_test_organization();

        let session_url = format!("https://app.example.com/coaching-sessions/{}", session.id);

        // Email to coachee — other_user is the coach, formatted in NY time.
        let _mock_coachee = server
            .mock("POST", "/emails")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Jane Doe\" <jane@example.com>"],
                "template": {
                    "id": "session_template_456",
                    "variables": {
                        "first_name": "Jane",
                        "other_user_first_name": "Alex",
                        "other_user_last_name": "Smith",
                        "other_user_role": "coach",
                        "organization_name": "Acme Corp",
                        "session_date": "Wednesday, March 4, 2026",
                        "session_time": "10:00 AM",
                        "session_url": session_url.clone(),
                    }
                }
            })))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        // Email to coach — other_user is the coachee, formatted in Tokyo time
        // (the session date rolls forward a day).
        let _mock_coach = server
            .mock("POST", "/emails")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Alex Smith\" <alex@example.com>"],
                "template": {
                    "id": "session_template_456",
                    "variables": {
                        "first_name": "Alex",
                        "other_user_first_name": "Jane",
                        "other_user_last_name": "Doe",
                        "other_user_role": "coachee",
                        "organization_name": "Acme Corp",
                        "session_date": "Thursday, March 5, 2026",
                        "session_time": "12:00 AM",
                        "session_url": session_url.clone(),
                    }
                }
            })))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let result = send_session_scheduled_email(&config, &coach, &coachee, &session, &org).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_session_scheduled_email_missing_template_id() {
        // Config has an API key and frontend base URL but no session-scheduled
        // template id — mirrors the welcome/action missing-template-id tests.
        let server = setup_test_server().await;
        let config = Config::from_args([
            "test",
            "--resend-api-key=test_api_key_123",
            &format!("--resend-base-url={}", server.url()),
            "--frontend-base-url=https://app.example.com",
        ]);

        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let session = create_test_session();
        let org = create_test_organization();

        let result = send_session_scheduled_email(&config, &coach, &coachee, &session, &org).await;

        assert!(result.is_err());
        if let Err(e) = result {
            match e.error_kind {
                DomainErrorKind::Internal(InternalErrorKind::Config) => {}
                _ => panic!("Expected Config error, got: {:?}", e.error_kind),
            }
        }
    }

    // ── Action Assigned Email Tests ────────────────────────────────────

    #[tokio::test]
    async fn test_send_action_assigned_email_success() {
        let mut server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());

        let assigner = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let assignee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let session_id = Id::new_v4();
        let org = create_test_organization();

        let session_url =
            format!("https://app.example.com/coaching-sessions/{session_id}?tab=actions");
        let due_by: DateTime<FixedOffset> = NaiveDate::from_ymd_opt(2026, 3, 7)
            .unwrap()
            .and_hms_opt(17, 0, 0)
            .unwrap()
            .and_utc()
            .fixed_offset();

        let _mock = server
            .mock("POST", "/emails")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Jane Doe\" <jane@example.com>"],
                "template": {
                    "id": "action_template_789",
                    "variables": {
                        "first_name": "Jane",
                        "action_body": "Read chapters 3-5 of Radical Candor",
                        "due_date": "Saturday, March 7, 2026",
                        "assigner_first_name": "Alex",
                        "assigner_last_name": "Smith",
                        "organization_name": "Acme Corp",
                        "goal": "Improve communication",
                        "session_url": session_url,
                    }
                }
            })))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let ctx = ActionEmailContext {
            action_body: "Read chapters 3-5 of Radical Candor",
            due_by: Some(due_by),
            session_id,
            organization: &org,
            goal: Some("Improve communication"),
        };

        let result = send_action_assigned_email(&config, &[assignee], &assigner, &ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_action_assigned_email_no_due_date() {
        let mut server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());

        let assigner = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let assignee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let session_id = Id::new_v4();
        let org = create_test_organization();

        let session_url =
            format!("https://app.example.com/coaching-sessions/{session_id}?tab=actions");

        let _mock = server
            .mock("POST", "/emails")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Jane Doe\" <jane@example.com>"],
                "template": {
                    "id": "action_template_789",
                    "variables": {
                        "first_name": "Jane",
                        "action_body": "Follow up with team",
                        "due_date": "No due date set",
                        "assigner_first_name": "Alex",
                        "assigner_last_name": "Smith",
                        "organization_name": "Acme Corp",
                        "session_url": session_url,
                    }
                }
            })))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let ctx = ActionEmailContext {
            action_body: "Follow up with team",
            due_by: None,
            session_id,
            organization: &org,
            goal: None,
        };

        let result = send_action_assigned_email(&config, &[assignee], &assigner, &ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_action_assigned_email_multiple_assignees() {
        let mut server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());

        let assigner = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let assignee1 = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let assignee2 = create_test_user_with("Bob", "Jones", "bob@example.com", "UTC");
        let session_id = Id::new_v4();
        let org = create_test_organization();

        let session_url =
            format!("https://app.example.com/coaching-sessions/{session_id}?tab=actions");

        // Each assignee must get their OWN email with their OWN first_name and
        // recipient address. Body-match per recipient so a regression that sends
        // both emails to the same person (or with swapped variables) fails here.
        let _mock_jane = server
            .mock("POST", "/emails")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Jane Doe\" <jane@example.com>"],
                "template": {
                    "id": "action_template_789",
                    "variables": {
                        "first_name": "Jane",
                        "action_body": "Complete the survey",
                        "due_date": "No due date set",
                        "assigner_first_name": "Alex",
                        "assigner_last_name": "Smith",
                        "organization_name": "Acme Corp",
                        "session_url": session_url.clone(),
                    }
                }
            })))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let _mock_bob = server
            .mock("POST", "/emails")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Bob Jones\" <bob@example.com>"],
                "template": {
                    "id": "action_template_789",
                    "variables": {
                        "first_name": "Bob",
                        "action_body": "Complete the survey",
                        "due_date": "No due date set",
                        "assigner_first_name": "Alex",
                        "assigner_last_name": "Smith",
                        "organization_name": "Acme Corp",
                        "session_url": session_url.clone(),
                    }
                }
            })))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let ctx = ActionEmailContext {
            action_body: "Complete the survey",
            due_by: None,
            session_id,
            organization: &org,
            goal: None,
        };

        let result =
            send_action_assigned_email(&config, &[assignee1, assignee2], &assigner, &ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_action_assigned_email_missing_template_id() {
        let server = setup_test_server().await;
        let config = Config::from_args([
            "test",
            "--resend-api-key=test_api_key_123",
            &format!("--resend-base-url={}", server.url()),
            "--frontend-base-url=https://app.example.com",
        ]);

        let assigner = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let assignee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let session_id = Id::new_v4();
        let org = create_test_organization();

        let ctx = ActionEmailContext {
            action_body: "Some action",
            due_by: None,
            session_id,
            organization: &org,
            goal: None,
        };

        let result = send_action_assigned_email(&config, &[assignee], &assigner, &ctx).await;

        assert!(result.is_err());
        if let Err(e) = result {
            match e.error_kind {
                DomainErrorKind::Internal(InternalErrorKind::Config) => {}
                _ => panic!("Expected Config error, got: {:?}", e.error_kind),
            }
        }
    }

    #[tokio::test]
    async fn test_send_action_assigned_email_empty_assignees_sends_nothing() {
        let mut server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());

        let assigner = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let session_id = Id::new_v4();
        let org = create_test_organization();

        // Expect exactly zero calls — no assignees means no emails
        let _mock = server
            .mock("POST", "/emails")
            .expect(0)
            .create_async()
            .await;

        let ctx = ActionEmailContext {
            action_body: "Some action",
            due_by: None,
            session_id,
            organization: &org,
            goal: None,
        };

        let result = send_action_assigned_email(&config, &[], &assigner, &ctx).await;
        assert!(result.is_ok());
    }

    // ── build_session_url Unit Tests ────────────────────────────────────

    /// Helper to construct a `ResolvedEmailConfig` with an optional
    /// `SessionUrlBuilder`, without needing a real Resend client.
    async fn create_test_email_config(
        server_url: &str,
        url_builder: Option<SessionUrlBuilder>,
    ) -> ResolvedEmailConfig {
        let config = create_config_with_mock(server_url);
        ResolvedEmailConfig {
            client: ResendClient::new(&config).await.unwrap(),
            template_id: "test_template".to_string(),
            session_url_builder: url_builder,
        }
    }

    #[tokio::test]
    async fn test_build_session_url_success() {
        let server = setup_test_server().await;
        let email_config = create_test_email_config(
            &server.url(),
            Some(SessionUrlBuilder {
                base_url: "https://app.example.com".to_string(),
                path_template: "/coaching-sessions/{session_id}".to_string(),
            }),
        )
        .await;

        let session_id = Id::new_v4();
        let result = email_config.build_session_url(&session_id);

        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            format!("https://app.example.com/coaching-sessions/{session_id}")
        );
    }

    #[tokio::test]
    async fn test_build_session_url_custom_path_template() {
        let server = setup_test_server().await;
        let email_config = create_test_email_config(
            &server.url(),
            Some(SessionUrlBuilder {
                base_url: "https://app.example.com".to_string(),
                path_template: "/sessions/{session_id}?tab=actions".to_string(),
            }),
        )
        .await;

        let session_id = Id::new_v4();
        let result = email_config.build_session_url(&session_id).unwrap();

        assert_eq!(
            result,
            format!("https://app.example.com/sessions/{session_id}?tab=actions")
        );
    }

    #[tokio::test]
    async fn test_build_session_url_no_url_builder() {
        let server = setup_test_server().await;
        let email_config = create_test_email_config(&server.url(), None).await;

        let result = email_config.build_session_url(&Id::new_v4());

        assert!(result.is_err());
        if let Err(e) = result {
            match e.error_kind {
                DomainErrorKind::Internal(InternalErrorKind::Config) => {}
                _ => panic!("Expected Config error, got: {:?}", e.error_kind),
            }
        }
    }

    // ── Recurring Sessions Scheduled Email Tests ───────────────────────

    fn create_test_session_on(date: NaiveDate) -> coaching_sessions::Model {
        coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            collab_document_name: None,
            date: date.and_hms_opt(15, 0, 0).unwrap(),
            meeting_url: None,
            provider: None,
            created_at: chrono::Utc::now().fixed_offset(),
            updated_at: chrono::Utc::now().fixed_offset(),
            hydrated_at: None,
        }
    }

    #[tokio::test]
    async fn test_send_recurring_sessions_scheduled_email_personalization() {
        let mut server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());

        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let org = create_test_organization();

        let sessions = vec![
            create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 4).unwrap()),
            create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 11).unwrap()),
            create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 18).unwrap()),
        ];

        let first_session_url = format!(
            "https://app.example.com/coaching-sessions/{}",
            sessions[0].id
        );

        // Email to coachee — other_user is the coach. Body-match per recipient
        // proves the role swap.
        let _mock_coachee = server
            .mock("POST", "/emails")
            .match_body(mockito::Matcher::Json(serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"Jane Doe\" <jane@example.com>"],
                "template": {
                    "id": "recurring_template_xyz",
                    "variables": {
                        "first_name": "Jane",
                        "other_user_first_name": "Alex",
                        "other_user_last_name": "Smith",
                        "other_user_role": "coach",
                        "organization_name": "Acme Corp",
                        "session_count": "3",
                        "first_session_date": "Wednesday, March 4, 2026",
                        "first_session_time": "3:00 PM",
                        "last_session_date": "Wednesday, March 18, 2026",
                        "session_url": first_session_url,
                    }
                }
            })))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        // Email to coach — only verify it was sent.
        let _mock_coach = server
            .mock("POST", "/emails")
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let result =
            send_recurring_sessions_scheduled_email(&config, &coach, &coachee, &sessions, &org)
                .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_recurring_sessions_scheduled_email_single_session() {
        let mut server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());

        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let org = create_test_organization();

        let sessions = vec![create_test_session_on(
            NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
        )];

        // With a single session, first and last dates must match.
        let _mock_coachee = server
            .mock("POST", "/emails")
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "template": {
                    "variables": {
                        "session_count": "1",
                        "first_session_date": "Wednesday, March 4, 2026",
                        "last_session_date": "Wednesday, March 4, 2026",
                    }
                }
            })))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let _mock_coach = server
            .mock("POST", "/emails")
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let result =
            send_recurring_sessions_scheduled_email(&config, &coach, &coachee, &sessions, &org)
                .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_recurring_sessions_scheduled_email_missing_template_id() {
        let server = setup_test_server().await;
        let config = Config::from_args([
            "test",
            "--resend-api-key=test_api_key_123",
            &format!("--resend-base-url={}", server.url()),
            "--frontend-base-url=https://app.example.com",
        ]);

        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let org = create_test_organization();
        let sessions = vec![create_test_session_on(
            NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
        )];

        let result =
            send_recurring_sessions_scheduled_email(&config, &coach, &coachee, &sessions, &org)
                .await;

        assert!(result.is_err());
        if let Err(e) = result {
            match e.error_kind {
                DomainErrorKind::Internal(InternalErrorKind::Config) => {}
                _ => panic!("Expected Config error, got: {:?}", e.error_kind),
            }
        }
    }

    #[tokio::test]
    async fn test_send_recurring_series_email_to_recipient_empty_sessions_errors() {
        let server = setup_test_server().await;
        let email_config = create_test_email_config(
            &server.url(),
            Some(SessionUrlBuilder {
                base_url: "https://app.example.com".to_string(),
                path_template: "/coaching-sessions/{session_id}".to_string(),
            }),
        )
        .await;

        let recipient = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let other = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let org = create_test_organization();

        let result = send_recurring_series_email_to_recipient(
            &email_config,
            &recipient,
            &other,
            "coach",
            &[],
            &org,
        )
        .await;

        assert!(result.is_err());
        if let Err(e) = result {
            match e.error_kind {
                DomainErrorKind::Internal(InternalErrorKind::Other(msg)) => {
                    assert!(msg.contains("sessions slice is empty"));
                }
                _ => panic!("Expected Internal(Other) error, got: {:?}", e.error_kind),
            }
        }
    }

    // ── format_session_date_time Unit Tests ────────────────────────────

    #[test]
    fn test_format_session_date_time_utc() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 4)
            .unwrap()
            .and_hms_opt(15, 0, 0)
            .unwrap();
        let (date_str, time_str) = format_session_date_time(date, "UTC");
        assert_eq!(date_str, "Wednesday, March 4, 2026");
        assert_eq!(time_str, "3:00 PM");
    }

    #[test]
    fn test_format_session_date_time_eastern() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 4)
            .unwrap()
            .and_hms_opt(15, 0, 0)
            .unwrap();
        let (date_str, time_str) = format_session_date_time(date, "America/New_York");
        assert_eq!(date_str, "Wednesday, March 4, 2026");
        assert_eq!(time_str, "10:00 AM");
    }

    #[test]
    fn test_format_session_date_time_invalid_timezone_falls_back_to_utc() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 4)
            .unwrap()
            .and_hms_opt(15, 0, 0)
            .unwrap();
        let (date_str, time_str) = format_session_date_time(date, "Invalid/Timezone");
        assert_eq!(date_str, "Wednesday, March 4, 2026");
        assert_eq!(time_str, "3:00 PM UTC");
    }

    #[test]
    fn test_format_session_date_time_date_rolls_over_with_timezone() {
        // 2026-03-07 23:00 UTC → 2026-03-08 08:00 in Asia/Tokyo (UTC+9)
        let date = NaiveDate::from_ymd_opt(2026, 3, 7)
            .unwrap()
            .and_hms_opt(23, 0, 0)
            .unwrap();
        let (date_str, time_str) = format_session_date_time(date, "Asia/Tokyo");
        assert_eq!(date_str, "Sunday, March 8, 2026");
        assert_eq!(time_str, "8:00 AM");
    }
}
