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
            "Session with {} on {} at {}",
            self.parties.other.full_name, self.when.day, self.when.time
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
    /// The start the recipient last heard of, when the start moved. `None` means only the
    /// length changed.
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
        let (label, other) = (self.parties.session_label, &self.parties.other.full_name);
        match self.previous {
            Some(_) => format!(
                "Your {label} with {other} was rescheduled to {}",
                self.when.day
            ),
            None => format!(
                "Your {label} with {other} on {} now runs {}",
                self.when.day, self.duration
            ),
        }
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
        let (label, other, day) = (
            self.parties.session_label,
            &self.parties.other.full_name,
            &self.when.day,
        );
        match self.reason {
            CancelReason::Cancelled { by: Actor::Other } => {
                format!("{other} cancelled your {label} on {day}")
            }
            CancelReason::Cancelled { by: Actor::You } => {
                format!("You cancelled your {label} with {other}")
            }
            CancelReason::Superseded => format!("Old calendar event removed for {day}"),
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
    /// Where they meet, when the session has a meeting link.
    pub meeting_url: Option<String>,
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
