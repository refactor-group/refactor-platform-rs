//! Decides which transcript speakers are the session's coach and coachee, never guessing.
//!
//! The coach is identified by the meeting host and by the platform's attendee list; when both
//! answer they must agree. The coachee is then found by elimination among the other speakers.
#![cfg_attr(not(test), allow(dead_code))]

use std::collections::BTreeSet;

use meeting_ai::types::transcription::Participant;

use crate::gateway::google_meet;
use crate::transcript_participant::MatchSource;
use crate::Id;

/// What is known about who the coach is, gathered when a transcript completes.
pub(crate) struct Evidence<'a> {
    /// The bot recorded the session's own meeting space; without it nothing is attributed.
    pub recorded_session_space: bool,
    /// The coach's account id on the meeting platform (a bare Google id, not `users/...`).
    pub coach_account_id: Option<&'a str>,
    /// The platform's attendee list for the recorded conference, when it could be fetched.
    pub attendees: Option<&'a [google_meet::Participant]>,
}

/// One speaker's attribution, in the same order as the input speakers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Attribution {
    pub provider_id: String,
    pub user_id: Option<Id>,
    pub source: Option<MatchSource>,
}

/// Identity used to treat several speaker entries as one person.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key<'a> {
    Account(&'a str),
    Name(&'a str),
    Provider(&'a str),
}

/// Speaker positions in the input slice.
type Indexes = BTreeSet<usize>;

/// Attributes each speaker to the coach, the coachee, or nobody.
pub(crate) fn attribute(
    speakers: &[Participant],
    coach_id: Id,
    coachee_id: Id,
    evidence: &Evidence,
) -> Vec<Attribution> {
    let coach = evidence
        .recorded_session_space
        .then(|| coach_answer(speakers, evidence))
        .flatten()
        .unwrap_or_default();
    let coachee = coachee_by_elimination(speakers, &coach);

    speakers
        .iter()
        .enumerate()
        .map(|(i, speaker)| {
            let matched = if coach.contains(&i) {
                Some((coach_id, MatchSource::Account))
            } else if coachee.contains(&i) {
                Some((coachee_id, MatchSource::Elimination))
            } else {
                None
            };
            Attribution {
                provider_id: speaker.provider_id.clone(),
                user_id: matched.map(|(user, _)| user),
                source: matched.map(|(_, source)| source),
            }
        })
        .collect()
}

/// Combines the two answers: both must agree when present, otherwise whichever exists.
fn coach_answer(speakers: &[Participant], evidence: &Evidence) -> Option<Indexes> {
    match (host_answer(speakers), google_answer(speakers, evidence)) {
        (Some(host), Some(google)) => (host == google).then_some(host),
        (host, google) => host.or(google),
    }
}

/// Speakers marked host, accepted only when they are all the same person.
fn host_answer(speakers: &[Participant]) -> Option<Indexes> {
    let hosts: Indexes = indexes_where(speakers, |s| s.is_host == Some(true));
    let people: BTreeSet<Key> = hosts.iter().map(|&i| name_key(&speakers[i])).collect();
    (people.len() == 1).then_some(hosts)
}

/// Speakers whose name matches the coach's signed-in attendee, when that name is unambiguous.
fn google_answer(speakers: &[Participant], evidence: &Evidence) -> Option<Indexes> {
    let account_id = evidence.coach_account_id?;
    let attendees = evidence.attendees?;
    let coach_user = format!("users/{account_id}");

    let coach_attendees: Vec<&google_meet::Participant> = attendees
        .iter()
        .filter(|a| a.user.as_deref() == Some(coach_user.as_str()))
        .collect();
    let [coach_attendee] = coach_attendees[..] else {
        return None;
    };
    let name = non_blank(coach_attendee.display_name.as_deref())?;

    let name_is_unique = attendees
        .iter()
        .filter(|a| a.display_name.as_deref() == Some(name))
        .count()
        == 1;
    let matched = indexes_where(speakers, |s| s.display_name.as_deref() == Some(name));

    (name_is_unique && !matched.is_empty()).then_some(matched)
}

/// Every non-coach speaker, when they are all one person; empty without a coach.
fn coachee_by_elimination(speakers: &[Participant], coach: &Indexes) -> Indexes {
    if coach.is_empty() {
        return Indexes::new();
    }
    let others: Indexes = (0..speakers.len()).filter(|i| !coach.contains(i)).collect();
    let people: BTreeSet<Key> = others
        .iter()
        .map(|&i| elimination_key(&speakers[i]))
        .collect();

    if people.len() == 1 {
        others
    } else {
        Indexes::new()
    }
}

fn indexes_where(speakers: &[Participant], keep: impl Fn(&Participant) -> bool) -> Indexes {
    speakers
        .iter()
        .enumerate()
        .filter(|(_, s)| keep(s))
        .map(|(i, _)| i)
        .collect()
}

fn name_key(speaker: &Participant) -> Key<'_> {
    non_blank(speaker.display_name.as_deref())
        .map(Key::Name)
        .unwrap_or(Key::Provider(&speaker.provider_id))
}

fn elimination_key(speaker: &Participant) -> Key<'_> {
    speaker
        .account_id
        .as_deref()
        .map(Key::Account)
        .unwrap_or_else(|| name_key(speaker))
}

fn non_blank(name: Option<&str>) -> Option<&str> {
    name.filter(|n| !n.trim().is_empty())
}

#[cfg(test)]
#[path = "transcript_attribution_tests.rs"]
mod tests;
