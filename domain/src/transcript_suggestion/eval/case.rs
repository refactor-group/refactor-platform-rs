//! Fixtures: one transcript file per variant, and the suggestions its case expects.

use std::fs;
use std::path::Path;

use chrono::{NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Deserializer};

use crate::transcript_export::{LabeledSegment, SpeakerRole};
use crate::transcript_suggestion::actions::Role;
use crate::transcript_suggestion::{Existing, NextSession, People, Session};
use crate::Id;

/// Where the fixture sets live.
pub(super) const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/transcript_suggestion/eval/fixtures"
);
/// Next session start when the header gives no time.
const DEFAULT_NEXT_SESSION_TIME: NaiveTime = NaiveTime::from_hms_opt(10, 0, 0).unwrap();

/// A fixture set; the sets are scored and reported apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Set {
    /// Clean text written from a script, so no transcription errors.
    Clean,
    /// Real transcripts of recordings, transcription errors included.
    Robustness,
}

impl Set {
    pub(super) const ALL: [Set; 2] = [Set::Clean, Set::Robustness];

    pub(super) fn dir(self) -> &'static str {
        match self {
            Set::Clean => "clean",
            Set::Robustness => "robustness",
        }
    }
}

/// One transcript of a case, with the suggestions it must produce.
pub(super) struct Variant {
    pub set: Set,
    pub case: String,
    /// `v1`, `v2` for wordings; `rec1`, `rec2` for recordings.
    pub name: String,
    pub transcript: Transcript,
    pub expected: Vec<Expected>,
}

impl Variant {
    /// `clean/f05-lookalikes.v2`.
    pub(super) fn id(&self) -> String {
        format!("{}/{}.{}", self.set.dir(), self.case, self.name)
    }

    /// `clean/f05-lookalikes`.
    pub(super) fn case_id(&self) -> String {
        format!("{}/{}", self.set.dir(), self.case)
    }
}

/// One suggestion a case expects.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct Expected {
    /// Line labels the suggestion may be sourced from.
    pub segments: Vec<String>,
    /// Every acceptable set of assignees; a single set may be written without the outer list.
    #[serde(deserialize_with = "assignee_sets")]
    pub assignees: Vec<Vec<Role>>,
    /// Every acceptable due date, `null` meaning no date.
    pub due_dates: Vec<Option<NaiveDate>>,
    /// The existing action label it repeats, if any.
    pub duplicate_of: Option<String>,
    pub note: String,
}

/// `["coach"]` or `[["coach", "coachee"], ["coachee"]]`.
fn assignee_sets<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Vec<Role>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Sets {
        One(Vec<Role>),
        Any(Vec<Vec<Role>>),
    }
    Ok(match Sets::deserialize(deserializer)? {
        Sets::One(set) => vec![set],
        Sets::Any(sets) => sets,
    })
}

/// A parsed transcript file.
#[derive(Debug)]
pub(super) struct Transcript {
    pub session_date: NaiveDate,
    pub timezone: Tz,
    pub next_session: Option<(NaiveDate, NaiveTime)>,
    pub week_end: Weekday,
    pub coach_name: String,
    pub coachee_name: String,
    /// `(label, text)` in file order.
    pub existing: Vec<(String, String)>,
    pub lines: Vec<Line>,
}

/// One speaker-labelled line.
#[derive(Debug)]
pub(super) struct Line {
    /// The label written in the file, such as `s3` or `s3b`; stable across variants.
    pub label: String,
    pub start_ms: i32,
    /// `Coach`, `Coachee`, or a third person's name.
    pub speaker: String,
    pub text: String,
}

/// A transcript as the backend's own input, with fresh ids.
pub(super) struct Input {
    pub coaching_session_id: Id,
    pub people: People,
    pub segments: Vec<LabeledSegment>,
    pub existing_ids: Vec<Id>,
}

impl Transcript {
    /// Builds the segments and ids a `Session` borrows.
    pub(super) fn input(&self) -> Input {
        let people = People {
            coach_id: Id::new_v4(),
            coachee_id: Id::new_v4(),
        };
        let segments = self
            .lines
            .iter()
            .map(|line| {
                let role = match line.speaker.as_str() {
                    "Coach" => Some(SpeakerRole::Coach),
                    "Coachee" => Some(SpeakerRole::Coachee),
                    _ => None,
                };
                let label = match role {
                    Some(SpeakerRole::Coach) => self.coach_name.clone(),
                    Some(SpeakerRole::Coachee) => self.coachee_name.clone(),
                    None => line.speaker.clone(),
                };
                LabeledSegment {
                    id: Id::new_v4(),
                    transcription_id: Id::nil(),
                    speaker_label: label,
                    speaker_user_id: role.map(|r| match r {
                        SpeakerRole::Coach => people.coach_id,
                        SpeakerRole::Coachee => people.coachee_id,
                    }),
                    speaker_role: role,
                    text: line.text.clone(),
                    start_ms: line.start_ms,
                    end_ms: line.start_ms + 1000,
                    confidence: None,
                    sentiment: None,
                    created_at: Utc::now().fixed_offset(),
                }
            })
            .collect();
        Input {
            coaching_session_id: Id::new_v4(),
            people,
            segments,
            existing_ids: self.existing.iter().map(|_| Id::new_v4()).collect(),
        }
    }

    /// The `Session` extraction reads, as the backend would build it from the database.
    pub(super) fn session<'a>(&self, input: &'a Input) -> Session<'a> {
        Session {
            coaching_session_id: input.coaching_session_id,
            people: input.people,
            coach_name: self.coach_name.clone(),
            coachee_name: self.coachee_name.clone(),
            coach_timezone: self.timezone,
            session_date: self.session_date,
            next_session: self.next_session.and_then(|(date, time)| {
                self.timezone
                    .from_local_datetime(&date.and_time(time))
                    .earliest()
                    .map(|start| NextSession {
                        date,
                        start: start.fixed_offset(),
                    })
            }),
            week_end: self.week_end,
            segments: &input.segments,
            existing: input
                .existing_ids
                .iter()
                .zip(&self.existing)
                .map(|(id, (_, text))| Existing {
                    id: *id,
                    text: text.clone(),
                })
                .collect(),
        }
    }

    /// The file label of the segment with this id.
    pub(super) fn segment_label(&self, input: &Input, id: Id) -> Option<&str> {
        let index = input.segments.iter().position(|s| s.id == id)?;
        self.lines.get(index).map(|l| l.label.as_str())
    }

    /// The file label of the existing action with this id.
    pub(super) fn existing_label(&self, input: &Input, id: Id) -> Option<&str> {
        let index = input.existing_ids.iter().position(|e| *e == id)?;
        self.existing.get(index).map(|(label, _)| label.as_str())
    }

    /// The text of the line with this label.
    pub(super) fn text(&self, label: &str) -> Option<&str> {
        self.lines
            .iter()
            .find(|l| l.label == label)
            .map(|l| l.text.as_str())
    }
}

/// Parses a transcript file.
///
/// Header lines start with `#`: `session`, `next_session` (a date and optional `HH:MM`, or `none`),
/// `coach`, `coachee`, and optionally `timezone`, `week_end` and `existing <label>`. Any other `#`
/// line is a comment. Each transcript line reads `[<label> <mm:ss>] <speaker>: <text>`.
pub(super) fn parse_transcript(source: &str) -> Result<Transcript, String> {
    let mut session_date = None;
    let mut next_session = None;
    let mut timezone = chrono_tz::America::Chicago;
    let mut week_end = Weekday::Fri;
    let (mut coach_name, mut coachee_name) = (None, None);
    let mut existing = Vec::new();
    let mut lines = Vec::new();

    for (number, raw) in source.lines().enumerate().map(|(i, l)| (i + 1, l.trim())) {
        let at = |message: String| format!("line {number}: {message}");
        if raw.is_empty() {
            continue;
        }
        if let Some(header) = raw.strip_prefix('#') {
            let Some((key, value)) = header.split_once(':') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "session" => {
                    session_date =
                        Some(date_in(value).ok_or_else(|| at(format!("no date in {value:?}")))?)
                }
                "next_session" if value.starts_with("none") => next_session = None,
                "next_session" => {
                    let date = date_in(value).ok_or_else(|| at(format!("no date in {value:?}")))?;
                    let time = value
                        .split_whitespace()
                        .find_map(|w| NaiveTime::parse_from_str(w, "%H:%M").ok())
                        .unwrap_or(DEFAULT_NEXT_SESSION_TIME);
                    next_session = Some((date, time));
                }
                "timezone" => {
                    timezone = value
                        .parse()
                        .map_err(|_| at(format!("unknown timezone {value:?}")))?
                }
                "week_end" => {
                    week_end = value
                        .parse()
                        .map_err(|_| at(format!("unknown weekday {value:?}")))?
                }
                "coach" => coach_name = Some(value.to_string()),
                "coachee" => coachee_name = Some(value.to_string()),
                key => {
                    if let Some(label) = key.strip_prefix("existing ") {
                        existing.push((label.trim().to_string(), value.to_string()));
                    }
                }
            }
            continue;
        }
        lines.push(parse_line(raw).ok_or_else(|| at(format!("not a transcript line: {raw:?}")))?);
    }

    Ok(Transcript {
        session_date: session_date.ok_or("missing `# session:`")?,
        timezone,
        next_session,
        week_end,
        coach_name: coach_name.ok_or("missing `# coach:`")?,
        coachee_name: coachee_name.ok_or("missing `# coachee:`")?,
        existing,
        lines,
    })
}

/// `[s3 01:05] Coachee: text`.
fn parse_line(raw: &str) -> Option<Line> {
    let (head, rest) = raw.strip_prefix('[')?.split_once("] ")?;
    let (label, clock) = head.split_once(' ')?;
    let (minutes, seconds) = clock.split_once(':')?;
    let (speaker, text) = rest.split_once(": ")?;
    let start_ms = (minutes.parse::<i32>().ok()? * 60 + seconds.parse::<i32>().ok()?) * 1000;
    Some(Line {
        label: label.to_string(),
        start_ms,
        speaker: speaker.trim().to_string(),
        text: text.trim().to_string(),
    })
}

/// The first `YYYY-MM-DD` word in `value`.
fn date_in(value: &str) -> Option<NaiveDate> {
    value
        .split_whitespace()
        .find_map(|w| NaiveDate::parse_from_str(w.trim_matches(','), "%Y-%m-%d").ok())
}

/// Every variant of every case, sorted by id.
///
/// `<case>.<variant>.txt` is scored against `<case>.<variant>.expected.json` when it exists, so a
/// recording with its own line breaks can carry its own expectations, else `<case>.expected.json`.
pub(super) fn load_all() -> Result<Vec<Variant>, String> {
    let mut variants = Set::ALL
        .into_iter()
        .map(|set| load_set(set, &Path::new(FIXTURES).join(set.dir())))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    variants.sort_by_key(Variant::id);
    Ok(variants)
}

fn load_set(set: Set, dir: &Path) -> Result<Vec<Variant>, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|x| x == "txt"))
        .map(|path| load_variant(set, &path))
        .collect()
}

fn load_variant(set: Set, path: &Path) -> Result<Variant, String> {
    let context = |message: String| format!("{}: {message}", path.display());
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| context("unreadable file name".into()))?;
    let (case, name) = stem
        .split_once('.')
        .ok_or_else(|| context("name it <case>.<variant>.txt".into()))?;
    let transcript = fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|source| parse_transcript(&source))
        .map_err(context)?;
    let expected_path = [
        format!("{case}.{name}.expected.json"),
        format!("{case}.expected.json"),
    ]
    .into_iter()
    .map(|file| path.with_file_name(file))
    .find(|p| p.exists())
    .ok_or_else(|| context(format!("no {case}.expected.json")))?;
    let expected = read_expected(&expected_path)?;
    Ok(Variant {
        set,
        case: case.to_string(),
        name: name.to_string(),
        transcript,
        expected,
    })
}

fn read_expected(path: &Path) -> Result<Vec<Expected>, String> {
    fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|json| serde_json::from_str(&json).map_err(|e| e.to_string()))
        .map_err(|e| format!("{}: {e}", path.display()))
}
