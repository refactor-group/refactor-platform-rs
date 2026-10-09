//! Speaker labeling and plain-text rendering for transcript reads and downloads.

use std::collections::HashMap;
use std::iter;

use chrono::NaiveDate;
use entity::transcript_segment::Model as Segment;
use sea_orm::prelude::DateTimeWithTimeZone;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::transcript_participant::Model as Participant;
use crate::users;
use crate::Id;

/// Which participant of the coaching relationship a speaker is.
///
/// Comes from attribution stored when the transcript completed (the participant's own
/// meeting account, or elimination), never from matching names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
#[schema(example = "coach")]
pub enum SpeakerRole {
    /// The relationship's coach.
    Coach,
    /// The relationship's coachee.
    Coachee,
}

/// A distinct transcript speaker and the relationship role attributed to them, if any.
///
/// One entry per distinct speaker in the transcript, in first-speaking order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, ToSchema)]
pub struct Speaker {
    /// The speaker label exactly as it appears on transcript lines; unique per transcript.
    #[schema(example = "Jim Hodapp")]
    pub label: String,
    /// Which participant the speaker is attributed to.
    ///
    /// `null` for anyone not attributed to the coach or coachee, such as a guest.
    pub role: Option<SpeakerRole>,
}

/// A transcript segment as readers see it: the shown label and who spoke it.
#[derive(Clone, Debug, PartialEq, Serialize, ToSchema)]
#[schema(as = domain::transcript_export::LabeledSegment)]
pub struct LabeledSegment {
    pub id: Id,
    pub transcription_id: Id,
    /// The speaker's label, identical to one `Speaker::label` of the same transcript.
    ///
    /// The profile name for the coach or coachee, the typed meeting name otherwise, or
    /// `Guest N` when there is none.
    #[schema(example = "Jim Hodapp")]
    pub speaker_label: String,
    /// The user who spoke, only ever the session's coach or coachee; `null` otherwise.
    pub speaker_user_id: Option<Id>,
    /// The speaker's role in the relationship; `null` when not the coach or coachee.
    pub speaker_role: Option<SpeakerRole>,
    pub text: String,
    pub start_ms: i32,
    pub end_ms: i32,
    pub confidence: Option<f64>,
    pub sentiment: Option<String>,
    #[schema(value_type = String, format = DateTime)]
    pub created_at: DateTimeWithTimeZone,
}

/// The speakers of a transcript and its segments, labeled consistently.
#[derive(Clone, Debug, PartialEq)]
pub struct Labeled {
    /// One entry per distinct speaker, in first-speaking order.
    pub speakers: Vec<Speaker>,
    /// Every segment, ordered by `(start_ms, id)`.
    pub segments: Vec<LabeledSegment>,
}

/// A rendered plain-text transcript and the filename to serve it under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    /// The full plain-text transcript body.
    pub body: String,
    /// The filename to offer the download under.
    pub filename: String,
}

/// What makes two segments the same speaker.
#[derive(Clone, Debug, PartialEq, Eq)]
enum GroupKey<'a> {
    User(Id),
    Account(&'a str),
    Name(&'a str),
    Participant(Id),
    Legacy(&'a str),
}

/// Labels a transcript's speakers and segments from its stored participant attribution.
///
/// The coach and coachee show their profile names; anyone else shows the name they typed
/// in the meeting, or `Guest N` without one. Labels are unique, the coach and coachee
/// keeping theirs unsuffixed. A transcript without participant rows keeps its stored labels.
pub fn label_transcript(
    participants: &[Participant],
    segments: &[Segment],
    coach: &users::Model,
    coachee: &users::Model,
) -> Labeled {
    let by_id: HashMap<Id, &Participant> = participants.iter().map(|p| (p.id, p)).collect();

    let mut ordered: Vec<&Segment> = segments.iter().collect();
    ordered.sort_by_key(|segment| (segment.start_ms, segment.id));

    let keyed: Vec<(&Segment, GroupKey, Option<&Participant>)> = ordered
        .into_iter()
        .map(|segment| {
            let participant = segment
                .participant_id
                .and_then(|id| by_id.get(&id).copied());
            let key = group_key(segment, participant, coach, coachee);
            (segment, key, participant)
        })
        .collect();

    // Distinct groups in first-speaking order with their base labels, and each segment's group.
    let (groups, group_of) = keyed.iter().fold(
        (Vec::<(GroupKey, Option<String>)>::new(), Vec::new()),
        |(mut groups, mut group_of), (_, key, participant)| {
            let index = groups
                .iter()
                .position(|(seen, _)| seen == key)
                .unwrap_or_else(|| {
                    let base = base_label(key, *participant, coach, coachee);
                    groups.push((key.clone(), base));
                    groups.len() - 1
                });
            group_of.push(index);
            (groups, group_of)
        },
    );

    let bases: Vec<String> = groups
        .iter()
        .scan(0, |guests, (_, base)| {
            Some(base.clone().unwrap_or_else(|| {
                *guests += 1;
                format!("Guest {guests}")
            }))
        })
        .collect();
    let keys: Vec<&GroupKey> = groups.iter().map(|(key, _)| key).collect();
    let labels = unique_labels(&keys, &bases);

    let speakers = keys
        .iter()
        .zip(&labels)
        .map(|(key, label)| Speaker {
            label: label.clone(),
            role: attribution(key, coach).1,
        })
        .collect();

    let segments = keyed
        .iter()
        .zip(group_of)
        .map(|((segment, _, _), index)| {
            let (speaker_user_id, speaker_role) = attribution(keys[index], coach);
            LabeledSegment {
                id: segment.id,
                transcription_id: segment.transcription_id,
                speaker_label: labels[index].clone(),
                speaker_user_id,
                speaker_role,
                text: segment.text.clone(),
                start_ms: segment.start_ms,
                end_ms: segment.end_ms,
                confidence: segment.confidence,
                sentiment: segment.sentiment.clone(),
                created_at: segment.created_at,
            }
        })
        .collect();

    Labeled { speakers, segments }
}

/// Which speaker a segment belongs to, strongest identity first.
fn group_key<'a>(
    segment: &'a Segment,
    participant: Option<&'a Participant>,
    coach: &users::Model,
    coachee: &users::Model,
) -> GroupKey<'a> {
    let Some(participant) = participant else {
        return GroupKey::Legacy(&segment.speaker_label);
    };

    participant
        .user_id
        .filter(|id| *id == coach.id || *id == coachee.id)
        .map(GroupKey::User)
        .or_else(|| {
            participant
                .platform_account_id
                .as_deref()
                .map(GroupKey::Account)
        })
        .or_else(|| typed_name(participant).map(GroupKey::Name))
        .unwrap_or(GroupKey::Participant(participant.id))
}

/// A group's label before numbering and de-duplication; `None` when it has no name.
fn base_label(
    key: &GroupKey,
    first_speaker: Option<&Participant>,
    coach: &users::Model,
    coachee: &users::Model,
) -> Option<String> {
    match key {
        GroupKey::User(id) => {
            let user = if *id == coach.id { coach } else { coachee };
            Some(user.preferred_name().into_owned())
        }
        GroupKey::Account(_) => first_speaker.and_then(typed_name).map(str::to_owned),
        GroupKey::Name(name) | GroupKey::Legacy(name) => Some((*name).to_owned()),
        GroupKey::Participant(_) => None,
    }
}

/// The name a participant typed in the meeting, when it is not blank.
fn typed_name(participant: &Participant) -> Option<&str> {
    participant
        .display_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
}

/// The user and role a group speaks for; only coach and coachee groups have them.
fn attribution(key: &GroupKey, coach: &users::Model) -> (Option<Id>, Option<SpeakerRole>) {
    match key {
        GroupKey::User(id) if *id == coach.id => (Some(*id), Some(SpeakerRole::Coach)),
        GroupKey::User(id) => (Some(*id), Some(SpeakerRole::Coachee)),
        _ => (None, None),
    }
}

/// Final labels in group order, suffixing repeats with ` (k)`; coach and coachee claim first.
fn unique_labels(keys: &[&GroupKey], bases: &[String]) -> Vec<String> {
    let is_user = |index: &usize| matches!(keys[*index], GroupKey::User(_));
    let claim_order = (0..keys.len())
        .filter(is_user)
        .chain((0..keys.len()).filter(|index| !is_user(index)));

    let mut labels: Vec<Option<String>> = vec![None; keys.len()];
    for index in claim_order {
        let base = &bases[index];
        let label = iter::once(base.clone())
            .chain((2..).map(|k| format!("{base} ({k})")))
            .find(|candidate| !labels.iter().flatten().any(|taken| taken == candidate))
            .unwrap_or_else(|| base.clone());
        labels[index] = Some(label);
    }

    labels.into_iter().flatten().collect()
}

/// Renders the labeled transcript as the downloadable plain-text file.
///
/// Keeps segments whose role is in `filter` (every segment when it is empty) and drops blank
/// ones; the header lists the speakers with at least one surviving line, in speaking order.
/// Fails when a filtered role has no speaker.
pub fn render_plain_text(
    session_date: NaiveDate,
    labeled: &Labeled,
    filter: &[SpeakerRole],
) -> Result<Rendered, Error> {
    let speakers = &labeled.speakers;
    if let Some(missing) = filter
        .iter()
        .find(|role| !speakers.iter().any(|speaker| speaker.role == Some(**role)))
    {
        return Err(speaker_not_identified(*missing, speakers));
    }

    let selected = |segment: &LabeledSegment| {
        filter.is_empty()
            || segment
                .speaker_role
                .is_some_and(|role| filter.contains(&role))
    };
    let lines: Vec<&LabeledSegment> = labeled
        .segments
        .iter()
        .filter(|segment| selected(segment) && !segment.text.trim().is_empty())
        .collect();

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

    let suffix = if filter.is_empty() {
        String::new()
    } else {
        let names = speakers
            .iter()
            .filter_map(|speaker| {
                let role = speaker.role.filter(|role| filter.contains(role))?;
                Some(slug(&speaker.label).unwrap_or_else(|| role_slug(role).to_owned()))
            })
            .collect::<Vec<_>>()
            .join("-");
        format!("-{names}")
    };

    Ok(Rendered {
        body,
        filename: format!("transcript-{date}{suffix}.txt"),
    })
}

fn role_slug(role: SpeakerRole) -> &'static str {
    match role {
        SpeakerRole::Coach => "coach",
        SpeakerRole::Coachee => "coachee",
    }
}

/// Lowercases a label into filename-safe words joined by single dashes; `None` when nothing remains.
fn slug(label: &str) -> Option<String> {
    let words = label
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    (!words.is_empty()).then(|| words.join("-"))
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
