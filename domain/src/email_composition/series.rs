//! Emails about a recurring series: scheduled, rescheduled and cancelled.

use crate::email_outbox::Attachment;

use super::{templates, Actor, Email, Parties, Person, When};

/// A recurring series was scheduled.
#[derive(Clone, PartialEq, Eq)]
pub struct SeriesScheduled {
    pub parties: Parties,
    pub by: Actor,
    /// The first session's start.
    pub first: When,
    pub last_date: String,
    pub duration: String,
    /// How often it repeats, for example `Every week on Tuesday`.
    pub cadence: String,
    pub count: usize,
    /// The first session's page.
    pub url: String,
    /// The series invite.
    pub calendar: Vec<Attachment>,
}

templates!(
    SeriesScheduled,
    "email/series_scheduled.html",
    "email/series_scheduled.txt"
);

impl Email for SeriesScheduled {
    const KIND: &'static str = "series_scheduled";

    fn recipient(&self) -> &Person {
        &self.parties.recipient
    }

    fn subject(&self) -> String {
        format!(
            "Your {} series with {} starts {}",
            self.parties.session_label, self.parties.other.first_name, self.first.date
        )
    }

    fn attachments(&self) -> Vec<Attachment> {
        self.calendar.clone()
    }
}

/// Which sessions of a series a reschedule moved, with what they were before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Every upcoming session.
    WholeSeries {
        /// The previous first start, when it moved.
        previous_start: Option<When>,
        /// The previous cadence, when it changed.
        previous_cadence: Option<String>,
    },
    /// One session and every one after it.
    FromSession {
        first_moved_date: String,
        previous_cadence: String,
        previous_time: String,
    },
}

/// A recurring series, or its tail from one session on, moved.
#[derive(Clone, PartialEq, Eq)]
pub struct SeriesRescheduled {
    pub parties: Parties,
    pub by: Actor,
    pub scope: Scope,
    /// The first moved session's new start.
    pub starts: When,
    pub cadence: String,
    pub duration: String,
    pub last_date: String,
    /// How many sessions it now holds, or for [`Scope::FromSession`], how many moved.
    pub count: usize,
    /// The first moved session's page.
    pub url: String,
    /// The updated invites and any cancellations riding along.
    pub calendar: Vec<Attachment>,
}

templates!(
    SeriesRescheduled,
    "email/series_rescheduled.html",
    "email/series_rescheduled.txt"
);

impl Email for SeriesRescheduled {
    const KIND: &'static str = "series_rescheduled";

    fn recipient(&self) -> &Person {
        &self.parties.recipient
    }

    fn subject(&self) -> String {
        let Parties {
            other,
            session_label_plural,
            ..
        } = &self.parties;
        match &self.scope {
            Scope::WholeSeries { .. } => format!(
                "Your series of {session_label_plural} with {} has been rescheduled",
                other.first_name
            ),
            Scope::FromSession {
                first_moved_date, ..
            } => format!(
                "Your {session_label_plural} with {} change from {first_moved_date}",
                other.first_name
            ),
        }
    }

    fn attachments(&self) -> Vec<Attachment> {
        self.calendar.clone()
    }
}

/// A recurring series was cancelled.
#[derive(Clone, PartialEq, Eq)]
pub struct SeriesCancelled {
    pub parties: Parties,
    pub by: Actor,
    /// How many upcoming sessions it removed.
    pub count: usize,
    pub first_date: String,
    pub last_date: String,
    /// The series cancellation and any cancellations riding along.
    pub calendar: Vec<Attachment>,
}

templates!(
    SeriesCancelled,
    "email/series_cancelled.html",
    "email/series_cancelled.txt"
);

impl Email for SeriesCancelled {
    const KIND: &'static str = "series_cancelled";

    fn recipient(&self) -> &Person {
        &self.parties.recipient
    }

    fn subject(&self) -> String {
        format!(
            "Your series of {} with {} has been cancelled",
            self.parties.session_label_plural, self.parties.other.first_name
        )
    }

    fn attachments(&self) -> Vec<Attachment> {
        self.calendar.clone()
    }
}

#[cfg(test)]
#[path = "series_tests.rs"]
mod tests;
