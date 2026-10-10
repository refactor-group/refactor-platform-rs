//! Deterministic resolution of spoken deadlines against the session date.

use chrono::{Datelike, Days, NaiveDate, Weekday};

/// What a spoken deadline names.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Phrase {
    Date(NaiveDate),
    Ambiguous,
    Unrecognized,
}

/// Leading words that only introduce a deadline.
const FILLERS: [&str; 8] = [
    "no later than",
    "by",
    "before",
    "on",
    "until",
    "due",
    "the",
    "our",
];

const SAME_DAY: [&str; 5] = ["today", "tonight", "eod", "end of day", "end of the day"];

const TOMORROW_PARTS: [&str; 7] = [
    "eod",
    "end of day",
    "end of the day",
    "morning",
    "afternoon",
    "evening",
    "night",
];

const NEXT_SESSION: [&str; 6] = [
    "next session",
    "next coaching session",
    "next time",
    "next meeting",
    "we meet again",
    "time we meet again",
];

const VAGUE: [&str; 11] = [
    "soon",
    "asap",
    "as soon as possible",
    "sometime",
    "eventually",
    "later",
    "later this year",
    "next week",
    "next month",
    "this year",
    "at some point",
];

const END_OF_WEEK: [&str; 3] = ["end of week", "end of the week", "end of this week"];
const WEEKEND: [&str; 3] = ["weekend", "this weekend", "over the weekend"];
const END_OF_MONTH: [&str; 3] = ["end of month", "end of the month", "end of this month"];

/// Resolves a spoken deadline against the session date and the next session's date.
pub(crate) fn resolve_phrase(
    phrase: &str,
    session_date: NaiveDate,
    next_session: Option<NaiveDate>,
) -> Phrase {
    let normalized = normalize(phrase);
    let text = strip_filler(&normalized);

    if NEXT_SESSION.contains(&text) {
        return next_session.map_or(Phrase::Ambiguous, Phrase::Date);
    }
    if VAGUE.contains(&text) || text.starts_with("at some point ") {
        return Phrase::Ambiguous;
    }
    if let Some(weekday) = text.strip_prefix("next ").and_then(weekday) {
        return next_weekday(weekday, session_date);
    }

    fixed(text, session_date)
        .or_else(|| relative(text, session_date))
        .or_else(|| explicit(text, session_date))
        .unwrap_or(Phrase::Unrecognized)
}

/// The deadline to store: a recognized phrase wins, an ambiguous one clears, otherwise the model's.
pub(crate) fn reconcile(
    model_date: Option<NaiveDate>,
    phrase: Option<&str>,
    session_date: NaiveDate,
    next_session: Option<NaiveDate>,
) -> Option<NaiveDate> {
    let latest = session_date.checked_add_days(Days::new(365))?;
    let in_bounds = |d: &NaiveDate| (session_date..=latest).contains(d);
    match phrase.map(|p| resolve_phrase(p, session_date, next_session)) {
        Some(Phrase::Date(date)) => Some(date).filter(in_bounds),
        Some(Phrase::Ambiguous) => None,
        Some(Phrase::Unrecognized) | None => model_date.filter(in_bounds),
    }
}

/// Lowercased, punctuation (except `/`) turned to spaces, whitespace collapsed.
fn normalize(phrase: &str) -> String {
    phrase
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '/' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Drops leading filler words, repeatedly.
fn strip_filler(text: &str) -> &str {
    FILLERS
        .iter()
        .find_map(|filler| text.strip_prefix(filler)?.strip_prefix(' '))
        .map_or(text, strip_filler)
}

/// Phrases tied to a fixed point: today, tomorrow, end of week, weekend, end of month.
fn fixed(text: &str, session_date: NaiveDate) -> Option<Phrase> {
    let is_tomorrow = text == "tomorrow"
        || text
            .strip_prefix("tomorrow ")
            .or_else(|| text.strip_suffix(" tomorrow"))
            .is_some_and(|part| TOMORROW_PARTS.contains(&part));
    let friday = on_or_after(session_date, Weekday::Fri);

    let date = if SAME_DAY.contains(&text) {
        Some(session_date)
    } else if is_tomorrow {
        session_date.succ_opt()
    } else if END_OF_WEEK.contains(&text) {
        friday
    } else if text == "end of next week" {
        friday.and_then(|d| d.checked_add_days(Days::new(7)))
    } else if WEEKEND.contains(&text) {
        on_or_after(session_date, Weekday::Sun)
    } else if END_OF_MONTH.contains(&text) {
        end_of_month(session_date)
    } else {
        return None;
    };
    date.map(Phrase::Date)
}

/// Weekdays, `in N days|weeks`, and `N weeks from <anchor>`.
fn relative(text: &str, session_date: NaiveDate) -> Option<Phrase> {
    if let Some(day) = weekday(text.strip_prefix("this ").unwrap_or(text)) {
        return after(session_date, day).map(Phrase::Date);
    }
    if let Some(rest) = text.strip_prefix("in ") {
        let (count, unit) = rest.split_once(' ')?;
        return offset(session_date, number(count)?, unit).map(Phrase::Date);
    }
    let (amount, anchor) = text.split_once(" from ")?;
    let (count, unit) = amount.split_once(' ')?;
    let start = match anchor {
        "today" | "now" => Some(session_date),
        _ => weekday(anchor.strip_prefix("this ").unwrap_or(anchor))
            .and_then(|day| on_or_after(session_date, day)),
    }?;
    offset(start, number(count)?, unit).map(Phrase::Date)
}

/// `<month> <day>`, `<day> of <month>`, `M/D`, in the session's year or the next.
fn explicit(text: &str, session_date: NaiveDate) -> Option<Phrase> {
    let (month, day) = if let Some((m, d)) = text.split_once('/') {
        (m.parse().ok()?, d.parse().ok()?)
    } else if let Some((d, m)) = text.split_once(" of ") {
        (month(m)?, ordinal(d)?)
    } else {
        let (m, d) = text.split_once(' ')?;
        (month(m)?, ordinal(d)?)
    };
    let in_year = |year| NaiveDate::from_ymd_opt(year, month, day);
    let date = in_year(session_date.year())
        .filter(|d| *d >= session_date)
        .or_else(|| in_year(session_date.year() + 1));
    Some(date.map_or(Phrase::Unrecognized, Phrase::Date))
}

/// `next <weekday>` is ambiguous while that weekday is still ahead in the session's week.
fn next_weekday(day: Weekday, session_date: NaiveDate) -> Phrase {
    if day.num_days_from_monday() > session_date.weekday().num_days_from_monday() {
        Phrase::Ambiguous
    } else {
        after(session_date, day).map_or(Phrase::Unrecognized, Phrase::Date)
    }
}

fn offset(start: NaiveDate, count: u64, unit: &str) -> Option<NaiveDate> {
    let days = match unit {
        "day" | "days" => count,
        "week" | "weeks" => count.checked_mul(7)?,
        _ => return None,
    };
    start.checked_add_days(Days::new(days))
}

/// The first `day` strictly after `date`.
fn after(date: NaiveDate, day: Weekday) -> Option<NaiveDate> {
    date.succ_opt().and_then(|d| on_or_after(d, day))
}

/// The first `day` on or after `date`.
fn on_or_after(date: NaiveDate, day: Weekday) -> Option<NaiveDate> {
    let ahead = (7 + day.num_days_from_monday() - date.weekday().num_days_from_monday()) % 7;
    date.checked_add_days(Days::new(u64::from(ahead)))
}

fn end_of_month(date: NaiveDate) -> Option<NaiveDate> {
    let (year, month) = match date.month() {
        12 => (date.year() + 1, 1),
        m => (date.year(), m + 1),
    };
    NaiveDate::from_ymd_opt(year, month, 1)?.pred_opt()
}

fn number(word: &str) -> Option<u64> {
    let words = [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    ];
    match word {
        "a" | "an" => Some(1),
        _ if word.chars().all(|c| c.is_ascii_digit()) => word.parse().ok(),
        _ => words
            .iter()
            .position(|w| *w == word)
            .and_then(|i| u64::try_from(i + 1).ok()),
    }
}

/// A day of the month, digits with an optional `st`/`nd`/`rd`/`th` suffix.
fn ordinal(word: &str) -> Option<u32> {
    let digits = ["st", "nd", "rd", "th"]
        .iter()
        .find_map(|suffix| word.strip_suffix(suffix))
        .unwrap_or(word);
    digits
        .chars()
        .all(|c| c.is_ascii_digit())
        .then_some(digits)
        .and_then(|d| d.parse().ok())
}

fn weekday(word: &str) -> Option<Weekday> {
    match word {
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" | "tues" => Some(Weekday::Tue),
        "wednesday" | "wed" => Some(Weekday::Wed),
        "thursday" | "thu" | "thur" | "thurs" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        "sunday" | "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

fn month(word: &str) -> Option<u32> {
    match word {
        "january" | "jan" => Some(1),
        "february" | "feb" => Some(2),
        "march" | "mar" => Some(3),
        "april" | "apr" => Some(4),
        "may" => Some(5),
        "june" | "jun" => Some(6),
        "july" | "jul" => Some(7),
        "august" | "aug" => Some(8),
        "september" | "sep" | "sept" => Some(9),
        "october" | "oct" => Some(10),
        "november" | "nov" => Some(11),
        "december" | "dec" => Some(12),
        _ => None,
    }
}

#[cfg(test)]
// The frozen spec file is kept byte-for-byte as written.
#[rustfmt::skip]
#[path = "due_date_tests.rs"]
mod tests;
