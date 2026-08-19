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

#[cfg(test)]
#[path = "emails_tests.rs"]
mod tests;

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

struct RecurringSessionsRescheduled;
impl EmailNotification for RecurringSessionsRescheduled {
    fn template_id(config: &Config) -> Option<String> {
        config.recurring_sessions_rescheduled_email_template_id()
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
struct RecurringSessionsCancelled;
impl EmailNotification for RecurringSessionsCancelled {
    fn template_id(config: &Config) -> Option<String> {
        config.recurring_sessions_cancelled_email_template_id()
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

/// The first and last session of a series slice, which every series email needs to
/// render its date range. Unreachable in practice: each `notify_*` caller returns
/// early on an empty slice. One guard rather than three keeps that fact in one place.
fn series_bounds(
    sessions: &[coaching_sessions::Model],
) -> Result<(&coaching_sessions::Model, &coaching_sessions::Model), Error> {
    match (sessions.first(), sessions.last()) {
        (Some(first), Some(last)) => Ok((first, last)),
        _ => Err(Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                "Cannot send series email: sessions slice is empty".to_string(),
            )),
        }),
    }
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

/// One-line start, e.g. `Tuesday, February 9, 2027 at 4:00 PM`, in the recipient's
/// timezone. The connector lives inside the value because Resend templates cannot
/// suppress literal text around a variable.
fn format_session_when(date: NaiveDateTime, timezone: &str) -> String {
    let (date_str, time_str) = format_session_date_time(date, timezone);
    format!("{date_str} at {time_str}")
}

/// Rendered as the `previous_session_when` value when the start did not move.
const UNCHANGED_SESSION_WHEN: &str = "Unchanged";

/// The previous start in the recipient's timezone, or [`UNCHANGED_SESSION_WHEN`] when
/// the start did not move (a title, meeting URL, or duration-only reschedule).
fn format_previous_session_when(
    previous_start: NaiveDateTime,
    current_start: NaiveDateTime,
    timezone: &str,
) -> String {
    if previous_start != current_start {
        format_session_when(previous_start, timezone)
    } else {
        UNCHANGED_SESSION_WHEN.to_string()
    }
}

/// The previous cadence phrase, or [`UNCHANGED_SESSION_WHEN`] when the series still
/// repeats as often as it did.
fn format_previous_recurrence_summary(previous: &str, current: &str) -> String {
    if previous != current {
        previous.to_string()
    } else {
        UNCHANGED_SESSION_WHEN.to_string()
    }
}

/// The series as it stood before a reschedule. A newtype so the call site names which
/// model is which: both are the same type, so bare references read ambiguously.
pub struct PreviousSeries<'a>(pub &'a coaching_session_series::Model);

/// True when an edit changes something the invite carries, so the calendar needs a fresh
/// `.ics` under the next `SEQUENCE`.
///
/// Lives here rather than with the session entity because it is a statement about invite
/// content, not about the session: `title` qualifies only because it rides in the `.ics`
/// DESCRIPTION. Anything added to an invite has to be added here too.
///
/// Known gap: the DESCRIPTION also carries topics, goals, and open actions, which are
/// edited through their own endpoints and so cannot be seen by a before/after comparison
/// of the session row. Editing those does not currently re-send.
///
/// Must stay pure and synchronous. The caller runs it inside the update transaction to
/// decide whether to bump `SEQUENCE`, which commits with the edit; the email that follows
/// is best-effort and cannot be what makes the decision.
pub fn affects_invite(old: &coaching_sessions::Model, new: &coaching_sessions::Model) -> bool {
    old.date != new.date
        || old.duration_minutes != new.duration_minutes
        || old.meeting_url != new.meeting_url
        || old.title != new.title
}

/// Whether an edit moved the session in time, and so earns a "has been rescheduled"
/// notification. Deliberately narrower than [`affects_invite`]: an edit can need a fresh
/// invite without any human needing to be told their session moved, and the reschedule
/// copy announces a start and a duration it would otherwise have to describe as
/// unchanged.
///
/// Every field here is also in [`affects_invite`], so a send always rides on an edit that
/// already bumped `SEQUENCE`.
pub fn affects_schedule(old: &coaching_sessions::Model, new: &coaching_sessions::Model) -> bool {
    old.date != new.date || old.duration_minutes != new.duration_minutes
}

/// The two people on a coaching session, carried as one named value rather than as an
/// adjacent pair of `&users::Model`. A transposed pair still type-checks and would swap
/// every "your coach" / "your coachee" phrase in the copy without failing a build.
struct Participants<'a> {
    coach: &'a users::Model,
    coachee: &'a users::Model,
}

/// One addressee of a per-recipient send, plus the other party as that recipient sees them.
/// These three always travel together and are meaningless apart: `other_user_role` labels
/// `other_user`, not the recipient.
struct Recipient<'a> {
    user: &'a users::Model,
    other_user: &'a users::Model,
    /// How the copy refers to the other party: "coach" or "coachee".
    other_user_role: &'a str,
}

/// Template variables carried only by the reschedule sends: the discriminant the
/// shared template copy reads, plus the start the recipient previously held.
struct RescheduleVars {
    session_or_series: &'static str,
    previous_start: NaiveDateTime,
    /// How the series repeated before the reschedule. `None` for a single session,
    /// which has no cadence to report.
    previous_recurrence_summary: Option<String>,
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

/// `UID` domain. Not a sending address: RFC 5545 only requires global uniqueness.
const UID_DOMAIN: &str = "myrefactor.com";

/// Display name paired with `FROM_ADDRESS` on the `.ics` `ORGANIZER`.
const FROM_DISPLAY_NAME: &str = "Refactor Coach";

/// The zone every `.ics` for a session is anchored to. The coach owns the schedule, so
/// their zone is the one whose DST rules the emitted `VTIMEZONE` must follow.
fn anchor_tz(coach: &users::Model) -> chrono_tz::Tz {
    coach.timezone.parse().unwrap_or(chrono_tz::UTC)
}

/// A globally unique, stable `UID` for a calendar event. Stability is the whole
/// mechanism: a client matches an update to the event it supersedes by `UID`.
fn ics_uid(id: Id) -> String {
    format!("{id}@{UID_DOMAIN}")
}

/// The event title as it appears on a calendar.
fn session_summary(organization: &organizations::Model) -> String {
    format!("Coaching Session: {}", organization.name)
}

/// The platform organizes every invite: calendar clients only apply updates when the
/// `ORGANIZER` matches the sending address.
fn platform_organizer() -> ical::Participant<'static> {
    ical::Participant::new(FROM_DISPLAY_NAME, FROM_ADDRESS)
}

/// A user as a calendar participant. The mapping lives here rather than on
/// `ical::Participant` so the builder stays free of entity types.
fn participant(user: &users::Model) -> ical::Participant<'_> {
    let name = user
        .display_name
        .clone()
        .unwrap_or_else(|| format!("{} {}", user.first_name, user.last_name));
    ical::Participant::new(&name, &user.email)
}

/// Coach first, then coachee, so both humans see each other in the guest list.
fn session_attendees<'a>(
    coach: &'a users::Model,
    coachee: &'a users::Model,
) -> Vec<ical::Participant<'a>> {
    vec![participant(coach), participant(coachee)]
}

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
    to: &Recipient<'_>,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
    ics_body: Option<&str>,
    reschedule: Option<&RescheduleVars>,
) -> Result<(), Error> {
    let recipient = to.user;
    let (session_date, session_time) = format_session_date_time(session.date, &recipient.timezone);
    let session_url = email_config.build_session_url(&session.id)?;
    let session_duration =
        crate::duration::Duration::from_minutes_unchecked(session.duration_minutes).to_string();

    let builder = SendEmailRequestBuilder::new()
        .from(FROM_ADDRESS)
        .to_with_name(
            &recipient.email,
            format!("{} {}", recipient.first_name, recipient.last_name),
        )
        .template_id(&email_config.template_id)
        .add_variable("first_name", recipient.first_name.as_str())
        .add_variable("other_user_first_name", to.other_user.first_name.as_str())
        .add_variable("other_user_last_name", to.other_user.last_name.as_str())
        .add_variable("other_user_role", to.other_user_role)
        .add_variable("organization_name", organization.name.as_str())
        .add_variable("session_date", session_date.as_str())
        .add_variable("session_time", session_time.as_str())
        .add_variable("session_duration", session_duration.as_str())
        .add_variable("session_url", session_url.as_str())
        .add_optional_variable("session_or_series", reschedule.map(|r| r.session_or_series))
        .add_optional_ics_attachment(ics_body, &ical::Method::Request);

    // The reschedule template declares both keys, so they ship together or not at all.
    let email_request = match reschedule {
        Some(r) => builder
            .add_variable(
                "session_when",
                format_session_when(session.date, &recipient.timezone),
            )
            .add_variable(
                "previous_session_when",
                format_previous_session_when(r.previous_start, session.date, &recipient.timezone),
            ),
        None => builder,
    }
    .build()
    .await?;

    email_config.client.send_email(email_request).await
}

/// Build the single-session invite `.ics` body. Pure: `dtstamp` is injected so the
/// output is deterministic for a given input.
fn build_session_invite_ics(
    coach: &users::Model,
    coachee: &users::Model,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
    description: String,
    dtstamp: chrono::NaiveDateTime,
) -> Result<String, Error> {
    let anchor_tz = anchor_tz(coach);
    let invite = ical::IcsInvite {
        uid: ics_uid(session.id),
        sequence: session.ical_sequence,
        method: ical::Method::Request,
        status: ical::EventStatus::Confirmed,
        summary: session_summary(organization),
        description,
        anchor_tz,
        dtstamp,
        start: session.date,
        duration_minutes: session.duration_minutes,
        organizer: platform_organizer(),
        attendees: session_attendees(coach, coachee),
        location_url: session.meeting_url.clone(),
        recurrence: None,
        recurrence_id: None,
    };
    ical::build(&invite)
}

/// Build the `.ics` body that moves ONE occurrence of a recurring series. Addressed by
/// the series `UID` plus the occurrence's original start as `RECURRENCE-ID`, so clients
/// override the existing instance instead of creating a standalone duplicate.
/// Pure: `dtstamp` is injected.
fn build_occurrence_reschedule_ics(
    coach: &users::Model,
    coachee: &users::Model,
    session: &coaching_sessions::Model,
    series_id: Id,
    organization: &organizations::Model,
    description: String,
    dtstamp: chrono::NaiveDateTime,
) -> Result<String, Error> {
    let anchor_tz = anchor_tz(coach);
    let invite = ical::IcsInvite {
        uid: ics_uid(series_id),
        sequence: session.ical_sequence,
        method: ical::Method::Request,
        status: ical::EventStatus::Confirmed,
        summary: session_summary(organization),
        description,
        anchor_tz,
        dtstamp,
        // The NEW start; `recurrence_id` keeps addressing the original slot.
        start: session.date,
        duration_minutes: session.duration_minutes,
        organizer: platform_organizer(),
        attendees: session_attendees(coach, coachee),
        location_url: session.meeting_url.clone(),
        recurrence: None,
        recurrence_id: session.ical_recurrence_id,
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
    reschedule: Option<&RescheduleVars>,
) -> Result<(), Error> {
    info!(
        "Initiating {} emails for session: {} (coach: {}, coachee: {})",
        N::notification_name(),
        session.id,
        coach.email,
        coachee.email
    );

    let email_config = ResolvedEmailConfig::new::<N>(config).await?;

    // Same VEVENT for both recipients.
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

    let anchor_tz = anchor_tz(coach);
    let description = ical::compose_description(&DescriptionParts {
        session_url: email_config.build_session_url(&session.id)?,
        title: session.title.as_deref(),
        topics: &topics,
        goal_titles: &goal_titles,
        open_actions: &open_actions,
        anchor_tz,
    });

    let dtstamp = chrono::Utc::now().naive_utc();
    // A session inside a series is addressed as an override of its occurrence. One that
    // predates `ical_recurrence_id` has no such address, so it goes out without an
    // attachment rather than not going out at all.
    let ics_body = build_session_ics(
        session,
        description,
        |series_id, description| {
            build_occurrence_reschedule_ics(
                coach,
                coachee,
                session,
                series_id,
                organization,
                description,
                dtstamp,
            )
        },
        |description| {
            build_session_invite_ics(coach, coachee, session, organization, description, dtstamp)
        },
    )?;

    // Email to coachee: "Your coach, ... has a session with you"
    if let Err(e) = send_session_email_to_recipient(
        &email_config,
        &Recipient {
            user: coachee,
            other_user: coach,
            other_user_role: "coach",
        },
        session,
        organization,
        ics_body.as_deref(),
        reschedule,
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
        &Recipient {
            user: coach,
            other_user: coachee,
            other_user_role: "coachee",
        },
        session,
        organization,
        ics_body.as_deref(),
        reschedule,
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
/// event in place (same `UID`, next `SEQUENCE`). `previous_start` is the start the
/// recipients last saw; equal to the new start when only a non-time field changed.
async fn send_session_rescheduled_email(
    db: &DatabaseConnection,
    config: &Config,
    coach: &users::Model,
    coachee: &users::Model,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
    previous_start: NaiveDateTime,
) -> Result<(), Error> {
    send_single_session_invite_email::<SessionRescheduled>(
        db,
        config,
        coach,
        coachee,
        session,
        organization,
        Some(&RescheduleVars {
            session_or_series: "session",
            previous_start,
            previous_recurrence_summary: None,
        }),
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

/// The invite for one session, or `None` when its occurrence cannot be addressed.
///
/// A session inside a series is addressed as an override of its occurrence; a standalone
/// one by its own `UID`. A series member materialized before `ical_recurrence_id` existed
/// has neither, so it goes out with no attachment rather than not going out at all.
///
/// `description` is passed through to whichever builder runs, so it moves exactly once.
fn build_session_ics(
    session: &coaching_sessions::Model,
    description: String,
    build_occurrence: impl FnOnce(Id, String) -> Result<String, Error>,
    build_standalone: impl FnOnce(String) -> Result<String, Error>,
) -> Result<Option<String>, Error> {
    match (
        session.coaching_session_series_id,
        session.ical_recurrence_id,
    ) {
        (Some(series_id), Some(_)) => build_occurrence(series_id, description).map(Some),
        (Some(_), None) => {
            warn_unaddressable(session);
            Ok(None)
        }
        (None, _) => build_standalone(description).map(Some),
    }
}

/// A series member that predates `ical_recurrence_id` has no valid `RECURRENCE-ID`, so no
/// invite can address its occurrence. The email still goes out; only the attachment is
/// withheld. These sessions were materialized before invites existed, so no calendar holds
/// an event for them and a `CANCEL` or update naming the series `UID` would act on the
/// wrong instance.
fn warn_unaddressable(session: &coaching_sessions::Model) {
    warn!(
        "Sending email without an invite for session {}: series member predates \
         ical_recurrence_id, so its occurrence cannot be addressed",
        session.id
    );
}

/// Orchestrate sending session-rescheduled emails (best-effort).
///
/// `session` must already carry the bumped `ical_sequence`. Looks up the coaching
/// relationship, both users, and the organization, then re-sends the invite to both
/// coach and coachee so their calendar event updates in place. `previous_start` is the
/// pre-update start, shown alongside the new one. Errors are logged internally and never
/// block or fail the calling operation.
pub async fn notify_session_rescheduled(
    db: &DatabaseConnection,
    config: &Config,
    session: &coaching_sessions::Model,
    previous_start: NaiveDateTime,
) {
    let result: Result<(), Error> = async {
        let relationship =
            coaching_relationship::find_by_id(db, session.coaching_relationship_id).await?;
        let coach = user::find_by_id(db, relationship.coach_id).await?;
        let coachee = user::find_by_id(db, relationship.coachee_id).await?;
        let org = organization::find_by_id(db, relationship.organization_id).await?;

        send_session_rescheduled_email(db, config, &coach, &coachee, session, &org, previous_start)
            .await
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
    coach: &users::Model,
    coachee: &users::Model,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
    description: String,
    dtstamp: chrono::NaiveDateTime,
) -> Result<String, Error> {
    let anchor_tz = anchor_tz(coach);
    let invite = ical::IcsInvite {
        uid: ics_uid(session.id),
        // Already bumped by the caller inside the delete transaction, so the cancellation
        // outranks any edit that committed alongside it.
        sequence: session.ical_sequence,
        method: ical::Method::Cancel,
        status: ical::EventStatus::Cancelled,
        summary: session_summary(organization),
        description,
        anchor_tz,
        dtstamp,
        start: session.date,
        duration_minutes: session.duration_minutes,
        organizer: platform_organizer(),
        attendees: session_attendees(coach, coachee),
        location_url: session.meeting_url.clone(),
        recurrence: None,
        recurrence_id: None,
    };
    ical::build(&invite)
}

/// Build the `.ics` body that cancels ONE occurrence of a recurring series. Addressed by
/// the series `UID` plus the occurrence's original start as `RECURRENCE-ID`, so clients
/// remove the instance they already hold. Pure: `dtstamp` is injected.
fn build_occurrence_cancel_ics(
    coach: &users::Model,
    coachee: &users::Model,
    session: &coaching_sessions::Model,
    series_id: Id,
    organization: &organizations::Model,
    description: String,
    dtstamp: chrono::NaiveDateTime,
) -> Result<String, Error> {
    let anchor_tz = anchor_tz(coach);
    let invite = ical::IcsInvite {
        uid: ics_uid(series_id),
        // Already bumped by the caller inside the delete transaction, so the cancellation
        // outranks any edit that committed alongside it.
        sequence: session.ical_sequence,
        method: ical::Method::Cancel,
        status: ical::EventStatus::Cancelled,
        summary: session_summary(organization),
        description,
        anchor_tz,
        dtstamp,
        start: session.date,
        duration_minutes: session.duration_minutes,
        organizer: platform_organizer(),
        attendees: session_attendees(coach, coachee),
        location_url: session.meeting_url.clone(),
        recurrence: None,
        recurrence_id: session.ical_recurrence_id,
    };
    ical::build(&invite)
}

/// Send a session-cancelled notification email to a single recipient. Carries fewer
/// variables than the invite sends: no `session_url` (the row is gone) and no duration.
async fn send_session_cancelled_email_to_recipient(
    email_config: &ResolvedEmailConfig,
    to: &Recipient<'_>,
    session: &coaching_sessions::Model,
    organization: &organizations::Model,
    ics_body: Option<&str>,
) -> Result<(), Error> {
    let recipient = to.user;
    let (session_date, session_time) = format_session_date_time(session.date, &recipient.timezone);

    let email_request = SendEmailRequestBuilder::new()
        .from(FROM_ADDRESS)
        .to_with_name(
            &recipient.email,
            format!("{} {}", recipient.first_name, recipient.last_name),
        )
        .template_id(&email_config.template_id)
        .add_variable("first_name", recipient.first_name.as_str())
        .add_variable("other_user_first_name", to.other_user.first_name.as_str())
        .add_variable("other_user_last_name", to.other_user.last_name.as_str())
        .add_variable("other_user_role", to.other_user_role)
        .add_variable("organization_name", organization.name.as_str())
        .add_variable("session_date", session_date.as_str())
        .add_variable("session_time", session_time.as_str())
        .add_optional_ics_attachment(ics_body, &ical::Method::Cancel)
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

    let dtstamp = chrono::Utc::now().naive_utc();
    // A session inside a series is addressed as an override of its occurrence.
    let ics_body = build_session_ics(
        session,
        SESSION_CANCELLED_DESCRIPTION.to_string(),
        |series_id, description| {
            build_occurrence_cancel_ics(
                coach,
                coachee,
                session,
                series_id,
                organization,
                description,
                dtstamp,
            )
        },
        |description| {
            build_session_cancel_ics(coach, coachee, session, organization, description, dtstamp)
        },
    )?;

    if let Err(e) = send_session_cancelled_email_to_recipient(
        &email_config,
        &Recipient {
            user: coachee,
            other_user: coach,
            other_user_role: "coach",
        },
        session,
        organization,
        ics_body.as_deref(),
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
        &Recipient {
            user: coach,
            other_user: coachee,
            other_user_role: "coachee",
        },
        session,
        organization,
        ics_body.as_deref(),
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
async fn send_recurring_series_email_to_recipient(
    email_config: &ResolvedEmailConfig,
    to: &Recipient<'_>,
    sessions: &[coaching_sessions::Model],
    organization: &organizations::Model,
    ics_body: &str,
    recurrence_summary: &str,
    reschedule: Option<&RescheduleVars>,
) -> Result<(), Error> {
    let recipient = to.user;
    let (first, last) = series_bounds(sessions)?;

    let (first_session_date, first_session_time) =
        format_session_date_time(first.date, &recipient.timezone);
    let (last_session_date, _last_session_time) =
        format_session_date_time(last.date, &recipient.timezone);
    let session_url = email_config.build_session_url(&first.id)?;
    // All sessions in a recurring series share the same duration.
    let session_duration =
        crate::duration::Duration::from_minutes_unchecked(first.duration_minutes).to_string();

    let builder = SendEmailRequestBuilder::new()
        .from(FROM_ADDRESS)
        .to_with_name(
            &recipient.email,
            format!("{} {}", recipient.first_name, recipient.last_name),
        )
        .template_id(&email_config.template_id)
        .add_variable("first_name", recipient.first_name.as_str())
        .add_variable("other_user_first_name", to.other_user.first_name.as_str())
        .add_variable("other_user_last_name", to.other_user.last_name.as_str())
        .add_variable("other_user_role", to.other_user_role)
        .add_variable("organization_name", organization.name.as_str())
        .add_variable("session_count", sessions.len() as u64)
        .add_variable("first_session_date", first_session_date.as_str())
        .add_variable("first_session_time", first_session_time.as_str())
        .add_variable("last_session_date", last_session_date.as_str())
        .add_variable("session_duration", session_duration.as_str())
        .add_variable("session_url", session_url.as_str())
        .add_variable("recurrence_summary", recurrence_summary)
        .add_optional_variable("session_or_series", reschedule.map(|r| r.session_or_series))
        .add_ics_attachment(ics_body, &ical::Method::Request);

    // The reschedule template declares these keys, so they ship together or not at all.
    let email_request = match reschedule {
        Some(r) => builder
            .add_variable(
                "session_when",
                format_session_when(first.date, &recipient.timezone),
            )
            .add_variable(
                "previous_session_when",
                format_previous_session_when(r.previous_start, first.date, &recipient.timezone),
            )
            .add_optional_variable(
                "previous_recurrence_summary",
                r.previous_recurrence_summary.as_deref().map(|previous| {
                    format_previous_recurrence_summary(previous, recurrence_summary)
                }),
            ),
        None => builder,
    }
    .build()
    .await?;

    email_config.client.send_email(email_request).await
}

/// Build the series invite `.ics` body (VEVENT + RRULE). Pure: `dtstamp` injected.
fn build_series_invite_ics(
    coach: &users::Model,
    coachee: &users::Model,
    first_session: &coaching_sessions::Model,
    organization: &organizations::Model,
    series: &coaching_session_series::Model,
    description: String,
    dtstamp: chrono::NaiveDateTime,
) -> Result<String, Error> {
    let anchor_tz = anchor_tz(coach);
    let rule: SeriesRule = serde_json::from_value(series.rule.clone())?;
    let invite = ical::IcsInvite {
        uid: ics_uid(series.id),
        sequence: series.ical_sequence,
        method: ical::Method::Request,
        status: ical::EventStatus::Confirmed,
        summary: session_summary(organization),
        description,
        anchor_tz,
        dtstamp,
        start: first_session.date,
        duration_minutes: first_session.duration_minutes,
        organizer: platform_organizer(),
        attendees: session_attendees(coach, coachee),
        location_url: first_session.meeting_url.clone(),
        recurrence: Some(rule.recurrence),
        recurrence_id: None,
    };
    ical::build(&invite)
}

/// Send series invite emails to both coach and coachee. Generic over the notification
/// type `N` so the scheduled and reschedule flows share one body; they differ only by
/// template (via `N`) and the `session_or_series` template variable. A reschedule passes
/// an already-bumped `series`, so the `.ics` carries the next `SEQUENCE` under a stable
/// `UID`.
async fn send_series_invite_email<N: EmailNotification>(
    db: &DatabaseConnection,
    config: &Config,
    series: &coaching_session_series::Model,
    participants: &Participants<'_>,
    sessions: &[coaching_sessions::Model],
    organization: &organizations::Model,
    reschedule: Option<&RescheduleVars>,
) -> Result<(), Error> {
    let Participants { coach, coachee } = *participants;
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

    let anchor_tz = anchor_tz(coach);
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

    // Timezone-independent, so it is computed once rather than per recipient.
    let current_rule: SeriesRule = serde_json::from_value(series.rule.clone())?;
    let recurrence_summary = current_rule.recurrence.summary();

    if let Err(e) = send_recurring_series_email_to_recipient(
        &email_config,
        &Recipient {
            user: coachee,
            other_user: coach,
            other_user_role: "coach",
        },
        sessions,
        organization,
        &ics_body,
        &recurrence_summary,
        reschedule,
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
        &Recipient {
            user: coach,
            other_user: coachee,
            other_user_role: "coachee",
        },
        sessions,
        organization,
        &ics_body,
        &recurrence_summary,
        reschedule,
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
        &Participants { coach, coachee },
        sessions,
        organization,
        None,
    )
    .await
}

/// Send series reschedule notification emails to both coach and coachee. `series` must
/// already carry the bumped `ical_sequence`, so the invite updates the recurring calendar
/// event in place (same `UID`, next `SEQUENCE`). The previous start comes from
/// `previous_series`, the pre-update model, and is shown alongside the new first
/// occurrence.
async fn send_recurring_sessions_rescheduled_email(
    db: &DatabaseConnection,
    config: &Config,
    series: &coaching_session_series::Model,
    previous_series: &coaching_session_series::Model,
    participants: &Participants<'_>,
    sessions: &[coaching_sessions::Model],
    organization: &organizations::Model,
) -> Result<(), Error> {
    let previous_rule: SeriesRule = serde_json::from_value(previous_series.rule.clone())?;

    send_series_invite_email::<RecurringSessionsRescheduled>(
        db,
        config,
        series,
        participants,
        sessions,
        organization,
        Some(&RescheduleVars {
            session_or_series: "series",
            previous_start: previous_rule.start_at,
            previous_recurrence_summary: Some(previous_rule.recurrence.summary()),
        }),
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

/// Orchestrate sending series reschedule emails (best-effort).
///
/// `series` must be the post-update model, so its `ical_sequence` is already
/// incremented. Looks up the coaching relationship, both users, and the organization,
/// then re-sends the series invite to both coach and coachee so their recurring calendar
/// event updates in place. A reschedule can legitimately leave no future sessions, in
/// which case there is nothing to invite anyone to. `previous_series` is the pre-update
/// model: its rule carries the start the recipients last saw. Errors are logged
/// internally and never block or fail the calling operation.
pub async fn notify_recurring_sessions_rescheduled(
    db: &DatabaseConnection,
    config: &Config,
    series: &coaching_session_series::Model,
    previous_series: PreviousSeries<'_>,
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

        send_recurring_sessions_rescheduled_email(
            db,
            config,
            series,
            previous_series.0,
            &Participants {
                coach: &coach,
                coachee: &coachee,
            },
            sessions,
            &org,
        )
        .await
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
    coach: &users::Model,
    coachee: &users::Model,
    first_session: &coaching_sessions::Model,
    organization: &organizations::Model,
    series: &coaching_session_series::Model,
    description: String,
    dtstamp: chrono::NaiveDateTime,
) -> Result<String, Error> {
    let anchor_tz = anchor_tz(coach);
    let rule: SeriesRule = serde_json::from_value(series.rule.clone())?;
    let invite = ical::IcsInvite {
        uid: ics_uid(series.id),
        // Already bumped by the caller inside the delete transaction, so the cancellation
        // outranks any edit that committed alongside it.
        sequence: series.ical_sequence,
        method: ical::Method::Cancel,
        status: ical::EventStatus::Cancelled,
        summary: session_summary(organization),
        description,
        anchor_tz,
        dtstamp,
        start: first_session.date,
        duration_minutes: first_session.duration_minutes,
        organizer: platform_organizer(),
        attendees: session_attendees(coach, coachee),
        location_url: first_session.meeting_url.clone(),
        recurrence: Some(rule.recurrence),
        recurrence_id: None,
    };
    ical::build(&invite)
}

/// Send a series cancellation notification email to a single recipient. Carries fewer
/// variables than the invite sends: no `session_url`, no duration, no first-session time.
async fn send_recurring_sessions_cancelled_email_to_recipient(
    email_config: &ResolvedEmailConfig,
    recipient: &users::Model,
    other_user: &users::Model,
    other_user_role: &str,
    sessions: &[coaching_sessions::Model],
    organization: &organizations::Model,
    ics_body: &str,
) -> Result<(), Error> {
    let (first, last) = series_bounds(sessions)?;

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

/// Send series cancellation emails to both coach and coachee. Takes no database handle:
/// a cancellation loads no topics, goals, or actions.
async fn send_recurring_sessions_cancelled_email(
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

    let email_config = ResolvedEmailConfig::new::<RecurringSessionsCancelled>(config).await?;

    let (first, _) = series_bounds(sessions)?;

    let ics_body = build_series_cancel_ics(
        coach,
        coachee,
        first,
        organization,
        series,
        SERIES_CANCELLED_DESCRIPTION.to_string(),
        chrono::Utc::now().naive_utc(),
    )?;

    if let Err(e) = send_recurring_sessions_cancelled_email_to_recipient(
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

    if let Err(e) = send_recurring_sessions_cancelled_email_to_recipient(
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

/// Orchestrate sending series cancellation emails (best-effort).
///
/// Call this with the in-memory models after the delete has committed. A series row is
/// deleted even when nothing upcoming remains, so an empty `sessions` slice returns
/// before any lookup rather than announcing zero cancelled sessions. Errors are logged
/// internally and never block the caller.
pub async fn notify_recurring_sessions_cancelled(
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

        send_recurring_sessions_cancelled_email(config, series, &coach, &coachee, sessions, &org)
            .await
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
