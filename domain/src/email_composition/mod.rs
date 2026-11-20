//! Emails rendered from in-repo templates on one shared layout, ready for the outbox.
//!
//! Each email kind is a struct of already-formatted, recipient-specific facts that
//! implements [`Email`]; its HTML and plain-text bodies are askama templates under
//! `domain/templates/email/`, wired by [`templates!`]. [`compose`] renders both through the
//! layout, inlines the stylesheet and addresses the result from [`Brand`]. See
//! `docs/architecture/email_composition.md`.

use std::sync::LazyLock;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use css_inline::CSSInliner;
use service::config::Config;

use crate::email_outbox::{Attachment, Mailbox, OutgoingEmail};
use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use crate::gateway::ical::Method;
use crate::{users, Id};

pub mod series;
pub mod session;

pub use series::{SeriesCancelled, SeriesRescheduled, SeriesScheduled};
pub use session::{SessionCancelled, SessionReminder, SessionRescheduled, SessionScheduled};

/// The `From:` address of every email. Kept on the `mail.` subdomain so the apex domain's
/// DMARC/SPF/DKIM records are unaffected by the provider's sending infrastructure.
pub(crate) const FROM_ADDRESS: &str = "hello@mail.myrefactor.com";

/// The name shown beside [`FROM_ADDRESS`].
pub(crate) const FROM_DISPLAY_NAME: &str = "Refactor Coach";

/// Where replies and support questions go.
const SUPPORT_ADDRESS: &str = "hello@refactorcoach.com";

/// The company behind the product, linked from the footer.
const COMPANY_URL: &str = "https://www.refactorgroup.com";

/// The logo, a public asset of the frontend.
const LOGO_PATH: &str = "/22JHC_Monogram_RGB_small.png";

/// The frontend's privacy policy page.
const PRIVACY_PATH: &str = "/privacy";

/// What every email shares: who it is from, where replies go, and the layout's links.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Brand {
    pub from: Mailbox,
    pub reply_to: Mailbox,
    pub support_email: &'static str,
    pub logo_url: String,
    pub company_url: &'static str,
    pub privacy_url: String,
}

impl Brand {
    /// The brand whose links point at the frontend served from `frontend_base_url`.
    pub fn new(frontend_base_url: &str) -> Self {
        let base = frontend_base_url.trim_end_matches('/');
        Self {
            from: Mailbox {
                email: FROM_ADDRESS.to_string(),
                name: Some(FROM_DISPLAY_NAME.to_string()),
            },
            reply_to: Mailbox {
                email: SUPPORT_ADDRESS.to_string(),
                name: None,
            },
            support_email: SUPPORT_ADDRESS,
            logo_url: format!("{base}{LOGO_PATH}"),
            company_url: COMPANY_URL,
            privacy_url: format!("{base}{PRIVACY_PATH}"),
        }
    }

    /// The brand for `config`'s frontend, or a config error when it names none.
    pub fn from_config(config: &Config) -> Result<Self, Error> {
        config
            .frontend_base_url()
            .map(|base| Self::new(&base))
            .ok_or_else(|| Error {
                source: None,
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
            })
    }
}

/// Someone an email names, as their account spells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    /// What a greeting calls them: their first name, else the next name they have.
    pub first_name: String,
    /// Their first and last name, else their email.
    pub full_name: String,
    pub email: String,
}

impl Person {
    /// This person as an addressee.
    fn mailbox(&self) -> Mailbox {
        Mailbox {
            email: self.email.clone(),
            name: Some(self.full_name.clone()),
        }
    }
}

impl From<&users::Model> for Person {
    fn from(user: &users::Model) -> Self {
        Self {
            first_name: user.short_name().to_string(),
            full_name: user.full_name().into_owned(),
            email: user.email.clone(),
        }
    }
}

/// A start, formatted in the recipient's zone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct When {
    /// For example `Wednesday, March 4, 2026`.
    pub date: String,
    /// For example `10:00 AM`.
    pub time: String,
}

impl From<(String, String)> for When {
    fn from((date, time): (String, String)) -> Self {
        Self { date, time }
    }
}

/// Who an email is between and what their sessions are called.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parties {
    pub recipient: Person,
    /// The session's other participant.
    pub other: Person,
    pub session_label: &'static str,
    pub session_label_plural: &'static str,
    pub organization_name: String,
}

/// Who made the change an email announces, as its recipient reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    /// The recipient made it.
    You,
    /// The other participant made it.
    Other,
}

impl Actor {
    /// How a recipient `recipient_id` sees a change `actor_id` made.
    pub fn of(actor_id: Id, recipient_id: Id) -> Self {
        match actor_id == recipient_id {
            true => Actor::You,
            false => Actor::Other,
        }
    }
}

/// The two rendered bodies of an email, generated by [`templates!`].
pub trait Bodies {
    fn html(&self, brand: &Brand) -> askama::Result<String>;
    fn text(&self, brand: &Brand) -> askama::Result<String>;
}

/// One kind of email: what it says and to whom.
pub trait Email: Bodies {
    /// Stable name, for logs and the outbox `kind`.
    const KIND: &'static str;

    /// Who receives it; the `to:` address.
    fn recipient(&self) -> &Person;

    fn subject(&self) -> String;

    /// Files sent with it, such as a calendar invite.
    fn attachments(&self) -> Vec<Attachment> {
        Vec::new()
    }
}

/// Renders `email` through the layout, inlines its stylesheet and addresses it from `brand`.
pub fn compose<E: Email>(email: &E, brand: &Brand) -> Result<OutgoingEmail, Error> {
    Ok(OutgoingEmail {
        from: brand.from.clone(),
        to: vec![email.recipient().mailbox()],
        reply_to: Some(brand.reply_to.clone()),
        subject: email.subject(),
        html: INLINER.inline(&email.html(brand)?)?,
        text: email.text(brand)?,
        attachments: email.attachments(),
    })
}

/// Inlines the layout's stylesheet and keeps its `@media` rules, which cannot be inlined,
/// and its web font `<link>`, which it never fetches.
static INLINER: LazyLock<CSSInliner<'static>> = LazyLock::new(|| {
    CSSInliner::options()
        .keep_at_rules(true)
        .keep_link_tags(true)
        .load_remote_stylesheets(false)
        .build()
});

/// Calendar parts as attachments, numbered from the second on so clients keep them apart.
pub fn calendar_attachments<'a>(
    parts: impl IntoIterator<Item = (&'a str, &'a Method)>,
) -> Vec<Attachment> {
    parts
        .into_iter()
        .enumerate()
        .map(|(position, (body, method))| calendar_attachment(position, body, method))
        .collect()
}

/// The calendar part at `position` among an email's calendar parts, counting from zero.
pub fn calendar_attachment(position: usize, body: &str, method: &Method) -> Attachment {
    Attachment {
        filename: match position {
            0 => "invite.ics".to_string(),
            n => format!("invite-{}.ics", n + 1),
        },
        content_type: format!("text/calendar; method={}; charset=UTF-8", method.as_str()),
        content: STANDARD.encode(body.as_bytes()),
    }
}

/// Emits `$email`'s HTML and text templates and its [`Bodies`] impl. The derive needs
/// literal paths, so each email names its pair.
macro_rules! templates {
    ($email:ident, $html:literal, $text:literal) => {
        const _: () = {
            #[derive(askama::Template)]
            #[template(path = $html)]
            struct Html<'a> {
                email: &'a $email,
                brand: &'a $crate::email_composition::Brand,
            }

            #[derive(askama::Template)]
            #[template(path = $text)]
            struct Text<'a> {
                email: &'a $email,
                brand: &'a $crate::email_composition::Brand,
            }

            impl $crate::email_composition::Bodies for $email {
                fn html(&self, brand: &$crate::email_composition::Brand) -> askama::Result<String> {
                    askama::Template::render(&Html { email: self, brand })
                }

                fn text(&self, brand: &$crate::email_composition::Brand) -> askama::Result<String> {
                    askama::Template::render(&Text { email: self, brand })
                }
            }
        };
    };
}
pub(crate) use templates;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
