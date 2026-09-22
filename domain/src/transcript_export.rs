//! Speaker resolution and plain-text rendering for transcript downloads.

use chrono::NaiveDate;
use entity::transcript_segment::Model as Segment;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::users;

/// Which participant of the coaching relationship a speaker label resolved to.
///
/// Transcript speaker labels are meeting display names reported by the recording
/// provider, so this names the participant a label was matched to by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
#[schema(example = "coach")]
pub enum SpeakerRole {
    /// The relationship's coach, matched by name against the transcript's speaker labels.
    Coach,
    /// The relationship's coachee, matched by name against the transcript's speaker labels.
    Coachee,
}

/// A distinct transcript speaker label and the participant it resolved to, if any.
///
/// One entry per distinct label in the transcript, in first-appearance order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct Speaker {
    /// The speaker label exactly as it appears on transcript lines.
    #[schema(example = "Jim Hodapp")]
    pub label: String,
    /// Which participant the label resolved to.
    ///
    /// `null` when it matched neither participant: a guest, `Unknown`, or a name
    /// that differs from the participant's platform name.
    pub role: Option<SpeakerRole>,
}

/// A rendered plain-text transcript and the filename to serve it under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    /// The full plain-text transcript body.
    pub body: String,
    /// The filename to offer the download under.
    pub filename: String,
}

/// Resolves the transcript's distinct speaker labels to the relationship's participants.
///
/// Coach first, then coachee; each tries preferred name, then full name, then first
/// name, matching case- and whitespace-insensitively. Each user claims at most one
/// label, and a label both participants answer to stays unresolved rather than guessed.
pub fn resolve_speakers(
    coach: &users::Model,
    coachee: &users::Model,
    segments: &[Segment],
) -> Vec<Speaker> {
    let labels = distinct_labels(segments);
    let normalized: Vec<String> = labels.iter().map(|label| normalize(label)).collect();
    let coach_names = name_tiers(coach);
    let coachee_names = name_tiers(coachee);

    let ambiguous: Vec<bool> = normalized
        .iter()
        .map(|label| coach_names.contains(label) && coachee_names.contains(label))
        .collect();
    let coach_index = claim_label(&coach_names, &normalized, |index| ambiguous[index]);
    let coachee_index = claim_label(&coachee_names, &normalized, |index| {
        ambiguous[index] || Some(index) == coach_index
    });

    labels
        .into_iter()
        .enumerate()
        .map(|(index, label)| Speaker {
            label: label.to_owned(),
            role: if Some(index) == coach_index {
                Some(SpeakerRole::Coach)
            } else if Some(index) == coachee_index {
                Some(SpeakerRole::Coachee)
            } else {
                None
            },
        })
        .collect()
}

/// Renders the selected segments as the downloadable plain-text transcript.
///
/// Segments are sorted by `(start_ms, id)` and blank ones dropped; the header lists the
/// speakers with at least one surviving line, in appearance order. Fails when a filtered role has no label.
pub fn render_plain_text(
    session_date: NaiveDate,
    speakers: &[Speaker],
    segments: &[Segment],
    filter: &[SpeakerRole],
) -> Result<Rendered, Error> {
    let resolved: Vec<&str> = filter
        .iter()
        .map(|role| {
            speakers
                .iter()
                .find(|speaker| speaker.role == Some(*role))
                .map(|speaker| speaker.label.as_str())
                .ok_or_else(|| speaker_not_identified(*role, speakers))
        })
        .collect::<Result<_, _>>()?;

    let selected = |label: &str| filter.is_empty() || resolved.contains(&label);

    let mut lines: Vec<&Segment> = segments
        .iter()
        .filter(|segment| selected(&segment.speaker_label) && !segment.text.trim().is_empty())
        .collect();
    lines.sort_by_key(|segment| (segment.start_ms, segment.id));

    // Only speakers who contribute a line are named, so a blank-only speaker is absent.
    let header_labels = speakers
        .iter()
        .filter(|speaker| lines.iter().any(|line| line.speaker_label == speaker.label))
        .map(|speaker| speaker.label.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    let date = session_date.format("%Y-%m-%d");
    let body = lines.iter().fold(
        format!("Coaching session transcript\nDate: {date}\nSpeakers: {header_labels}\n\n"),
        |mut body, segment| {
            body.push_str(&format!(
                "[{}] {}: {}\n",
                format_timestamp(segment.start_ms),
                segment.speaker_label,
                segment.text.trim()
            ));
            body
        },
    );

    let suffix = if filter.is_empty() { "" } else { "-filtered" };

    Ok(Rendered {
        body,
        filename: format!("transcript-{date}{suffix}.txt"),
    })
}

/// The distinct speaker labels in `(start_ms, id)` order, compared exactly as stored.
fn distinct_labels(segments: &[Segment]) -> Vec<&str> {
    let mut ordered: Vec<&Segment> = segments.iter().collect();
    ordered.sort_by_key(|segment| (segment.start_ms, segment.id));
    ordered
        .into_iter()
        .map(|segment| segment.speaker_label.as_str())
        .fold(Vec::new(), |mut labels, label| {
            if !labels.contains(&label) {
                labels.push(label);
            }
            labels
        })
}

/// Lowercases, trims, and collapses internal whitespace runs to a single space.
fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A user's names in matching order, normalized: preferred, full, then first.
fn name_tiers(user: &users::Model) -> Vec<String> {
    [
        user.preferred_name().into_owned(),
        format!("{} {}", user.first_name, user.last_name),
        user.first_name.clone(),
    ]
    .iter()
    .map(|name| normalize(name))
    .collect()
}

/// The index of the first normalized label a name tier matches, skipping `skip` indexes.
fn claim_label(
    names: &[String],
    normalized: &[String],
    skip: impl Fn(usize) -> bool,
) -> Option<usize> {
    names.iter().find_map(|name| {
        normalized
            .iter()
            .enumerate()
            .find(|(index, label)| !skip(*index) && *label == name)
            .map(|(index, _)| index)
    })
}

fn speaker_not_identified(role: SpeakerRole, speakers: &[Speaker]) -> Error {
    Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::SpeakerNotIdentified {
                role,
                labels: speakers
                    .iter()
                    .map(|speaker| speaker.label.clone())
                    .collect(),
            },
        )),
    }
}

/// Formats milliseconds as `m:ss`, or `h:mm:ss` at or over one hour, truncating sub-seconds.
fn format_timestamp(ms: i32) -> String {
    let total_seconds = ms.max(0) / 1000;
    let (hours, minutes, seconds) = (
        total_seconds / 3600,
        (total_seconds % 3600) / 60,
        total_seconds % 60,
    );
    match hours {
        0 => format!("{minutes}:{seconds:02}"),
        _ => format!("{hours}:{minutes:02}:{seconds:02}"),
    }
}

#[cfg(test)]
#[path = "transcript_export_tests.rs"]
mod tests;
