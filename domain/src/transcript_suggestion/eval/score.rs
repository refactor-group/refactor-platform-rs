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

/// Cost of leaving a suggestion or an expectation unpaired; above any pairing's problem count, so a
/// suggestion on an expected line is always paired with one.
const UNPAIRED: i64 = 10;
/// Cost of pairing a suggestion with an expectation on another line; never chosen.
const FORBIDDEN: i64 = 1_000;

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
    let pairs = pair(expected, got, transcript);
    let mut findings: Vec<Finding> = got
        .iter()
        .zip(&pairs)
        .flat_map(|(g, paired)| {
            let finding = match paired {
                None => Finding::Extra(describe(g)),
                Some(index) => match problems(&expected[*index], g, transcript) {
                    problems if problems.is_empty() => Finding::Ok(describe(g)),
                    problems => Finding::Wrong(describe(g), problems),
                },
            };
            let note = has_note(&g.body).then(|| Finding::Note(g.body.clone()));
            std::iter::once(finding).chain(note)
        })
        .collect();
    findings.extend(
        expected
            .iter()
            .enumerate()
            .filter(|(index, _)| !pairs.contains(&Some(*index)))
            .map(|(_, e)| Finding::Miss(format!("{:?} {}", e.segments, e.note))),
    );
    findings
}

/// For each suggestion, the expectation it answers, if any.
///
/// A suggestion pairs only with an expectation that allows its line. Of all such pairings, this is
/// the one with the fewest errors, so the order the model lists suggestions in never changes a score.
fn pair(expected: &[Expected], got: &[Got], transcript: &Transcript) -> Vec<Option<usize>> {
    let (rows, cols) = (got.len(), expected.len());
    let on_line = |row: usize, col: usize| expected[col].segments.contains(&got[row].segment);
    // Suggestions then one placeholder per expectation (a miss), by expectations then one
    // placeholder per suggestion (an extra).
    let cost: Vec<Vec<i64>> = (0..rows + cols)
        .map(|row| {
            (0..rows + cols)
                .map(|col| match (row < rows, col < cols) {
                    (true, true) if on_line(row, col) => {
                        problems(&expected[col], &got[row], transcript).len() as i64
                    }
                    (true, true) => FORBIDDEN,
                    (true, false) | (false, true) => UNPAIRED,
                    (false, false) => 0,
                })
                .collect()
        })
        .collect();
    let assignment = min_cost_assignment(&cost);
    (0..rows)
        .map(|row| Some(assignment[row]).filter(|&col| col < cols && on_line(row, col)))
        .collect()
}

/// For each row of a square cost matrix, its column in the assignment with the lowest total cost.
///
/// The Hungarian algorithm, O(n³); kept imperative because that is how it reads best.
fn min_cost_assignment(cost: &[Vec<i64>]) -> Vec<usize> {
    let n = cost.len();
    // 1-based potentials and matches; column 0 is the algorithm's sentinel.
    let (mut u, mut v) = (vec![0i64; n + 1], vec![0i64; n + 1]);
    let (mut row_of, mut way) = (vec![0usize; n + 1], vec![0usize; n + 1]);
    for row in 1..=n {
        row_of[0] = row;
        let mut col0 = 0;
        let mut min_slack = vec![i64::MAX; n + 1];
        let mut used = vec![false; n + 1];
        loop {
            used[col0] = true;
            let row0 = row_of[col0];
            let (mut delta, mut col1) = (i64::MAX, 0);
            for col in (1..=n).filter(|&c| !used[c]) {
                let slack = cost[row0 - 1][col - 1] - u[row0] - v[col];
                if slack < min_slack[col] {
                    min_slack[col] = slack;
                    way[col] = col0;
                }
                if min_slack[col] < delta {
                    delta = min_slack[col];
                    col1 = col;
                }
            }
            for col in 0..=n {
                if used[col] {
                    u[row_of[col]] += delta;
                    v[col] -= delta;
                } else {
                    min_slack[col] -= delta;
                }
            }
            col0 = col1;
            if row_of[col0] == 0 {
                break;
            }
        }
        while col0 != 0 {
            let previous = way[col0];
            row_of[col0] = row_of[previous];
            col0 = previous;
        }
    }
    let mut column_of = vec![0; n];
    for col in 1..=n {
        column_of[row_of[col] - 1] = col - 1;
    }
    column_of
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
