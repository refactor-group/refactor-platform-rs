use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use log::*;
use sea_orm::DatabaseConnection;
use service::config::Config;

use crate::{
    actions, coaching_relationship, coaching_session, coaching_session_series,
    coaching_session_series::SeriesRule,
    coaching_sessions,
    error::Error,
    error::{DomainErrorKind, InternalErrorKind},
    gateway::ical::{self, DescriptionParts, OpenAction},
    gateway::resend::{Client as ResendClient, SendEmailRequestBuilder},
    goal, organization, organizations, user, users,
    users::Role,
    Id,
};

#[cfg(test)]
#[path = "emails_added_to_organization_tests.rs"]
mod added_to_organization_tests;

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

struct SessionRescheduled;
impl EmailNotification for SessionRescheduled {
    fn template_id(config: &Config) -> Option<String> {
        config.session_rescheduled_email_template_id()
    }
    fn notification_name() -> &'static str {
        "session rescheduled"
    }
    fn url_path_template(config: &Config) -> Option<String> {
        Some(config.session_scheduled_email_url_path().to_owned())
    }
}

struct SeriesRescheduled;
impl EmailNotification for SeriesRescheduled {
    fn template_id(config: &Config) -> Option<String> {
        config.series_rescheduled_email_template_id()
    }
    fn notification_name() -> &'static str {
        "series rescheduled"
    }
    fn url_path_template(config: &Config) -> Option<String> {
        Some(config.session_scheduled_email_url_path().to_owned())
    }
}

/// Deliberately keeps the trait's `None` URL template: the session row is gone by the
/// time a recipient could click through.
struct SessionCancelled;
impl EmailNotification for SessionCancelled {
    fn template_id(config: &Config) -> Option<String> {
        config.session_cancelled_email_template_id()
    }
    fn notification_name() -> &'static str {
        "session cancelled"
    }
}

/// Deliberately keeps the trait's `None` URL template: the series row is gone by the
/// time a recipient could click through.
struct SeriesCancelled;
impl EmailNotification for SeriesCancelled {
    fn template_id(config: &Config) -> Option<String> {
        config.series_cancelled_email_template_id()
    }
    fn notification_name() -> &'static str {
        "series cancelled"
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

struct AddedToOrganization;
impl EmailNotification for AddedToOrganization {
    fn template_id(config: &Config) -> Option<String> {
        config.added_to_organization_email_template_id()
    }
    fn notification_name() -> &'static str {
        "added to organization"
    }
    fn url_path_template(config: &Config) -> Option<String> {
        Some(config.added_to_organization_email_url_path().to_owned())
    }
}

struct PasswordResetEmail;
impl EmailNotification for PasswordResetEmail {
    fn template_id(config: &Config) -> Option<String> {
        config.password_reset_email_template_id()
    }
    fn notification_name() -> &'static str {
        "password reset"
    }
    fn url_path_template(config: &Config) -> Option<String> {
        Some(config.password_reset_email_url_path().to_owned())
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
    let raw_token = crate::magic_link_token::create_setup_link(db, user.id, config).await?;
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
    match crate::magic_link_token::create_setup_link(db, user.id, config).await {
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
        .add_variable("first_name", user.first_name.as_str())
        .add_variable("last_name", user.last_name.as_str())
        .add_variable("coach_first_name", inviter.first_name.as_str())
        .add_variable("coach_full_name", coach_full_name.as_str())
        .add_variable("magic_link_url", magic_link_url.as_str())
        .build()
        .await?;
    debug!("Email request created for {}", user.email);

    email_config.client.send_email(email_request).await
}

/// Send a best-effort notification that a user was granted a role in an organization.
///
/// `inviter` is the administrator who added them. Errors are logged internally and
/// never propagate: a failed notification must not undo a membership that already
/// committed.
pub async fn notify_added_to_organization(
    config: &Config,
    user: &users::Model,
    inviter: &users::Model,
    organization: &organizations::Model,
    role: Role,
) {
    if let Err(e) =
        send_added_to_organization_email(config, user, inviter, organization, role).await
    {
        warn!(
            "Failed to send added to organization email to {}: {e:?}",
            user.email
        );
    }
}

/// Recipient-facing name for a role, as the product labels it in the UI.
pub(crate) fn role_display_name(role: Role) -> &'static str {
    match role {
        Role::Admin => "Admin",
        Role::User | Role::SuperAdmin => "Member",
    }
}

/// Build and send the added-to-organization email to the newly attached user.
async fn send_added_to_organization_email(
    config: &Config,
    user: &users::Model,
    inviter: &users::Model,
    organization: &organizations::Model,
    role: Role,
) -> Result<(), Error> {
    info!(
        "Initiating added to organization email for user: {} ({})",
        user.email, user.id
    );

    let email_config = ResolvedEmailConfig::new::<AddedToOrganization>(config).await?;

    let organization_url = email_config
        .session_url_builder
        .as_ref()
        .map(|b| b.build(ORGANIZATION_ID_PLACEHOLDER, &organization.id.to_string()))
        .unwrap_or_default();

    let inviter_full_name = format!("{} {}", inviter.first_name, inviter.last_name);

    let email_request = SendEmailRequestBuilder::new()
        .from(FROM_ADDRESS)
        .to_with_name(
            &user.email,
            format!("{} {}", user.first_name, user.last_name),
        )
        .template_id(&email_config.template_id)
        .add_variable("first_name", user.first_name.as_str())
        .add_variable("last_name", user.last_name.as_str())
        .add_variable("organization_name", organization.name.as_str())
        .add_variable("role_name", role_display_name(role))
        .add_variable("inviter_first_name", inviter.first_name.as_str())
        .add_variable("inviter_full_name", inviter_full_name.as_str())
        .add_variable("organization_url", organization_url.as_str())
        .build()
        .await?;

    email_config.client.send_email(email_request).await
}

/// Build and send a password-reset email to a single user.
///
/// Called from the password-reset domain flow after a token has been issued.
/// Note: logs `user.id` rather than `user.email` — raw email addresses are
/// PII and are not emitted at INFO level on this code path.
pub(crate) async fn send_password_reset_email(
    config: &Config,
    user: &users::Model,
    raw_token: &str,
) -> Result<(), Error> {
    info!("Initiating password-reset email for user {}", user.id);

    let email_config = ResolvedEmailConfig::new::<PasswordResetEmail>(config).await?;

    let password_reset_url = email_config
        .session_url_builder
        .as_ref()
        .map(|b| b.build(TOKEN_PLACEHOLDER, raw_token))
        .unwrap_or_default();

    let email_request = SendEmailRequestBuilder::new()
        .from(FROM_ADDRESS)
        .to_with_name(
            &user.email,
            format!("{} {}", user.first_name, user.last_name),
        )
        .template_id(&email_config.template_id)
        .add_variable("first_name", user.first_name.as_str())
        .add_variable("last_name", user.last_name.as_str())
        .add_variable("password_reset_url", password_reset_url.as_str())
        .build()
        .await?;

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
const ORGANIZATION_ID_PLACEHOLDER: &str = "{organization_id}";

/// `.ics` DESCRIPTION for a cancellation. A cancellation only needs to identify the
/// event, so no topics, goals, or actions are loaded.
const SESSION_CANCELLED_DESCRIPTION: &str = "This coaching session has been cancelled.";
const SERIES_CANCELLED_DESCRIPTION: &str =
    "This recurring coaching session series has been cancelled.";

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
#[allow(clippy::too_many_arguments)]
async fn send_session_email_to_recipient(
    email_config: &ResolvedEmailConfig,
    recipient: &users::Model,
    other_user: &users::Model,
    other_user_role: &str,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
    ics_body: &str,
    session_or_series: Option<&str>,
) -> Result<(), Error> {
    let (session_date, session_time) = format_session_date_time(session.date, &recipient.timezone);
    let session_url = email_config.build_session_url(&session.id)?;
    let session_duration =
        crate::duration::Duration::from_minutes_unchecked(session.duration_minutes).to_string();

    let email_request = SendEmailRequestBuilder::new()
        .from(FROM_ADDRESS)
        .to_with_name(
            &recipient.email,
            format!("{} {}", recipient.first_name, recipient.last_name),
        )
        .template_id(&email_config.template_id)
        .add_variable("first_name", recipient.first_name.as_str())
        .add_variable("other_user_first_name", other_user.first_name.as_str())
        .add_variable("other_user_last_name", other_user.last_name.as_str())
        .add_variable("other_user_role", other_user_role)
        .add_variable("organization_name", organization.name.as_str())
        .add_variable("session_date", session_date.as_str())
        .add_variable("session_time", session_time.as_str())
        .add_variable("session_duration", session_duration.as_str())
        .add_variable("session_url", session_url.as_str())
        .add_optional_variable("session_or_series", session_or_series)
        .add_ics_attachment(ics_body, &ical::Method::Request)
        .build()
        .await?;

    email_config.client.send_email(email_request).await
}

/// Build the single-session invite `.ics` body. Pure: `dtstamp` is injected so the
/// output is deterministic for a given input.
fn build_session_invite_ics(
    organizer: &users::Model,
    attendee: &users::Model,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
    description: String,
    dtstamp: chrono::NaiveDateTime,
) -> Result<String, Error> {
    let anchor_tz = organizer
        .timezone
        .parse::<chrono_tz::Tz>()
        .unwrap_or(chrono_tz::UTC);
    let invite = ical::IcsInvite {
        uid: format!("{}@myrefactor.com", session.id),
        sequence: session.ical_sequence,
        method: ical::Method::Request,
        status: ical::EventStatus::Confirmed,
        summary: format!("Coaching Session: {}", organization.name),
        description,
        anchor_tz,
        dtstamp,
        start: session.date,
        duration_minutes: session.duration_minutes,
        organizer,
        attendee,
        location_url: session.meeting_url.clone(),
        recurrence: None,
    };
    ical::build(&invite)
}

/// Send single-session invite emails to both coach and coachee. Generic over the
/// notification type `N` so the scheduled and reschedule flows share one body; they
/// differ only by template (via `N`) and the `session_or_series` template variable.
/// A reschedule passes an already-bumped `session`, so the `.ics` carries the next
/// `SEQUENCE` under a stable `UID`.
async fn send_single_session_invite_email<N: EmailNotification>(
    db: &DatabaseConnection,
    config: &Config,
    coach: &users::Model,
    coachee: &users::Model,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
    session_or_series: Option<&str>,
) -> Result<(), Error> {
    info!(
        "Initiating {} emails for session: {} (coach: {}, coachee: {})",
        N::notification_name(),
        session.id,
        coach.email,
        coachee.email
    );

    let email_config = ResolvedEmailConfig::new::<N>(config).await?;

    // Same VEVENT for both recipients (organizer = coach, attendee = coachee).
    let topics = entity_api::coaching_session_topic::find_by_coaching_session_id(db, session.id)
        .await?
        .into_iter()
        .map(|t| t.body)
        .collect::<Vec<String>>();
    let goal_titles =
        entity_api::coaching_session_goal::find_in_progress_goals_by_coaching_session_id(
            db, session.id,
        )
        .await?
        .into_iter()
        .filter_map(|g| g.title)
        .collect::<Vec<String>>();
    let open_actions = entity_api::action::find_open_by_coaching_session_id(db, session.id)
        .await?
        .into_iter()
        .filter_map(|a| {
            a.body.map(|body| OpenAction {
                body,
                due_by: a.due_by.map(|d| d.naive_utc()),
            })
        })
        .collect::<Vec<OpenAction>>();

    let anchor_tz = coach
        .timezone
        .parse::<chrono_tz::Tz>()
        .unwrap_or(chrono_tz::UTC);
    let description = ical::compose_description(&DescriptionParts {
        session_url: email_config.build_session_url(&session.id)?,
        title: session.title.as_deref(),
        topics: &topics,
        goal_titles: &goal_titles,
        open_actions: &open_actions,
        anchor_tz,
    });

    let ics_body = build_session_invite_ics(
        coach,
        coachee,
        session,
        organization,
        description,
        chrono::Utc::now().naive_utc(),
    )?;

    // Email to coachee: "Your coach, ... has a session with you"
    if let Err(e) = send_session_email_to_recipient(
        &email_config,
        coachee,
        coach,
        "coach",
        session,
        organization,
        &ics_body,
        session_or_series,
    )
    .await
    {
        warn!(
            "Failed to send {} email to coachee {}: {e:?}",
            N::notification_name(),
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
        &ics_body,
        session_or_series,
    )
    .await
    {
        warn!(
            "Failed to send {} email to coach {}: {e:?}",
            N::notification_name(),
            coach.email
        );
    }

    Ok(())
}

/// Send session-scheduled notification emails to both coach and coachee.
async fn send_session_scheduled_email(
    db: &DatabaseConnection,
    config: &Config,
    coach: &users::Model,
    coachee: &users::Model,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
) -> Result<(), Error> {
    send_single_session_invite_email::<SessionScheduled>(
        db,
        config,
        coach,
        coachee,
        session,
        organization,
        None,
    )
    .await
}

/// Send session-rescheduled notification emails to both coach and coachee. `session`
/// must already carry the bumped `ical_sequence`, so the invite updates the calendar
/// event in place (same `UID`, next `SEQUENCE`).
async fn send_session_rescheduled_email(
    db: &DatabaseConnection,
    config: &Config,
    coach: &users::Model,
    coachee: &users::Model,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
) -> Result<(), Error> {
    send_single_session_invite_email::<SessionRescheduled>(
        db,
        config,
        coach,
        coachee,
        session,
        organization,
        Some("session"),
    )
    .await
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
            .add_variable("first_name", assignee.first_name.as_str())
            .add_variable("action_body", ctx.action_body)
            .add_variable("due_date", due_date_str.as_str())
            .add_variable("assigner_first_name", assigner.first_name.as_str())
            .add_variable("assigner_last_name", assigner.last_name.as_str())
            .add_variable("organization_name", ctx.organization.name.as_str())
            .add_optional_variable("goal", ctx.goal)
            .add_variable("session_url", session_url.as_str())
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

        send_session_scheduled_email(db, config, &coach, &coachee, session, &org).await
    }
    .await;

    if let Err(e) = result {
        warn!(
            "Failed to send session scheduled emails for session {}: {e:?}",
            session.id
        );
    }
}

/// Orchestrate sending session-rescheduled emails (best-effort).
///
/// `session` must already carry the bumped `ical_sequence`. Looks up the coaching
/// relationship, both users, and the organization, then re-sends the invite to both
/// coach and coachee so their calendar event updates in place. Errors are logged
/// internally and never block or fail the calling operation.
pub async fn notify_session_rescheduled(
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

        send_session_rescheduled_email(db, config, &coach, &coachee, session, &org).await
    }
    .await;

    if let Err(e) = result {
        warn!(
            "Failed to send session rescheduled emails for session {}: {e:?}",
            session.id
        );
    }
}

/// Build the single-session cancellation `.ics` body. Pure: `dtstamp` is injected so the
/// output is deterministic for a given input. Keeps the invite's `UID` and `DTSTART` so
/// calendar clients match the cancellation to the event they already hold.
fn build_session_cancel_ics(
    organizer: &users::Model,
    attendee: &users::Model,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
    description: String,
    dtstamp: chrono::NaiveDateTime,
) -> Result<String, Error> {
    let anchor_tz = organizer
        .timezone
        .parse::<chrono_tz::Tz>()
        .unwrap_or(chrono_tz::UTC);
    let invite = ical::IcsInvite {
        uid: format!("{}@myrefactor.com", session.id),
        // In-memory bump only: the row is being deleted, so persisting it is a wasted write.
        sequence: session.ical_sequence + 1,
        method: ical::Method::Cancel,
        status: ical::EventStatus::Cancelled,
        summary: format!("Coaching Session: {}", organization.name),
        description,
        anchor_tz,
        dtstamp,
        start: session.date,
        duration_minutes: session.duration_minutes,
        organizer,
        attendee,
        location_url: session.meeting_url.clone(),
        recurrence: None,
    };
    ical::build(&invite)
}

/// Send a session-cancelled notification email to a single recipient. Carries fewer
/// variables than the invite sends: no `session_url` (the row is gone) and no duration.
async fn send_session_cancelled_email_to_recipient(
    email_config: &ResolvedEmailConfig,
    recipient: &users::Model,
    other_user: &users::Model,
    other_user_role: &str,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
    ics_body: &str,
) -> Result<(), Error> {
    let (session_date, session_time) = format_session_date_time(session.date, &recipient.timezone);

    let email_request = SendEmailRequestBuilder::new()
        .from(FROM_ADDRESS)
        .to_with_name(
            &recipient.email,
            format!("{} {}", recipient.first_name, recipient.last_name),
        )
        .template_id(&email_config.template_id)
        .add_variable("first_name", recipient.first_name.as_str())
        .add_variable("other_user_first_name", other_user.first_name.as_str())
        .add_variable("other_user_last_name", other_user.last_name.as_str())
        .add_variable("other_user_role", other_user_role)
        .add_variable("organization_name", organization.name.as_str())
        .add_variable("session_date", session_date.as_str())
        .add_variable("session_time", session_time.as_str())
        .add_ics_attachment(ics_body, &ical::Method::Cancel)
        .build()
        .await?;

    email_config.client.send_email(email_request).await
}

/// Send session-cancelled emails to both coach and coachee. Takes no database handle:
/// a cancellation loads no topics, goals, or actions.
async fn send_session_cancelled_email(
    config: &Config,
    coach: &users::Model,
    coachee: &users::Model,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
) -> Result<(), Error> {
    info!(
        "Initiating session cancelled emails for session: {} (coach: {}, coachee: {})",
        session.id, coach.email, coachee.email
    );

    let email_config = ResolvedEmailConfig::new::<SessionCancelled>(config).await?;

    let ics_body = build_session_cancel_ics(
        coach,
        coachee,
        session,
        organization,
        SESSION_CANCELLED_DESCRIPTION.to_string(),
        chrono::Utc::now().naive_utc(),
    )?;

    if let Err(e) = send_session_cancelled_email_to_recipient(
        &email_config,
        coachee,
        coach,
        "coach",
        session,
        organization,
        &ics_body,
    )
    .await
    {
        warn!(
            "Failed to send session cancelled email to coachee {}: {e:?}",
            coachee.email
        );
    }

    if let Err(e) = send_session_cancelled_email_to_recipient(
        &email_config,
        coach,
        coachee,
        "coachee",
        session,
        organization,
        &ics_body,
    )
    .await
    {
        warn!(
            "Failed to send session cancelled email to coach {}: {e:?}",
            coach.email
        );
    }

    Ok(())
}

/// Orchestrate sending session-cancelled emails (best-effort).
///
/// Call this with the in-memory model after the delete has committed. Deleting a session
/// whose date has already passed is housekeeping rather than a cancellation, so that case
/// returns before any lookup. Errors are logged internally and never block the caller.
pub async fn notify_session_cancelled(
    db: &DatabaseConnection,
    config: &Config,
    session: &coaching_sessions::Model,
) {
    if session.date < chrono::Utc::now().naive_utc() {
        return;
    }

    let result: Result<(), Error> = async {
        let relationship =
            coaching_relationship::find_by_id(db, session.coaching_relationship_id).await?;
        let coach = user::find_by_id(db, relationship.coach_id).await?;
        let coachee = user::find_by_id(db, relationship.coachee_id).await?;
        let org = organization::find_by_id(db, relationship.organization_id).await?;

        send_session_cancelled_email(config, &coach, &coachee, session, &org).await
    }
    .await;

    if let Err(e) = result {
        warn!(
            "Failed to send session cancelled emails for session {}: {e:?}",
            session.id
        );
    }
}

/// Send a recurring-sessions-scheduled notification email to a single recipient.
/// One email per recipient — coach and coachee each get their own summarizing
/// the freshly scheduled series.
#[allow(clippy::too_many_arguments)]
async fn send_recurring_series_email_to_recipient(
    email_config: &ResolvedEmailConfig,
    recipient: &users::Model,
    other_user: &users::Model,
    other_user_role: &str,
    sessions: &[coaching_sessions::Model],
    organization: &organizations::Model,
    ics_body: &str,
    session_or_series: Option<&str>,
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
    // All sessions in a recurring series share the same duration.
    let session_duration =
        crate::duration::Duration::from_minutes_unchecked(first.duration_minutes).to_string();

    let email_request = SendEmailRequestBuilder::new()
        .from(FROM_ADDRESS)
        .to_with_name(
            &recipient.email,
            format!("{} {}", recipient.first_name, recipient.last_name),
        )
        .template_id(&email_config.template_id)
        .add_variable("first_name", recipient.first_name.as_str())
        .add_variable("other_user_first_name", other_user.first_name.as_str())
        .add_variable("other_user_last_name", other_user.last_name.as_str())
        .add_variable("other_user_role", other_user_role)
        .add_variable("organization_name", organization.name.as_str())
        .add_variable("session_count", sessions.len() as u64)
        .add_variable("first_session_date", first_session_date.as_str())
        .add_variable("first_session_time", first_session_time.as_str())
        .add_variable("last_session_date", last_session_date.as_str())
        .add_variable("session_duration", session_duration.as_str())
        .add_variable("session_url", session_url.as_str())
        .add_optional_variable("session_or_series", session_or_series)
        .add_ics_attachment(ics_body, &ical::Method::Request)
        .build()
        .await?;

    email_config.client.send_email(email_request).await
}

/// Build the series invite `.ics` body (VEVENT + RRULE). Pure: `dtstamp` injected.
fn build_series_invite_ics(
    organizer: &users::Model,
    attendee: &users::Model,
    first_session: &coaching_sessions::Model,
    organization: &organizations::Model,
    series: &coaching_session_series::Model,
    description: String,
    dtstamp: chrono::NaiveDateTime,
) -> Result<String, Error> {
    let anchor_tz = organizer
        .timezone
        .parse::<chrono_tz::Tz>()
        .unwrap_or(chrono_tz::UTC);
    let rule: SeriesRule = serde_json::from_value(series.rule.clone())?;
    let invite = ical::IcsInvite {
        uid: format!("{}@myrefactor.com", series.id),
        sequence: series.ical_sequence,
        method: ical::Method::Request,
        status: ical::EventStatus::Confirmed,
        summary: format!("Coaching Session: {}", organization.name),
        description,
        anchor_tz,
        dtstamp,
        start: first_session.date,
        duration_minutes: first_session.duration_minutes,
        organizer,
        attendee,
        location_url: first_session.meeting_url.clone(),
        recurrence: Some(rule.recurrence),
    };
    ical::build(&invite)
}

/// Send series invite emails to both coach and coachee. Generic over the notification
/// type `N` so the scheduled and reschedule flows share one body; they differ only by
/// template (via `N`) and the `session_or_series` template variable. A reschedule passes
/// an already-bumped `series`, so the `.ics` carries the next `SEQUENCE` under a stable
/// `UID`.
#[allow(clippy::too_many_arguments)]
async fn send_series_invite_email<N: EmailNotification>(
    db: &DatabaseConnection,
    config: &Config,
    series: &coaching_session_series::Model,
    coach: &users::Model,
    coachee: &users::Model,
    sessions: &[coaching_sessions::Model],
    organization: &organizations::Model,
    session_or_series: Option<&str>,
) -> Result<(), Error> {
    info!(
        "Initiating {} emails for {} sessions (coach: {}, coachee: {})",
        N::notification_name(),
        sessions.len(),
        coach.email,
        coachee.email
    );

    let email_config = ResolvedEmailConfig::new::<N>(config).await?;

    let first = sessions.first().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
            "Cannot send recurring sessions email: sessions slice is empty".to_string(),
        )),
    })?;

    // Series description links to the first session and lists only its in-progress goals.
    let first_goal_titles =
        entity_api::coaching_session_goal::find_in_progress_goals_by_coaching_session_id(
            db, first.id,
        )
        .await?
        .into_iter()
        .filter_map(|g| g.title)
        .collect::<Vec<String>>();

    let anchor_tz = coach
        .timezone
        .parse::<chrono_tz::Tz>()
        .unwrap_or(chrono_tz::UTC);
    let description = ical::compose_description(&DescriptionParts {
        session_url: email_config.build_session_url(&first.id)?,
        title: None,
        topics: &[],
        goal_titles: &first_goal_titles,
        open_actions: &[],
        anchor_tz,
    });

    let ics_body = build_series_invite_ics(
        coach,
        coachee,
        first,
        organization,
        series,
        description,
        chrono::Utc::now().naive_utc(),
    )?;

    if let Err(e) = send_recurring_series_email_to_recipient(
        &email_config,
        coachee,
        coach,
        "coach",
        sessions,
        organization,
        &ics_body,
        session_or_series,
    )
    .await
    {
        warn!(
            "Failed to send {} email to coachee {}: {e:?}",
            N::notification_name(),
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
        &ics_body,
        session_or_series,
    )
    .await
    {
        warn!(
            "Failed to send {} email to coach {}: {e:?}",
            N::notification_name(),
            coach.email
        );
    }

    Ok(())
}

/// Send recurring-sessions-scheduled notification emails to both coach and coachee.
async fn send_recurring_sessions_scheduled_email(
    db: &DatabaseConnection,
    config: &Config,
    series: &coaching_session_series::Model,
    coach: &users::Model,
    coachee: &users::Model,
    sessions: &[coaching_sessions::Model],
    organization: &organizations::Model,
) -> Result<(), Error> {
    send_series_invite_email::<RecurringSessionsScheduled>(
        db,
        config,
        series,
        coach,
        coachee,
        sessions,
        organization,
        None,
    )
    .await
}

/// Send series-rescheduled notification emails to both coach and coachee. `series` must
/// already carry the bumped `ical_sequence`, so the invite updates the recurring calendar
/// event in place (same `UID`, next `SEQUENCE`).
async fn send_series_rescheduled_email(
    db: &DatabaseConnection,
    config: &Config,
    series: &coaching_session_series::Model,
    coach: &users::Model,
    coachee: &users::Model,
    sessions: &[coaching_sessions::Model],
    organization: &organizations::Model,
) -> Result<(), Error> {
    send_series_invite_email::<SeriesRescheduled>(
        db,
        config,
        series,
        coach,
        coachee,
        sessions,
        organization,
        Some("series"),
    )
    .await
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
    series: &coaching_session_series::Model,
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

        send_recurring_sessions_scheduled_email(
            db, config, series, &coach, &coachee, sessions, &org,
        )
        .await
    }
    .await;

    if let Err(e) = result {
        warn!(
            "Failed to send recurring sessions scheduled emails for {} sessions: {e:?}",
            sessions.len()
        );
    }
}

/// Orchestrate sending series-rescheduled emails (best-effort).
///
/// `series` must be the post-update model, so its `ical_sequence` is already
/// incremented. Looks up the coaching relationship, both users, and the organization,
/// then re-sends the series invite to both coach and coachee so their recurring calendar
/// event updates in place. A reschedule can legitimately leave no future sessions, in
/// which case there is nothing to invite anyone to. Errors are logged internally and
/// never block or fail the calling operation.
pub async fn notify_series_rescheduled(
    db: &DatabaseConnection,
    config: &Config,
    series: &coaching_session_series::Model,
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

        send_series_rescheduled_email(db, config, series, &coach, &coachee, sessions, &org).await
    }
    .await;

    if let Err(e) = result {
        warn!(
            "Failed to send series rescheduled emails for series {}: {e:?}",
            series.id
        );
    }
}

/// Build the series cancellation `.ics` body (VEVENT + RRULE). Pure: `dtstamp` injected.
/// Keeps the invite's `UID`, `DTSTART`, and `RRULE` so calendar clients match the
/// cancellation to the recurring event they already hold.
fn build_series_cancel_ics(
    organizer: &users::Model,
    attendee: &users::Model,
    first_session: &coaching_sessions::Model,
    organization: &organizations::Model,
    series: &coaching_session_series::Model,
    description: String,
    dtstamp: chrono::NaiveDateTime,
) -> Result<String, Error> {
    let anchor_tz = organizer
        .timezone
        .parse::<chrono_tz::Tz>()
        .unwrap_or(chrono_tz::UTC);
    let rule: SeriesRule = serde_json::from_value(series.rule.clone())?;
    let invite = ical::IcsInvite {
        uid: format!("{}@myrefactor.com", series.id),
        // In-memory bump only: the row is being deleted, so persisting it is a wasted write.
        sequence: series.ical_sequence + 1,
        method: ical::Method::Cancel,
        status: ical::EventStatus::Cancelled,
        summary: format!("Coaching Session: {}", organization.name),
        description,
        anchor_tz,
        dtstamp,
        start: first_session.date,
        duration_minutes: first_session.duration_minutes,
        organizer,
        attendee,
        location_url: first_session.meeting_url.clone(),
        recurrence: Some(rule.recurrence),
    };
    ical::build(&invite)
}

/// Send a series-cancelled notification email to a single recipient. Carries fewer
/// variables than the invite sends: no `session_url`, no duration, no first-session time.
async fn send_series_cancelled_email_to_recipient(
    email_config: &ResolvedEmailConfig,
    recipient: &users::Model,
    other_user: &users::Model,
    other_user_role: &str,
    sessions: &[coaching_sessions::Model],
    organization: &organizations::Model,
    ics_body: &str,
) -> Result<(), Error> {
    let (Some(first), Some(last)) = (sessions.first(), sessions.last()) else {
        return Err(Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Cannot send series cancelled email: sessions slice is empty".to_string(),
            )),
        });
    };

    let (first_session_date, _) = format_session_date_time(first.date, &recipient.timezone);
    let (last_session_date, _) = format_session_date_time(last.date, &recipient.timezone);

    let email_request = SendEmailRequestBuilder::new()
        .from(FROM_ADDRESS)
        .to_with_name(
            &recipient.email,
            format!("{} {}", recipient.first_name, recipient.last_name),
        )
        .template_id(&email_config.template_id)
        .add_variable("first_name", recipient.first_name.as_str())
        .add_variable("other_user_first_name", other_user.first_name.as_str())
        .add_variable("other_user_last_name", other_user.last_name.as_str())
        .add_variable("other_user_role", other_user_role)
        .add_variable("organization_name", organization.name.as_str())
        .add_variable("session_count", sessions.len() as u64)
        .add_variable("first_session_date", first_session_date.as_str())
        .add_variable("last_session_date", last_session_date.as_str())
        .add_ics_attachment(ics_body, &ical::Method::Cancel)
        .build()
        .await?;

    email_config.client.send_email(email_request).await
}

/// Send series-cancelled emails to both coach and coachee. Takes no database handle:
/// a cancellation loads no topics, goals, or actions.
async fn send_series_cancelled_email(
    config: &Config,
    series: &coaching_session_series::Model,
    coach: &users::Model,
    coachee: &users::Model,
    sessions: &[coaching_sessions::Model],
    organization: &organizations::Model,
) -> Result<(), Error> {
    info!(
        "Initiating series cancelled emails for {} sessions (coach: {}, coachee: {})",
        sessions.len(),
        coach.email,
        coachee.email
    );

    let email_config = ResolvedEmailConfig::new::<SeriesCancelled>(config).await?;

    let first = sessions.first().ok_or_else(|| Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
            "Cannot send series cancelled email: sessions slice is empty".to_string(),
        )),
    })?;

    let ics_body = build_series_cancel_ics(
        coach,
        coachee,
        first,
        organization,
        series,
        SERIES_CANCELLED_DESCRIPTION.to_string(),
        chrono::Utc::now().naive_utc(),
    )?;

    if let Err(e) = send_series_cancelled_email_to_recipient(
        &email_config,
        coachee,
        coach,
        "coach",
        sessions,
        organization,
        &ics_body,
    )
    .await
    {
        warn!(
            "Failed to send series cancelled email to coachee {}: {e:?}",
            coachee.email
        );
    }

    if let Err(e) = send_series_cancelled_email_to_recipient(
        &email_config,
        coach,
        coachee,
        "coachee",
        sessions,
        organization,
        &ics_body,
    )
    .await
    {
        warn!(
            "Failed to send series cancelled email to coach {}: {e:?}",
            coach.email
        );
    }

    Ok(())
}

/// Orchestrate sending series-cancelled emails (best-effort).
///
/// Call this with the in-memory models after the delete has committed. A series row is
/// deleted even when nothing upcoming remains, so an empty `sessions` slice returns
/// before any lookup rather than announcing zero cancelled sessions. Errors are logged
/// internally and never block the caller.
pub async fn notify_series_cancelled(
    db: &DatabaseConnection,
    config: &Config,
    series: &coaching_session_series::Model,
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

        send_series_cancelled_email(config, series, &coach, &coachee, sessions, &org).await
    }
    .await;

    if let Err(e) = result {
        warn!(
            "Failed to send series cancelled emails for series {}: {e:?}",
            series.id
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

    /// `Matcher::Json` for a Resend body that guards against a `subject` key —
    /// Resend templates own the subject line; payload-level subject is a bug.
    fn expect_resend_body(expected: serde_json::Value) -> mockito::Matcher {
        assert!(
            expected.get("subject").is_none(),
            "test bug: expected body must not include `subject`",
        );
        mockito::Matcher::Json(expected)
    }

    /// Body matcher for a Resend request that also carries an `.ics` attachment:
    /// partial-JSON on the template vars plus regexes requiring the `invite.ics`
    /// attachment for the given `method`. The base64 `content` is intentionally not
    /// asserted (dtstamp is `now`).
    fn expect_resend_body_with_ics(
        expected: serde_json::Value,
        method: &ical::Method,
    ) -> mockito::Matcher {
        assert!(
            expected.get("subject").is_none(),
            "test bug: expected body must not include `subject`",
        );
        let method_name = match method {
            ical::Method::Request => "REQUEST",
            ical::Method::Cancel => "CANCEL",
        };
        mockito::Matcher::AllOf(vec![
            mockito::Matcher::PartialJson(expected),
            mockito::Matcher::Regex(r#""filename":"invite\.ics""#.to_string()),
            mockito::Matcher::Regex(format!(
                r#""content_type":"text/calendar; method={method_name}; charset=UTF-8""#
            )),
        ])
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
            default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
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
            default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
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
            coaching_session_series_id: None,
            ical_sequence: 0,
            collab_document_name: None,
            date: NaiveDate::from_ymd_opt(2026, 3, 4)
                .unwrap()
                .and_hms_opt(15, 0, 0)
                .unwrap(),
            duration_minutes: crate::duration::Duration::default_minutes(),
            title: None,
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
            archived_at: None,
            archived_by: None,
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
            "--session-rescheduled-email-template-id=session_reschedule_template_abc",
            "--series-rescheduled-email-template-id=series_reschedule_template_abc",
            "--session-cancelled-email-template-id=session_cancel_template_abc",
            "--series-cancelled-email-template-id=series_cancel_template_abc",
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
            .match_body(expect_resend_body(serde_json::json!({
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
            .match_body(expect_resend_body(serde_json::json!({
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

    // ── Password Reset Email Tests ─────────────────────────────────────

    #[tokio::test]
    async fn test_send_password_reset_email_wire_contract() {
        // Pins the wire contract that the Resend template depends on. The
        // gateway-level tests already prove the builder/HTTP plumbing works;
        // this test exists to catch regressions that are unique to *how this
        // function wires up* the request:
        //   1. `from` is the `mail.` subdomain — apex would not be verified
        //      in Resend and prod sends would silently fail.
        //   2. Variable keys are exactly `first_name`, `last_name`,
        //      `password_reset_url` — a rename would render to empty strings
        //      in the recipient's inbox (Resend still returns 200).
        //   3. The URL substitutes `{token}` (not `{session_id}` like other
        //      email types) — a copy-paste regression would send a malformed
        //      reset link.
        //   4. No `subject` field is present in the JSON payload — main moved
        //      subjects to be template-owned (commit 172907a); re-adding
        //      `.subject(...)` here would override the template default.
        let mut server = setup_test_server().await;
        let user = create_test_user_with("John", "Doe", "john@example.com", "UTC");
        let config = Config::from_args([
            "test",
            "--resend-api-key=test_api_key_123",
            "--password-reset-email-template-id=pw_reset_template_test",
            "--password-reset-email-url-path=/reset-password/{token}",
            "--frontend-base-url=https://app.example.com",
            &format!("--resend-base-url={}", server.url()),
        ]);

        // `Matcher::Json` is structural — any extra field (e.g. an
        // accidentally-readded `subject`) or missing/renamed variable will
        // fail the mock match and the test will hang on `expect(1)`.
        let _mock = server
            .mock("POST", "/emails")
            .match_header("authorization", "Bearer test_api_key_123")
            .match_body(expect_resend_body(serde_json::json!({
                "from": FROM_ADDRESS,
                "to": ["\"John Doe\" <john@example.com>"],
                "template": {
                    "id": "pw_reset_template_test",
                    "variables": {
                        "first_name": "John",
                        "last_name": "Doe",
                        "password_reset_url": "https://app.example.com/reset-password/raw-reset-token-abc"
                    }
                }
            })))
            .with_status(200)
            .with_body(r#"{"id":"email_pw_reset"}"#)
            .expect(1)
            .create_async()
            .await;

        let result = send_password_reset_email(&config, &user, "raw-reset-token-abc").await;
        assert!(
            result.is_ok(),
            "send_password_reset_email failed: {result:?}"
        );
    }

    // ── Session Scheduled Email Tests ──────────────────────────────────

    #[test]
    fn test_build_session_invite_ics_structure() {
        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let mut session = create_test_session();
        session.date = NaiveDate::from_ymd_opt(2026, 9, 15)
            .unwrap()
            .and_hms_opt(19, 0, 0)
            .unwrap();
        session.duration_minutes = 60;
        session.ical_sequence = 0;
        session.meeting_url = Some("https://meet.example/xyz".into());
        let mut org = create_test_organization();
        org.name = "Acme".to_string();
        let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();

        let ics = build_session_invite_ics(
            &coach,
            &coachee,
            &session,
            &org,
            "View this session: https://app/x".into(),
            dtstamp,
        )
        .unwrap();

        assert!(ics.contains(&format!("UID:{}@myrefactor.com", session.id)));
        assert!(ics.contains("METHOD:REQUEST"));
        assert!(ics.contains("STATUS:CONFIRMED"));
        assert!(ics.contains("SEQUENCE:0"));
        assert!(ics.contains("SUMMARY:Coaching Session: Acme"));
        assert!(ics.contains("BEGIN:VTIMEZONE"));
        assert!(ics.contains("TZID:America/New_York"));
        assert!(ics.contains("DTSTART;TZID=America/New_York:20260915T150000"));
        assert!(ics.contains("View this session: https://app/x"));
    }

    /// A reschedule bumps `ical_sequence`; the invite must carry the bumped
    /// `SEQUENCE` while keeping the same `UID` (so calendar clients update the
    /// existing event in place rather than creating a duplicate).
    #[test]
    fn test_build_session_invite_ics_bumped_sequence() {
        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let mut session = create_test_session();
        session.date = NaiveDate::from_ymd_opt(2026, 9, 15)
            .unwrap()
            .and_hms_opt(19, 0, 0)
            .unwrap();
        session.duration_minutes = 60;
        session.ical_sequence = 1;
        session.meeting_url = Some("https://meet.example/xyz".into());
        let mut org = create_test_organization();
        org.name = "Acme".to_string();
        let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();

        let ics = build_session_invite_ics(
            &coach,
            &coachee,
            &session,
            &org,
            "View this session: https://app/x".into(),
            dtstamp,
        )
        .unwrap();

        assert!(ics.contains("SEQUENCE:1"));
        assert!(ics.contains("METHOD:REQUEST"));
        // UID keys off the session id, unchanged from a SEQUENCE:0 invite.
        assert!(ics.contains(&format!("UID:{}@myrefactor.com", session.id)));
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn test_send_session_scheduled_email_variables() {
        use sea_orm::{DatabaseBackend, MockDatabase};

        let mut server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());

        // Description loaders return empty sets; the `.ics` content is not asserted here.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<entity::coaching_session_topics::Model, _, _>(vec![vec![]])
            .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
            .append_query_results::<entity::actions::Model, _, _>(vec![vec![]])
            .into_connection();

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
        let mock_coachee = server
            .mock("POST", "/emails")
            .match_body(expect_resend_body_with_ics(
                serde_json::json!({
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
                            "session_duration": "1 hour",
                            "session_url": session_url.clone(),
                        }
                    }
                }),
                &ical::Method::Request,
            ))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        // Email to coach — other_user is the coachee, formatted in Tokyo time
        // (the session date rolls forward a day).
        let mock_coach = server
            .mock("POST", "/emails")
            .match_body(expect_resend_body_with_ics(
                serde_json::json!({
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
                            "session_duration": "1 hour",
                            "session_url": session_url.clone(),
                        }
                    }
                }),
                &ical::Method::Request,
            ))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let result =
            send_session_scheduled_email(&db, &config, &coach, &coachee, &session, &org).await;
        assert!(result.is_ok());

        // Sends are best-effort (errors are swallowed), so assert the mocks
        // matched to prove the attachment-bearing bodies actually went out.
        mock_coachee.assert_async().await;
        mock_coach.assert_async().await;
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn test_send_session_rescheduled_email() {
        use sea_orm::{DatabaseBackend, MockDatabase};

        let mut server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());

        // Description loaders return empty sets; the `.ics` content is not asserted here.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<entity::coaching_session_topics::Model, _, _>(vec![vec![]])
            .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
            .append_query_results::<entity::actions::Model, _, _>(vec![vec![]])
            .into_connection();

        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "Asia/Tokyo");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "America/New_York");
        // Already-bumped session: a reschedule invite carries SEQUENCE:2.
        let mut session = create_test_session();
        session.ical_sequence = 2;
        let org = create_test_organization();

        // Both recipients target the reschedule template and carry the
        // `session_or_series=session` discriminant plus the `.ics` attachment.
        let mock_coachee = server
            .mock("POST", "/emails")
            .match_body(expect_resend_body_with_ics(
                serde_json::json!({
                    "to": ["\"Jane Doe\" <jane@example.com>"],
                    "template": {
                        "id": "session_reschedule_template_abc",
                        "variables": {
                            "first_name": "Jane",
                            "other_user_role": "coach",
                            "session_or_series": "session",
                        }
                    }
                }),
                &ical::Method::Request,
            ))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let mock_coach = server
            .mock("POST", "/emails")
            .match_body(expect_resend_body_with_ics(
                serde_json::json!({
                    "to": ["\"Alex Smith\" <alex@example.com>"],
                    "template": {
                        "id": "session_reschedule_template_abc",
                        "variables": {
                            "first_name": "Alex",
                            "other_user_role": "coachee",
                            "session_or_series": "session",
                        }
                    }
                }),
                &ical::Method::Request,
            ))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let result =
            send_session_rescheduled_email(&db, &config, &coach, &coachee, &session, &org).await;
        assert!(result.is_ok());

        // The send swallows errors, so the mock assertions are what give this
        // test teeth: they prove the reschedule template + attachment went out.
        mock_coachee.assert_async().await;
        mock_coach.assert_async().await;
    }

    // ── Session Cancelled Email Tests ──────────────────────────────────

    /// A cancellation must supersede the invite it replaces: same `UID`, next `SEQUENCE`.
    #[test]
    fn test_build_session_cancel_ics_structure() {
        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let mut session = create_test_session();
        session.date = NaiveDate::from_ymd_opt(2026, 9, 15)
            .unwrap()
            .and_hms_opt(19, 0, 0)
            .unwrap();
        session.duration_minutes = 60;
        session.ical_sequence = 2;
        let org = create_test_organization();
        let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();

        let ics = build_session_cancel_ics(
            &coach,
            &coachee,
            &session,
            &org,
            SESSION_CANCELLED_DESCRIPTION.to_string(),
            dtstamp,
        )
        .unwrap();

        assert!(ics.contains("METHOD:CANCEL"));
        assert!(ics.contains("STATUS:CANCELLED"));
        assert!(ics.contains("SEQUENCE:3"));
        assert!(ics.contains(&format!("UID:{}@myrefactor.com", session.id)));
    }

    #[tokio::test]
    async fn test_send_session_cancelled_email() {
        let mut server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());

        // No session link: the row is gone by the time a recipient could click it.
        assert!(
            SessionCancelled::url_path_template(&config).is_none(),
            "a cancellation must not carry a session URL template"
        );

        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "Asia/Tokyo");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "America/New_York");
        let session = create_test_session();
        let org = create_test_organization();

        // Email to coachee: other_user is the coach, formatted in NY time.
        let mock_coachee = server
            .mock("POST", "/emails")
            .match_body(expect_resend_body_with_ics(
                serde_json::json!({
                    "from": FROM_ADDRESS,
                    "to": ["\"Jane Doe\" <jane@example.com>"],
                    "template": {
                        "id": "session_cancel_template_abc",
                        "variables": {
                            "first_name": "Jane",
                            "other_user_first_name": "Alex",
                            "other_user_last_name": "Smith",
                            "other_user_role": "coach",
                            "organization_name": "Acme Corp",
                            "session_date": "Wednesday, March 4, 2026",
                            "session_time": "10:00 AM",
                        }
                    }
                }),
                &ical::Method::Cancel,
            ))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        // Email to coach: other_user is the coachee, Tokyo time rolls the date forward.
        let mock_coach = server
            .mock("POST", "/emails")
            .match_body(expect_resend_body_with_ics(
                serde_json::json!({
                    "from": FROM_ADDRESS,
                    "to": ["\"Alex Smith\" <alex@example.com>"],
                    "template": {
                        "id": "session_cancel_template_abc",
                        "variables": {
                            "first_name": "Alex",
                            "other_user_first_name": "Jane",
                            "other_user_last_name": "Doe",
                            "other_user_role": "coachee",
                            "organization_name": "Acme Corp",
                            "session_date": "Thursday, March 5, 2026",
                            "session_time": "12:00 AM",
                        }
                    }
                }),
                &ical::Method::Cancel,
            ))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let result = send_session_cancelled_email(&config, &coach, &coachee, &session, &org).await;
        assert!(result.is_ok());

        // The send swallows errors, so the mock assertions are what give this test teeth.
        mock_coachee.assert_async().await;
        mock_coach.assert_async().await;
    }

    /// Deleting an already-completed session is housekeeping, not a cancellation. The
    /// guard must fire before any lookup, so the connection sees no statements at all.
    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn test_notify_session_cancelled_skips_past_session() {
        use sea_orm::{DatabaseBackend, MockDatabase};

        let server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());
        let mut session = create_test_session();
        session.date = NaiveDate::from_ymd_opt(2020, 1, 15)
            .unwrap()
            .and_hms_opt(15, 0, 0)
            .unwrap();

        // Zero appended results: any query would panic or error rather than pass.
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        notify_session_cancelled(&db, &config, &session).await;

        assert!(
            db.into_transaction_log().is_empty(),
            "a past session must return before any statement runs"
        );
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn test_send_session_scheduled_email_missing_template_id() {
        use sea_orm::{DatabaseBackend, MockDatabase};

        // Config has an API key and frontend base URL but no session-scheduled
        // template id — mirrors the welcome/action missing-template-id tests.
        // Fails at config resolution, before any loader query runs.
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
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

        let result =
            send_session_scheduled_email(&db, &config, &coach, &coachee, &session, &org).await;

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
            .match_body(expect_resend_body(serde_json::json!({
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
            .match_body(expect_resend_body(serde_json::json!({
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
            .match_body(expect_resend_body(serde_json::json!({
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
            .match_body(expect_resend_body(serde_json::json!({
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

    #[cfg(feature = "mock")]
    fn create_test_session_on(date: NaiveDate) -> coaching_sessions::Model {
        coaching_sessions::Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            coaching_session_series_id: None,
            ical_sequence: 0,
            collab_document_name: None,
            date: date.and_hms_opt(15, 0, 0).unwrap(),
            duration_minutes: crate::duration::Duration::default_minutes(),
            title: None,
            meeting_url: None,
            provider: None,
            created_at: chrono::Utc::now().fixed_offset(),
            updated_at: chrono::Utc::now().fixed_offset(),
            hydrated_at: None,
        }
    }

    fn create_test_series() -> coaching_session_series::Model {
        coaching_session_series::Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            rule: serde_json::json!({
                "start_at": "2026-09-15T15:00:00",
                "recurrence": { "frequency": "weekly", "interval": 1 },
                "duration_minutes": 60,
            }),
            ical_sequence: 0,
            created_by_user_id: Id::new_v4(),
            created_at: chrono::Utc::now().fixed_offset(),
            updated_at: chrono::Utc::now().fixed_offset(),
        }
    }

    #[test]
    fn test_build_series_invite_ics_structure() {
        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let mut first = create_test_session();
        first.date = NaiveDate::from_ymd_opt(2026, 9, 15)
            .unwrap()
            .and_hms_opt(19, 0, 0)
            .unwrap();
        first.duration_minutes = 60;
        first.meeting_url = Some("https://meet.example/xyz".into());
        let mut org = create_test_organization();
        org.name = "Acme".to_string();
        let series = create_test_series();
        let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();

        let ics = build_series_invite_ics(
            &coach,
            &coachee,
            &first,
            &org,
            &series,
            "View this session: https://app/x".into(),
            dtstamp,
        )
        .unwrap();

        assert!(ics.contains(&format!("UID:{}@myrefactor.com", series.id)));
        assert!(ics.contains("METHOD:REQUEST"));
        assert!(ics.contains("STATUS:CONFIRMED"));
        assert!(ics.contains("SEQUENCE:0"));
        assert!(ics.contains("RRULE:FREQ=WEEKLY"));
        assert!(ics.contains("BEGIN:VTIMEZONE"));
        assert!(ics.contains("TZID:America/New_York"));
        assert!(ics.contains("DTSTART;TZID=America/New_York:20260915T150000"));
        assert!(ics.contains("View this session: https://app/x"));
    }

    /// A series reschedule bumps `ical_sequence`; the invite must carry the bumped
    /// `SEQUENCE` under the same series-derived `UID` so calendar clients replace the
    /// existing recurring event instead of duplicating it.
    #[test]
    fn test_build_series_invite_ics_carries_bumped_sequence() {
        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let mut first = create_test_session();
        first.date = NaiveDate::from_ymd_opt(2026, 9, 15)
            .unwrap()
            .and_hms_opt(19, 0, 0)
            .unwrap();
        first.duration_minutes = 60;
        let org = create_test_organization();
        let mut series = create_test_series();
        series.ical_sequence = 3;
        let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();

        let ics = build_series_invite_ics(
            &coach,
            &coachee,
            &first,
            &org,
            &series,
            "View this session: https://app/x".into(),
            dtstamp,
        )
        .unwrap();

        assert!(ics.contains("SEQUENCE:3"));
        assert!(ics.contains(&format!("UID:{}@myrefactor.com", series.id)));
        assert!(ics.contains("RRULE:FREQ=WEEKLY"));
        assert!(ics.contains("METHOD:REQUEST"));
        assert!(ics.contains("STATUS:CONFIRMED"));
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn test_send_recurring_sessions_scheduled_email_personalization() {
        use sea_orm::{DatabaseBackend, MockDatabase};

        let mut server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());

        // Series description loads only the first session's in-progress goals.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
            .into_connection();

        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let org = create_test_organization();
        let series = create_test_series();

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
        // proves the role swap; the `.ics` attachment must be present.
        let mock_coachee = server
            .mock("POST", "/emails")
            .match_body(expect_resend_body_with_ics(
                serde_json::json!({
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
                            "session_count": 3,
                            "first_session_date": "Wednesday, March 4, 2026",
                            "first_session_time": "3:00 PM",
                            "last_session_date": "Wednesday, March 18, 2026",
                            "session_duration": "1 hour",
                            "session_url": first_session_url,
                        }
                    }
                }),
                &ical::Method::Request,
            ))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        // Email to coach — require the `.ics` attachment too.
        let mock_coach = server
            .mock("POST", "/emails")
            .match_body(expect_resend_body_with_ics(
                serde_json::json!({
                    "template": { "id": "recurring_template_xyz" }
                }),
                &ical::Method::Request,
            ))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let result = send_recurring_sessions_scheduled_email(
            &db, &config, &series, &coach, &coachee, &sessions, &org,
        )
        .await;
        assert!(result.is_ok());

        // Sends are best-effort (errors are swallowed), so assert the mocks
        // matched to prove the attachment-bearing bodies actually went out.
        mock_coachee.assert_async().await;
        mock_coach.assert_async().await;
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn test_send_series_rescheduled_email() {
        use sea_orm::{DatabaseBackend, MockDatabase};

        let mut server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());

        // Series description loads only the first session's in-progress goals.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
            .into_connection();

        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let org = create_test_organization();
        // Already-bumped series: a reschedule invite carries SEQUENCE:3.
        let mut series = create_test_series();
        series.ical_sequence = 3;

        let sessions = vec![
            create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 4).unwrap()),
            create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 11).unwrap()),
            create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 18).unwrap()),
        ];

        // Both recipients target the reschedule template and carry the
        // `session_or_series=series` discriminant plus the `.ics` attachment.
        let mock_coachee = server
            .mock("POST", "/emails")
            .match_body(expect_resend_body_with_ics(
                serde_json::json!({
                    "to": ["\"Jane Doe\" <jane@example.com>"],
                    "template": {
                        "id": "series_reschedule_template_abc",
                        "variables": {
                            "first_name": "Jane",
                            "other_user_role": "coach",
                            "session_or_series": "series",
                        }
                    }
                }),
                &ical::Method::Request,
            ))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let mock_coach = server
            .mock("POST", "/emails")
            .match_body(expect_resend_body_with_ics(
                serde_json::json!({
                    "to": ["\"Alex Smith\" <alex@example.com>"],
                    "template": {
                        "id": "series_reschedule_template_abc",
                        "variables": {
                            "first_name": "Alex",
                            "other_user_role": "coachee",
                            "session_or_series": "series",
                        }
                    }
                }),
                &ical::Method::Request,
            ))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let result =
            send_series_rescheduled_email(&db, &config, &series, &coach, &coachee, &sessions, &org)
                .await;
        assert!(result.is_ok());

        // The send swallows errors, so the mock assertions are what give this
        // test teeth: they prove the reschedule template + attachment went out.
        mock_coachee.assert_async().await;
        mock_coach.assert_async().await;
    }

    /// A reschedule can legitimately leave zero future sessions. The early return must
    /// happen before any lookup, so the connection sees no statements at all.
    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn test_notify_series_rescheduled_with_no_sessions_does_nothing() {
        use sea_orm::{DatabaseBackend, MockDatabase};

        let server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());
        let series = create_test_series();

        // Zero appended results: any query would panic or error rather than pass.
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        notify_series_rescheduled(&db, &config, &series, &[]).await;

        assert!(
            db.into_transaction_log().is_empty(),
            "an empty sessions slice must return before any statement runs"
        );
    }

    // ── Series Cancelled Email Tests ───────────────────────────────────

    /// A series cancellation keeps the `UID`, `DTSTART`, and `RRULE` of the invite it
    /// supersedes so clients can match it, and carries the next `SEQUENCE`.
    #[test]
    fn test_build_series_cancel_ics_structure() {
        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "America/New_York");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let mut first = create_test_session();
        first.date = NaiveDate::from_ymd_opt(2026, 9, 15)
            .unwrap()
            .and_hms_opt(19, 0, 0)
            .unwrap();
        first.duration_minutes = 60;
        let org = create_test_organization();
        let mut series = create_test_series();
        series.ical_sequence = 4;
        let dtstamp = NaiveDate::from_ymd_opt(2026, 9, 1)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();

        let ics = build_series_cancel_ics(
            &coach,
            &coachee,
            &first,
            &org,
            &series,
            SERIES_CANCELLED_DESCRIPTION.to_string(),
            dtstamp,
        )
        .unwrap();

        assert!(ics.contains("METHOD:CANCEL"));
        assert!(ics.contains("STATUS:CANCELLED"));
        assert!(ics.contains("SEQUENCE:5"));
        assert!(ics.contains(&format!("UID:{}@myrefactor.com", series.id)));
        assert!(ics.contains("RRULE:FREQ=WEEKLY"));
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn test_send_series_cancelled_email() {
        let mut server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());

        // No session link: the rows are gone by the time a recipient could click one.
        assert!(
            SeriesCancelled::url_path_template(&config).is_none(),
            "a cancellation must not carry a session URL template"
        );

        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let org = create_test_organization();
        let series = create_test_series();

        let sessions = vec![
            create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 4).unwrap()),
            create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 11).unwrap()),
            create_test_session_on(NaiveDate::from_ymd_opt(2026, 3, 18).unwrap()),
        ];

        let mock_coachee = server
            .mock("POST", "/emails")
            .match_body(expect_resend_body_with_ics(
                serde_json::json!({
                    "from": FROM_ADDRESS,
                    "to": ["\"Jane Doe\" <jane@example.com>"],
                    "template": {
                        "id": "series_cancel_template_abc",
                        "variables": {
                            "first_name": "Jane",
                            "other_user_first_name": "Alex",
                            "other_user_last_name": "Smith",
                            "other_user_role": "coach",
                            "organization_name": "Acme Corp",
                            "session_count": 3,
                            "first_session_date": "Wednesday, March 4, 2026",
                            "last_session_date": "Wednesday, March 18, 2026",
                        }
                    }
                }),
                &ical::Method::Cancel,
            ))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let mock_coach = server
            .mock("POST", "/emails")
            .match_body(expect_resend_body_with_ics(
                serde_json::json!({
                    "from": FROM_ADDRESS,
                    "to": ["\"Alex Smith\" <alex@example.com>"],
                    "template": {
                        "id": "series_cancel_template_abc",
                        "variables": {
                            "first_name": "Alex",
                            "other_user_first_name": "Jane",
                            "other_user_last_name": "Doe",
                            "other_user_role": "coachee",
                            "organization_name": "Acme Corp",
                            "session_count": 3,
                            "first_session_date": "Wednesday, March 4, 2026",
                            "last_session_date": "Wednesday, March 18, 2026",
                        }
                    }
                }),
                &ical::Method::Cancel,
            ))
            .with_status(200)
            .with_body(r#"{"id":"email_test"}"#)
            .expect(1)
            .create_async()
            .await;

        let result =
            send_series_cancelled_email(&config, &series, &coach, &coachee, &sessions, &org).await;
        assert!(result.is_ok());

        // The send swallows errors, so the mock assertions are what give this test teeth.
        mock_coachee.assert_async().await;
        mock_coach.assert_async().await;
    }

    /// The series row is deleted even when nothing upcoming remains. The early return must
    /// happen before any lookup, so the connection sees no statements at all.
    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn test_notify_series_cancelled_with_no_sessions_does_nothing() {
        use sea_orm::{DatabaseBackend, MockDatabase};

        let server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());
        let series = create_test_series();

        // Zero appended results: any query would panic or error rather than pass.
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        notify_series_cancelled(&db, &config, &series, &[]).await;

        assert!(
            db.into_transaction_log().is_empty(),
            "an empty sessions slice must return before any statement runs"
        );
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn test_send_recurring_sessions_scheduled_email_single_session() {
        use sea_orm::{DatabaseBackend, MockDatabase};

        let mut server = setup_test_server().await;
        let config = create_full_config_with_mock(&server.url());

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<entity::goals::Model, _, _>(vec![vec![]])
            .into_connection();

        let coach = create_test_user_with("Alex", "Smith", "alex@example.com", "UTC");
        let coachee = create_test_user_with("Jane", "Doe", "jane@example.com", "UTC");
        let org = create_test_organization();
        let series = create_test_series();

        let sessions = vec![create_test_session_on(
            NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
        )];

        // With a single session, first and last dates must match.
        let _mock_coachee = server
            .mock("POST", "/emails")
            .match_body(mockito::Matcher::PartialJson(serde_json::json!({
                "template": {
                    "variables": {
                        "session_count": 1,
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

        let result = send_recurring_sessions_scheduled_email(
            &db, &config, &series, &coach, &coachee, &sessions, &org,
        )
        .await;
        assert!(result.is_ok());
    }

    #[cfg(feature = "mock")]
    #[tokio::test]
    async fn test_send_recurring_sessions_scheduled_email_missing_template_id() {
        use sea_orm::{DatabaseBackend, MockDatabase};

        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
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
        let series = create_test_series();
        let sessions = vec![create_test_session_on(
            NaiveDate::from_ymd_opt(2026, 3, 4).unwrap(),
        )];

        let result = send_recurring_sessions_scheduled_email(
            &db, &config, &series, &coach, &coachee, &sessions, &org,
        )
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
            "",
            None,
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
