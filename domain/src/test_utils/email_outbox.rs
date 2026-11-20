//! Rendered emails for outbox tests.

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

/// The outbox's one row; fails when there is not exactly one.
pub(crate) async fn only_row(db: &impl ConnectionTrait) -> Model {
    let mut all = rows(db).await;
    assert_eq!(all.len(), 1, "{all:?}");
    all.remove(0)
}
