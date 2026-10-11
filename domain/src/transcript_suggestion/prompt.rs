//! The model's input for one session: session details, a calendar, existing actions and the transcript.

use chrono::{Datelike, NaiveDate, Weekday};

use super::Session;
use crate::transcript_export::{LabeledSegment, SpeakerRole};
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

    [
        format!("Coach: {}", session.coach_name),
        format!("Coachee: {}", session.coachee_name),
        format!("Session date: {}", long_date(session.session_date)),
        format!("Next session: {next_session}"),
        "Calendar (the 60 days after the session):".to_string(),
    ]
    .into_iter()
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
            .map(|(i, s)| transcript_line(i + 1, s)),
    )
    .collect::<Vec<_>>()
    .join("\n")
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

fn transcript_line(index: usize, segment: &LabeledSegment) -> String {
    let speaker = match segment.speaker_role {
        Some(SpeakerRole::Coach) => "Coach",
        Some(SpeakerRole::Coachee) => "Coachee",
        None => segment.speaker_label.as_str(),
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
