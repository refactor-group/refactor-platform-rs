//! Transcript suggestions: data access, the model's input, and verification of its answer.

use chrono::NaiveDate;
use chrono_tz::Tz;
use log::{debug, warn};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{DomainErrorKind, Error};
use crate::transcript_export::LabeledSegment;
use crate::Id;

pub use crate::transcript_suggestion_target::Target;
pub use actions::{ActionProposal, Actions};
pub use entity::transcript_suggestion::{ActiveModel, Model, Status};
pub use entity_api::transcript_suggestion::{
    create_batch, find_by_id, find_by_session, find_pending_by_session, transition,
};
pub use prompt::{render, Refs};

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
    /// The coach's IANA timezone; due dates are anchored there.
    pub coach_timezone: Tz,
    /// The session's calendar date in the coach's timezone.
    pub session_date: NaiveDate,
    /// The next session's calendar date in the coach's timezone, if one is scheduled.
    pub next_session: Option<NaiveDate>,
    /// Ordered by (start_ms, id); refs are assigned in this order.
    pub segments: &'a [LabeledSegment],
    pub existing: Vec<Existing>,
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

    fn instructions() -> &'static str;
    fn schema_name() -> &'static str;
    /// JSON schema of the whole model answer: `{"suggestions": [Candidate, ...]}`.
    fn output_schema() -> serde_json::Value;
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

pub(crate) fn validation(message: &str) -> Error {
    Error {
        source: None,
        error_kind: DomainErrorKind::Validation(message.to_string()),
    }
}

#[cfg(test)]
mod sqlite_tests;
