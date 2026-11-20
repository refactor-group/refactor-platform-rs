//! Rendered emails for outbox tests.

use crate::email_outbox::{Mailbox, OutgoingEmail};

/// A rendered email to one recipient, told apart from others by `subject`.
pub(crate) fn outgoing_email(subject: &str) -> OutgoingEmail {
    let mailbox = |email: &str| Mailbox {
        email: email.to_string(),
        name: None,
    };
    OutgoingEmail {
        from: mailbox("sender@example.com"),
        to: vec![mailbox("recipient@example.com")],
        reply_to: Some(mailbox("hello@example.com")),
        subject: subject.to_string(),
        html: format!("<p>{subject}</p>"),
        text: subject.to_string(),
        attachments: Vec::new(),
    }
}
