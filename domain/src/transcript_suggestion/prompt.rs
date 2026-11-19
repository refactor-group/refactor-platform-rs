//! The model's input for one session: session details, a calendar, existing actions and the transcript.

use chrono::{Datelike, NaiveDate, Weekday};

use super::Session;
use crate::coaching_relationship_kind;
use crate::transcript_export::LabeledSegment;
use crate::Id;

/// Days after the session the calendar covers.
const CALENDAR_DAYS: usize = 60;
/// Longest existing action text shown to the model.
const EXISTING_CHARS: usize = 300;

/// Short refs the model uses instead of ids: `s1..sN` for segments, `a1..aN` for existing actions.
pub struct Refs {
    segments: Vec<Id>,
    existing: Vec<Id>,
}

impl Refs {
    /// Numbers the session's segments and existing actions in order, from 1.
    pub fn new(session: &Session) -> Self {
        Self {
            segments: session.segments.iter().map(|s| s.id).collect(),
            existing: session.existing.iter().map(|e| e.id).collect(),
        }
    }

    /// The segment id for a ref like `s3`.
    pub fn segment(&self, r: &str) -> Option<Id> {
        lookup(&self.segments, r, 's')
    }

    /// The existing action id for a ref like `a1`.
    pub fn existing(&self, r: &str) -> Option<Id> {
        lookup(&self.existing, r, 'a')
    }

    /// The text of the session's segment with this id.
    pub fn segment_text<'a>(&self, session: &'a Session, id: Id) -> Option<&'a str> {
        session
            .segments
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.text.as_str())
    }
}

/// The user message for one session.
pub fn render(session: &Session) -> String {
    let next_session = session
        .next_session
        .map_or("none scheduled".to_string(), |next| long_date(next.date));
    let existing = if session.existing.is_empty() {
        vec!["(none)".to_string()]
    } else {
        session
            .existing
            .iter()
            .enumerate()
            .map(|(i, e)| format!("[a{}] {}", i + 1, existing_text(&e.text)))
            .collect()
    };

    let people = if session.kind.rules().names_people_in_prompt() {
        vec![format!(
            "Colleagues: {}, {}",
            session.coach_name, session.coachee_name
        )]
    } else {
        vec![
            format!("Coach: {}", session.coach_name),
            format!("Coachee: {}", session.coachee_name),
        ]
    };

    people
        .into_iter()
        .chain([
            format!("Session date: {}", long_date(session.session_date)),
            format!("Next session: {next_session}"),
            format!("Week ends on: {}", day_name(session.week_end)),
            "Calendar (the 60 days after the session):".to_string(),
        ])
        .chain(calendar(session.session_date))
        .chain([
            String::new(),
            "Existing actions in this session:".to_string(),
        ])
        .chain(existing)
        .chain([String::new(), "Transcript:".to_string()])
        .chain(
            session
                .segments
                .iter()
                .enumerate()
                .map(|(i, s)| transcript_line(session, i + 1, s)),
        )
        .collect::<Vec<_>>()
        .join("\n")
}

/// The names a colleague prompt gives the two people, in seat order: their preferred names when
/// those differ, else their full names when those differ, else the preferred names numbered.
pub fn distinct_names(preferred: [String; 2], full: [String; 2]) -> [String; 2] {
    let [coach, coachee] = preferred;
    if coach != coachee {
        [coach, coachee]
    } else if full[0] != full[1] {
        full
    } else {
        [format!("{coach} (1)"), format!("{coachee} (2)")]
    }
}

/// `prefix` then a 1-based index with no leading zero.
fn lookup(ids: &[Id], r: &str, prefix: char) -> Option<Id> {
    let digits = r.strip_prefix(prefix)?;
    let index: usize = (!digits.starts_with('0') && digits.chars().all(|c| c.is_ascii_digit()))
        .then_some(digits)?
        .parse()
        .ok()?;
    ids.get(index.checked_sub(1)?).copied()
}

fn day_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

fn long_date(date: NaiveDate) -> String {
    date.format("%A %Y-%m-%d").to_string()
}

/// One line per Monday-Sunday week, from the session date through `CALENDAR_DAYS` after it.
fn calendar(session_date: NaiveDate) -> Vec<String> {
    let days: Vec<NaiveDate> = session_date.iter_days().take(CALENDAR_DAYS + 1).collect();
    days.chunk_by(|_, next| next.weekday() != Weekday::Mon)
        .map(|week| {
            week.iter()
                .map(|d| d.format("%a %Y-%m-%d").to_string())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .collect()
}

fn existing_text(text: &str) -> String {
    collapse(text).chars().take(EXISTING_CHARS).collect()
}

fn transcript_line(session: &Session, index: usize, segment: &LabeledSegment) -> String {
    let speaker = if session.kind.rules().names_people_in_prompt() {
        segment
            .speaker_user_id
            .and_then(|id| session.seat_name(id))
            .unwrap_or(&segment.speaker_label)
    } else {
        match segment.speaker_role {
            Some(coaching_relationship_kind::Role::Coach) => "Coach",
            Some(coaching_relationship_kind::Role::Coachee) => "Coachee",
            Some(coaching_relationship_kind::Role::Colleague) | None => {
                segment.speaker_label.as_str()
            }
        }
    };
    let seconds = segment.start_ms.max(0) / 1000;
    let text = segment.text.lines().collect::<Vec<_>>().join(" ");
    format!(
        "[s{index} {:02}:{:02}] {speaker}: {text}",
        seconds / 60,
        seconds % 60
    )
}

/// Runs of whitespace, newlines included, become single spaces; trimmed.
pub(super) fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
