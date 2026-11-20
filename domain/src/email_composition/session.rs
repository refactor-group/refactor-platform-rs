//! Emails about one session: scheduled, rescheduled, cancelled and the upcoming reminder.

use crate::email_outbox::Attachment;

use super::{templates, Actor, Email, Parties, Person, When};

/// A session was scheduled.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionScheduled {
    pub parties: Parties,
    pub by: Actor,
    pub when: When,
    pub duration: String,
    pub url: String,
    /// The calendar invite.
    pub calendar: Vec<Attachment>,
}

templates!(
    SessionScheduled,
    "email/session_scheduled.html",
    "email/session_scheduled.txt"
);

impl Email for SessionScheduled {
    const KIND: &'static str = "session_scheduled";

    fn recipient(&self) -> &Person {
        &self.parties.recipient
    }

    fn subject(&self) -> String {
        format!(
            "Your {} with {} on {}",
            self.parties.session_label, self.parties.other.full_name, self.when.date
        )
    }

    fn attachments(&self) -> Vec<Attachment> {
        self.calendar.clone()
    }
}

/// A session moved or changed length. Neutral about who did it: edits are batched.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionRescheduled {
    pub parties: Parties,
    pub when: When,
    /// The start the recipient last heard of, when the start moved.
    pub previous: Option<When>,
    pub duration: String,
    pub url: String,
    /// The calendar invite that updates the event in place.
    pub calendar: Vec<Attachment>,
}

templates!(
    SessionRescheduled,
    "email/session_rescheduled.html",
    "email/session_rescheduled.txt"
);

impl Email for SessionRescheduled {
    const KIND: &'static str = "session_rescheduled";

    fn recipient(&self) -> &Person {
        &self.parties.recipient
    }

    fn subject(&self) -> String {
        format!(
            "Your {} with {} has been rescheduled",
            self.parties.session_label, self.parties.other.full_name
        )
    }

    fn attachments(&self) -> Vec<Attachment> {
        self.calendar.clone()
    }
}

/// Why a session's calendar event is being removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelReason {
    /// A participant cancelled the session.
    Cancelled { by: Actor },
    /// A series edit replaced the standalone event a session had from before it joined the series.
    Superseded,
}

/// A session's calendar event is removed.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionCancelled {
    pub parties: Parties,
    /// When it was scheduled for.
    pub when: When,
    pub reason: CancelReason,
    /// The calendar cancellation.
    pub calendar: Vec<Attachment>,
}

templates!(
    SessionCancelled,
    "email/session_cancelled.html",
    "email/session_cancelled.txt"
);

impl Email for SessionCancelled {
    const KIND: &'static str = "session_cancelled";

    fn recipient(&self) -> &Person {
        &self.parties.recipient
    }

    fn subject(&self) -> String {
        let label = self.parties.session_label;
        match self.reason {
            CancelReason::Cancelled { .. } => format!(
                "Your {label} with {} has been cancelled",
                self.parties.other.full_name
            ),
            CancelReason::Superseded => format!(
                "An old calendar event for your {label} on {} was removed",
                self.when.date
            ),
        }
    }

    fn attachments(&self) -> Vec<Attachment> {
        self.calendar.clone()
    }
}

/// A session starts soon. Carries no calendar part: the event is already on the calendar.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionReminder {
    pub parties: Parties,
    pub title: Option<String>,
    pub when: When,
    pub duration: String,
    pub url: String,
}

templates!(
    SessionReminder,
    "email/session_reminder.html",
    "email/session_reminder.txt"
);

impl Email for SessionReminder {
    const KIND: &'static str = "session_reminder";

    fn recipient(&self) -> &Person {
        &self.parties.recipient
    }

    fn subject(&self) -> String {
        format!(
            "Your {} with {} is coming up",
            self.parties.session_label, self.parties.other.full_name
        )
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
