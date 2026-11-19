//! Transcript suggestions: data access, the model's input, and verification of its answer.

use std::borrow::Cow;

use chrono::{NaiveDate, Weekday};
use chrono_tz::Tz;
use log::{debug, warn};
use sea_orm::prelude::DateTimeWithTimeZone;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::coaching_relationship_kind::Kind;
use crate::error::{DomainErrorKind, Error};
use crate::transcript_export::LabeledSegment;
use crate::Id;

pub use crate::transcript_suggestion_target::Target;
pub use actions::{ActionProposal, Actions};
pub use entity::transcript_suggestion::{ActiveModel, Model, Status};
pub use entity_api::transcript_suggestion::{
    create_batch, find_by_id, find_by_session, transition,
};
pub use prompt::{distinct_names, render, Refs};

pub mod actions;
pub(crate) mod due_date;
pub mod extract;
pub mod prompt;
pub mod review;

/// The most verified suggestions kept from one answer; a backstop against a runaway answer.
pub const MAX_SUGGESTIONS: usize = 50;

/// One action in the session that the model may flag as a duplicate.
pub struct Existing {
    pub id: Id,
    pub text: String,
}

/// The session's coach and coachee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct People {
    pub coach_id: Id,
    pub coachee_id: Id,
}

/// Everything the prompt and the verification read about one session.
pub struct Session<'a> {
    pub coaching_session_id: Id,
    pub people: People,
    pub coach_name: String,
    pub coachee_name: String,
    /// The schedule owner's IANA timezone; the calendar is read there.
    pub anchor_timezone: Tz,
    /// Each participant's IANA timezone.
    pub timezones: Timezones,
    /// The relationship's kind, whose rules decide deadline zones and how the prompt names people.
    pub kind: Kind,
    /// When the session starts.
    pub start: DateTimeWithTimeZone,
    /// The session's calendar date in the anchor timezone.
    pub session_date: NaiveDate,
    /// The next session in the relationship, if one is scheduled.
    pub next_session: Option<NextSession>,
    /// The day the schedule owner's week ends on; "end of the week" and "this week" mean it.
    pub week_end: Weekday,
    /// Ordered by (start_ms, id); refs are assigned in this order.
    pub segments: &'a [LabeledSegment],
    pub existing: Vec<Existing>,
}

impl Session<'_> {
    /// The timezone a deadline for `assignee_ids` falls in: a lone assignee's own when the
    /// relationship says so, otherwise the anchor timezone.
    pub fn due_timezone(&self, assignee_ids: &[Id]) -> Tz {
        let [only] = assignee_ids else {
            return self.anchor_timezone;
        };
        self.kind
            .rules()
            .due_in_assignee_zone()
            .then_some(*only)
            .and_then(|id| self.timezone_of(id))
            .unwrap_or(self.anchor_timezone)
    }

    /// The session's calendar date in `tz`.
    pub fn date_in(&self, tz: Tz) -> NaiveDate {
        self.start.with_timezone(&tz).date_naive()
    }

    /// What the model calls the coach and coachee seats, in that order: the two people's names
    /// when the prompt names them, otherwise `coach` and `coachee`.
    pub fn seat_names(&self) -> [&str; 2] {
        if self.kind.rules().names_people_in_prompt() {
            [&self.coach_name, &self.coachee_name]
        } else {
            ["coach", "coachee"]
        }
    }

    /// The id of the seat the model calls `name`.
    pub fn seat_id(&self, name: &str) -> Option<Id> {
        self.seats()
            .into_iter()
            .find_map(|(id, seat)| (seat == name).then_some(id))
    }

    /// What the model calls the seat of the person with `id`.
    pub fn seat_name(&self, id: Id) -> Option<&str> {
        self.seats()
            .into_iter()
            .find_map(|(person, seat)| (person == id).then_some(seat))
    }

    fn seats(&self) -> [(Id, &str); 2] {
        let [coach, coachee] = self.seat_names();
        [
            (self.people.coach_id, coach),
            (self.people.coachee_id, coachee),
        ]
    }

    fn timezone_of(&self, id: Id) -> Option<Tz> {
        [
            (self.people.coach_id, self.timezones.coach),
            (self.people.coachee_id, self.timezones.coachee),
        ]
        .into_iter()
        .find_map(|(person, tz)| (person == id).then_some(tz))
    }
}

/// The IANA timezones of the session's coach and coachee seats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timezones {
    pub coach: Tz,
    pub coachee: Tz,
}

/// The next session in a coaching relationship.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NextSession {
    /// Its calendar date in the anchor timezone.
    pub date: NaiveDate,
    pub start: DateTimeWithTimeZone,
}

/// A proposal that passed verification, with where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Verified<P> {
    pub proposal: P,
    pub segment_id: Id,
    pub quote: String,
    pub duplicate_of_id: Option<Id>,
}

/// An entity type the model can suggest from a transcript.
pub trait Suggestible {
    /// The entity type this suggests.
    const TARGET: Target;
    /// One item as the model returns it.
    type Candidate: DeserializeOwned;
    /// What accept will create: the real entity's create input plus reviewer-only hints.
    type Proposal: Serialize + DeserializeOwned;

    /// The system prompt for `session`: what to extract and how to write each item.
    fn instructions(session: &Session) -> Cow<'static, str>;
    /// Name of the structured-output schema sent with the request.
    fn schema_name() -> &'static str;
    /// JSON schema of the whole model answer for `session`: `{"suggestions": [Candidate, ...]}`.
    fn output_schema(session: &Session) -> serde_json::Value;
    /// Model output to a stored proposal, or `None` when it fails a check.
    fn verify(
        candidate: Self::Candidate,
        session: &Session,
        refs: &Refs,
    ) -> Option<Verified<Self::Proposal>>;
    /// Who the proposal is for.
    fn assignees(proposal: &Self::Proposal) -> &[Id];
    /// Rejects an edited proposal that names someone outside the session or is otherwise invalid.
    fn check(proposal: &Self::Proposal, people: People) -> Result<(), Error>;
}

/// Parses the model's whole answer and verifies each item, keeping at most `MAX_SUGGESTIONS`.
pub fn verify_output<S: Suggestible>(
    output: &serde_json::Value,
    session: &Session,
    refs: &Refs,
) -> Result<Vec<Verified<S::Proposal>>, Error> {
    let items = output
        .get("suggestions")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| validation("Model output has no suggestions array."))?;

    let mut verified: Vec<_> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let verified = serde_json::from_value::<S::Candidate>(item.clone())
                .ok()
                .and_then(|candidate| S::verify(candidate, session, refs));
            if verified.is_none() {
                debug!("Dropped {} suggestion at index {index}", S::TARGET);
            }
            verified
        })
        .collect();
    if verified.len() > MAX_SUGGESTIONS {
        warn!(
            "Kept {MAX_SUGGESTIONS} of {} verified {} suggestions for session {}",
            verified.len(),
            S::TARGET,
            session.coaching_session_id
        );
        verified.truncate(MAX_SUGGESTIONS);
    }
    Ok(verified)
}

/// A validation error (422 at the edge) carrying `message`.
pub(crate) fn validation(message: &str) -> Error {
    Error {
        source: None,
        error_kind: DomainErrorKind::Validation(message.to_string()),
    }
}

#[cfg(test)]
mod sqlite_tests;

#[cfg(test)]
mod eval;
