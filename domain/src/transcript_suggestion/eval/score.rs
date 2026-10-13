//! Scoring one run's verified suggestions against what its case expects.

use std::fmt;

use chrono::NaiveDate;

use super::case::{Expected, Transcript};
use crate::transcript_suggestion::actions::Role;
use crate::transcript_suggestion::prompt::collapse;

/// Words in an action body that mean the model wrote a note or a placeholder instead of a task.
const NOTE_WORDS: [&str; 6] = [
    "unclear",
    "tbd",
    "not specified",
    "unspecified",
    "details from",
    "placeholder",
];

/// One verified suggestion in fixture terms.
#[derive(Debug, Clone)]
pub(super) struct Got {
    /// The source line's file label.
    pub segment: String,
    pub assignees: Vec<Role>,
    /// `due_by` as a date in the coach's timezone.
    pub due: Option<NaiveDate>,
    pub due_phrase: Option<String>,
    /// The repeated existing action's file label.
    pub duplicate_of: Option<String>,
    pub quote: String,
    pub body: String,
}

/// What scoring found about one suggestion, or one expected suggestion that never came.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Finding {
    /// Matched an expected suggestion with nothing wrong.
    Ok(String),
    /// Matched an expected suggestion, with these problems.
    Wrong(String, Vec<String>),
    /// Matched nothing expected.
    Extra(String),
    /// Expected but never suggested.
    Miss(String),
    /// The body reads like a note or placeholder.
    Note(String),
}

impl Finding {
    /// Problems this finding counts; a run passes with none.
    pub(super) fn errors(&self) -> usize {
        match self {
            Finding::Ok(_) => 0,
            Finding::Wrong(_, problems) => problems.len(),
            Finding::Extra(_) | Finding::Miss(_) | Finding::Note(_) => 1,
        }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Finding::Ok(what) => write!(f, "ok    {what}"),
            Finding::Wrong(what, problems) => {
                write!(f, "WRONG {what}  <-- {}", problems.join("; "))
            }
            Finding::Extra(what) => write!(f, "EXTRA {what}"),
            Finding::Miss(what) => write!(f, "MISS  {what}"),
            Finding::Note(body) => write!(f, "NOTE  body has a note or placeholder: {body:?}"),
        }
    }
}

/// Every finding for one run, in the model's order then the misses.
pub(super) fn score(expected: &[Expected], got: &[Got], transcript: &Transcript) -> Vec<Finding> {
    let mut open: Vec<&Expected> = expected.iter().collect();
    let mut findings: Vec<Finding> = got
        .iter()
        .flat_map(|g| {
            let matched = best_match(&open, g).map(|index| open.remove(index));
            let finding = match matched {
                None => Finding::Extra(describe(g)),
                Some(e) => match problems(e, g, transcript) {
                    problems if problems.is_empty() => Finding::Ok(describe(g)),
                    problems => Finding::Wrong(describe(g), problems),
                },
            };
            let note = has_note(&g.body).then(|| Finding::Note(g.body.clone()));
            std::iter::once(finding).chain(note)
        })
        .collect();
    findings.extend(
        open.into_iter()
            .map(|e| Finding::Miss(format!("{:?} {}", e.segments, e.note))),
    );
    findings
}

/// The open expectation `got` matches best: same line, people and date; then line and people;
/// then line alone, which is then reported as wrong.
fn best_match(open: &[&Expected], got: &Got) -> Option<usize> {
    let on_line = |e: &&Expected| e.segments.contains(&got.segment);
    let people = |e: &&Expected| any_people(e, &got.assignees);
    let date = |e: &&Expected| e.due_dates.contains(&got.due);
    open.iter()
        .position(|e| on_line(e) && people(e) && date(e))
        .or_else(|| open.iter().position(|e| on_line(e) && people(e)))
        .or_else(|| open.iter().position(on_line))
}

fn problems(expected: &Expected, got: &Got, transcript: &Transcript) -> Vec<String> {
    let quote_found = transcript
        .text(&got.segment)
        .is_some_and(|text| contains(text, &got.quote));
    [
        (!any_people(expected, &got.assignees)).then(|| {
            format!(
                "assignees {:?}, expected {:?}",
                got.assignees, expected.assignees
            )
        }),
        (!expected.due_dates.contains(&got.due)).then(|| {
            format!(
                "due {}, expected one of {}",
                show_date(got.due),
                show_dates(&expected.due_dates)
            )
        }),
        (expected.duplicate_of != got.duplicate_of).then(|| {
            format!(
                "duplicate_of {:?}, expected {:?}",
                got.duplicate_of, expected.duplicate_of
            )
        }),
        (!quote_found).then(|| format!("quote {:?} is not in {}", got.quote, got.segment)),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn any_people(expected: &Expected, got: &[Role]) -> bool {
    expected.assignees.iter().any(|set| same_people(set, got))
}

fn same_people(a: &[Role], b: &[Role]) -> bool {
    a.iter().all(|r| b.contains(r)) && b.iter().all(|r| a.contains(r))
}

/// Case- and whitespace-insensitive containment, as verification checks quotes.
fn contains(text: &str, quote: &str) -> bool {
    let quote = collapse(quote).to_lowercase();
    !quote.is_empty() && collapse(text).to_lowercase().contains(&quote)
}

pub(super) fn has_note(body: &str) -> bool {
    let lower = body.to_lowercase();
    body.contains(['(', ')', '[', ']', '?']) || NOTE_WORDS.iter().any(|w| lower.contains(w))
}

fn describe(got: &Got) -> String {
    let phrase = got
        .due_phrase
        .as_deref()
        .map(|p| format!(" ({p:?})"))
        .unwrap_or_default();
    let duplicate = got
        .duplicate_of
        .as_deref()
        .map(|d| format!(" dup={d}"))
        .unwrap_or_default();
    format!(
        "{} {:?} due {}{phrase}{duplicate} | {}",
        got.segment,
        got.assignees,
        show_date(got.due),
        got.body
    )
}

fn show_date(date: Option<NaiveDate>) -> String {
    date.map_or("none".to_string(), |d| d.to_string())
}

fn show_dates(dates: &[Option<NaiveDate>]) -> String {
    dates
        .iter()
        .map(|d| show_date(*d))
        .collect::<Vec<_>>()
        .join(", ")
}
