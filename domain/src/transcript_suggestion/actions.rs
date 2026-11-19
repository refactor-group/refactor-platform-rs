//! Actions as a suggestion target: the model's candidate, its verification and the stored proposal.

use std::borrow::Cow;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use sea_orm::prelude::DateTimeWithTimeZone;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use super::due_date::{names_next_session, Anchors};
use super::prompt::collapse;
use super::{validation, People, Refs, Session, Suggestible, Target, Verified};
use crate::action::ActionWithAssignees;
use crate::actions::Model;
use crate::error::Error;
use crate::status::Status;
use crate::Id;

/// Longest action body accepted.
const MAX_BODY_CHARS: usize = 2000;
/// Longest due phrase kept.
const MAX_PHRASE_CHARS: usize = 200;
/// Longest segment text used as a fallback quote.
const MAX_QUOTE_CHARS: usize = 200;
/// The coaching system prompt.
const PROMPT: &str = include_str!("actions_prompt.md");
/// A colleague passage, given the two people's names in seat order.
type ColleaguePassage = fn(&str, &str) -> String;
/// The colleague prompt's changes to the coaching prompt: each coaching passage and its colleague
/// passage.
pub(super) const COLLEAGUE_PASSAGES: [(&str, ColleaguePassage); 6] = [
    (
        "You read the transcript of a coaching session between a coach and a coachee and list the actions that the\ncoach or the coachee committed to doing after the session.",
        |a, b| format!("You read the transcript of a session between two colleagues, {a} and {b}, and list the actions that\neither of them committed to doing after the session."),
    ),
    (
        "- The coach or coachee says they will do something after the session:",
        |a, b| format!("- {a} or {b} says they will do something after the session:"),
    ),
    (
        "- Agreements about how the coach and coachee work together",
        |_, _| "- Agreements about how the two of them work together".to_string(),
    ),
    (
        "- Scheduling the next coaching session, and anything about the recording or the meeting tool.",
        |_, _| "- Scheduling the next session or meeting, and anything about the recording or the meeting tool.".to_string(),
    ),
    (
        "- Commitments by anyone other than the coach and coachee (other attendees, managers, colleagues).",
        |a, b| format!("- Commitments by anyone other than {a} and {b} (other attendees, managers, other colleagues)."),
    ),
    (
        "- assignees: [\"coachee\"], [\"coach\"], or [\"coach\", \"coachee\"] when they both committed to the same thing.",
        |a, b| format!("- assignees: [\"{a}\"], [\"{b}\"], or [\"{a}\", \"{b}\"] when they both committed to the same thing."),
    ),
];

/// Suggested actions.
pub struct Actions;

/// One action as the model returns it.
#[derive(Debug, Deserialize)]
pub struct Candidate {
    pub body: String,
    /// What the model calls each owner; see [`Session::seat_names`].
    pub assignees: Vec<String>,
    pub due_date: Option<String>,
    pub due_phrase: Option<String>,
    pub source_segment: String,
    pub quote: String,
    pub duplicate_of: Option<String>,
}

/// The action accept will create, plus the spoken deadline for the reviewer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[schema(as = domain::transcript_suggestion::actions::ActionProposal)]
pub struct ActionProposal {
    #[serde(flatten)]
    pub action: ActionWithAssignees,
    pub due_phrase: Option<String>,
}

impl Suggestible for Actions {
    const TARGET: Target = Target::Action;
    type Candidate = Candidate;
    type Proposal = ActionProposal;

    fn instructions(session: &Session) -> Cow<'static, str> {
        if !session.kind.rules().names_people_in_prompt() {
            return Cow::Borrowed(PROMPT);
        }
        let [a, b] = session.seat_names();
        Cow::Owned(
            COLLEAGUE_PASSAGES
                .iter()
                .fold(PROMPT.to_string(), |prompt, (coaching, colleague)| {
                    prompt.replacen(coaching, &colleague(a, b), 1)
                }),
        )
    }

    fn schema_name() -> &'static str {
        "action_suggestions"
    }

    fn output_schema(session: &Session) -> serde_json::Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["suggestions"],
            "properties": {
                "suggestions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": [
                            "body",
                            "assignees",
                            "due_date",
                            "due_phrase",
                            "source_segment",
                            "quote",
                            "duplicate_of"
                        ],
                        "properties": {
                            "body": { "type": "string" },
                            "assignees": {
                                "type": "array",
                                "items": { "type": "string", "enum": session.seat_names() }
                            },
                            "due_date": { "type": ["string", "null"] },
                            "due_phrase": { "type": ["string", "null"] },
                            "source_segment": { "type": "string" },
                            "quote": { "type": "string" },
                            "duplicate_of": { "type": ["string", "null"] }
                        }
                    }
                }
            }
        })
    }

    fn verify(
        candidate: Candidate,
        session: &Session,
        refs: &Refs,
    ) -> Option<Verified<ActionProposal>> {
        let body = Some(candidate.body.trim())
            .filter(|b| valid_body(b))?
            .to_string();
        let segment_id = refs.segment(&candidate.source_segment)?;
        let segment_text = refs.segment_text(session, segment_id)?;
        let quote = quote(&candidate.quote, segment_text);

        let assignee_ids = candidate
            .assignees
            .iter()
            .map(|name| session.seat_id(name))
            .collect::<Option<Vec<_>>>()?
            .into_iter()
            .fold(Vec::new(), |mut ids, id| {
                if !ids.contains(&id) {
                    ids.push(id);
                }
                ids
            });

        let due_phrase = candidate
            .due_phrase
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty() && p.chars().count() <= MAX_PHRASE_CHARS)
            .map(str::to_string);
        let model_date = candidate
            .due_date
            .as_deref()
            .and_then(|d| NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d").ok());
        let at_next_session = session
            .next_session
            .filter(|_| due_phrase.as_deref().is_some_and(names_next_session));
        let anchors = Anchors {
            session_date: session.session_date,
            next_session: session.next_session.map(|next| next.date),
            week_end: session.week_end,
        };
        let due_by = anchors
            .reconcile(model_date, due_phrase.as_deref())
            .and_then(|date| match at_next_session {
                Some(next) => Some(next.start),
                None => due_at(date, session, session.due_timezone(&assignee_ids)),
            });

        let duplicate_of_id = candidate
            .duplicate_of
            .as_deref()
            .and_then(|r| refs.existing(r));
        let placeholder = DateTime::<Utc>::UNIX_EPOCH.fixed_offset();

        Some(Verified {
            proposal: ActionProposal {
                action: ActionWithAssignees {
                    action: Model {
                        id: Id::nil(),
                        coaching_session_id: session.coaching_session_id,
                        goal_id: None,
                        user_id: Id::nil(),
                        body: Some(body),
                        due_by,
                        status: Status::NotStarted,
                        status_changed_at: placeholder,
                        created_at: placeholder,
                        updated_at: placeholder,
                    },
                    assignee_ids,
                },
                due_phrase,
            },
            segment_id,
            quote,
            duplicate_of_id,
        })
    }

    fn assignees(proposal: &ActionProposal) -> &[Id] {
        &proposal.action.assignee_ids
    }

    fn check(proposal: &ActionProposal, people: People) -> Result<(), Error> {
        let outsider = proposal
            .action
            .assignee_ids
            .iter()
            .any(|id| *id != people.coach_id && *id != people.coachee_id);
        if outsider {
            return Err(validation(
                "Assignees must be the session's coach or coachee.",
            ));
        }
        proposal
            .action
            .action
            .body
            .as_deref()
            .filter(|b| valid_body(b.trim()))
            .map(|_| ())
            .ok_or_else(|| validation("Action body must be 1 to 2000 characters."))
    }
}

fn valid_body(body: &str) -> bool {
    !body.is_empty() && body.chars().count() <= MAX_BODY_CHARS
}

/// The model's quote when it appears in the segment, otherwise the segment's own text.
fn quote(candidate: &str, segment_text: &str) -> String {
    let wanted = collapse(candidate).to_lowercase();
    if !wanted.is_empty() && collapse(segment_text).to_lowercase().contains(&wanted) {
        candidate.trim().to_string()
    } else {
        segment_text.chars().take(MAX_QUOTE_CHARS).collect()
    }
}

/// Noon on `date` in `tz`, or the end of the session's own day there when `date` is that day or
/// noon would fall before the session starts.
fn due_at(date: NaiveDate, session: &Session, tz: Tz) -> Option<DateTimeWithTimeZone> {
    let at = |date: NaiveDate, h, m| {
        tz.from_local_datetime(&date.and_time(NaiveTime::from_hms_opt(h, m, 0)?))
            .earliest()
            .map(|t| t.fixed_offset())
    };
    let session_date = session.date_in(tz);
    let end_of_session_day = at(session_date, 23, 59);
    let due = if date == session_date {
        end_of_session_day
    } else {
        at(date, 12, 0)
    };
    due.filter(|due| *due >= session.start)
        .or(end_of_session_day)
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
