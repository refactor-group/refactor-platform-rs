//! Rendered emails for outbox tests.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use entity_api::email_outbox::Model;
use sea_orm::{ConnectionTrait, EntityTrait, QueryOrder};

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

/// Every outbox row, oldest first.
pub(crate) async fn rows(db: &impl ConnectionTrait) -> Vec<Model> {
    entity::email_outbox::Entity::find()
        .order_by_asc(entity::email_outbox::Column::CreatedAt)
        .order_by_asc(entity::email_outbox::Column::Position)
        .all(db)
        .await
        .expect("the rows read")
}

/// Every outbox row, oldest first, with the email it stores.
pub(crate) async fn emails(db: &impl ConnectionTrait) -> Vec<(Model, OutgoingEmail)> {
    rows(db)
        .await
        .into_iter()
        .map(|row| {
            let email = serde_json::from_value(row.request.clone().unwrap_or_default())
                .expect("the stored email reads");
            (row, email)
        })
        .collect()
}

/// The only calendar part of `email`, decoded and unfolded.
pub(crate) fn calendar_part(email: &OutgoingEmail) -> String {
    let [part] = calendar_parts(email)
        .try_into()
        .unwrap_or_else(|parts: Vec<String>| {
            panic!("one calendar part, not {}: {email:?}", parts.len())
        });
    part
}

/// Every calendar part of `email` in order, decoded and unfolded.
pub(crate) fn calendar_parts(email: &OutgoingEmail) -> Vec<String> {
    email
        .attachments
        .iter()
        .map(|attachment| {
            String::from_utf8(
                STANDARD
                    .decode(&attachment.content)
                    .expect("the part is base64"),
            )
            .expect("the part is UTF-8")
            // RFC 5545 folds long lines with a space or a tab after the break.
            .replace("\r\n ", "")
            .replace("\r\n\t", "")
        })
        .collect()
}

/// The outbox's one row; fails when there is not exactly one.
pub(crate) async fn only_row(db: &impl ConnectionTrait) -> Model {
    let mut all = rows(db).await;
    assert_eq!(all.len(), 1, "{all:?}");
    all.remove(0)
}
