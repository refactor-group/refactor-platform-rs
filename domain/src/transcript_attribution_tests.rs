//! Frozen acceptance tests for attributing meeting participants to the coach and coachee.
//! Written by the overseer before implementation; read-only during the build.

use chrono::Utc;
use meeting_ai::types::transcription::Participant;

use super::*;
use crate::coaching_relationship_kind::Kind;
use crate::gateway::google_meet::Participant as Attendee;
use crate::transcript_participant::MatchSource;
use crate::Id;

const COACH_GOOGLE_ID: &str = "111111111111111111111";
const COACHEE_GOOGLE_ID: &str = "222222222222222222222";

fn speaker(provider_id: &str, name: Option<&str>, is_host: Option<bool>) -> Participant {
    Participant {
        provider_id: provider_id.to_string(),
        display_name: name.map(str::to_string),
        is_host,
        platform: Some("desktop".to_string()),
        account_id: None,
        extra_data: None,
    }
}

fn signed_in(google_id: &str, name: &str) -> Attendee {
    Attendee {
        user: Some(format!("users/{google_id}")),
        display_name: Some(name.to_string()),
    }
}

fn anonymous(name: &str) -> Attendee {
    Attendee {
        user: None,
        display_name: Some(name.to_string()),
    }
}

struct People {
    coach_id: Id,
    coachee_id: Id,
}

fn people() -> People {
    People {
        coach_id: Id::new_v4(),
        coachee_id: Id::new_v4(),
    }
}

/// The captured production shape: the coach hosts, the coachee does not, the bot is an
/// anonymous attendee that never speaks, and both people are signed in to Google.
fn typical_attendees() -> Vec<Attendee> {
    vec![
        anonymous("Refactor Coach"),
        signed_in(COACH_GOOGLE_ID, "Coach Person"),
        signed_in(COACHEE_GOOGLE_ID, "Coachee Person"),
    ]
}

fn typical_speakers() -> Vec<Participant> {
    vec![
        speaker("100", Some("Coachee Person"), Some(false)),
        speaker("200", Some("Coach Person"), Some(true)),
    ]
}

fn evidence(attendees: Option<&[Attendee]>) -> Evidence<'_> {
    Evidence {
        recorded_session_space: true,
        anchor_account_id: Some(COACH_GOOGLE_ID),
        attendees,
    }
}

/// `(provider_id, user, source)` per input participant, in input order.
fn outcome(attributions: &[Attribution]) -> Vec<(&str, Option<Id>, Option<MatchSource>)> {
    attributions
        .iter()
        .map(|a| (a.provider_id.as_str(), a.user_id, a.source))
        .collect()
}

#[test]
fn both_answers_agreeing_identify_the_coach_and_then_the_coachee() {
    let p = people();
    let attendees = typical_attendees();

    let result = attribute(
        &typical_speakers(),
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(Some(&attendees)),
    );

    assert_eq!(
        outcome(&result),
        vec![
            ("100", Some(p.coachee_id), Some(MatchSource::Elimination)),
            ("200", Some(p.coach_id), Some(MatchSource::Account)),
        ]
    );
}

#[test]
fn the_host_alone_identifies_the_coach_when_google_cannot_answer() {
    let p = people();

    let result = attribute(
        &typical_speakers(),
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(None),
    );

    assert_eq!(
        outcome(&result),
        vec![
            ("100", Some(p.coachee_id), Some(MatchSource::Elimination)),
            ("200", Some(p.coach_id), Some(MatchSource::Account)),
        ]
    );
}

#[test]
fn google_alone_identifies_the_coach_when_there_is_no_single_host() {
    let p = people();
    let attendees = typical_attendees();
    let speakers = vec![
        speaker("100", Some("Coachee Person"), None),
        speaker("200", Some("Coach Person"), None),
    ];

    let result = attribute(
        &speakers,
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(Some(&attendees)),
    );

    assert_eq!(
        outcome(&result),
        vec![
            ("100", Some(p.coachee_id), Some(MatchSource::Elimination)),
            ("200", Some(p.coach_id), Some(MatchSource::Account)),
        ]
    );
}

#[test]
fn answers_that_disagree_identify_nobody() {
    let p = people();
    let attendees = typical_attendees();
    // The coachee holds host while Google places the coach's account on the other speaker.
    let speakers = vec![
        speaker("100", Some("Coachee Person"), Some(true)),
        speaker("200", Some("Coach Person"), Some(false)),
    ];

    let result = attribute(
        &speakers,
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(Some(&attendees)),
    );

    assert!(result
        .iter()
        .all(|a| a.user_id.is_none() && a.source.is_none()));
}

#[test]
fn a_recording_of_some_other_meeting_identifies_nobody() {
    let p = people();
    let attendees = typical_attendees();
    let other_meeting = Evidence {
        recorded_session_space: false,
        anchor_account_id: Some(COACH_GOOGLE_ID),
        attendees: Some(&attendees),
    };

    let result = attribute(
        &typical_speakers(),
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &other_meeting,
    );

    assert!(result.iter().all(|a| a.user_id.is_none()));
}

#[test]
fn two_hosts_leave_the_decision_to_google() {
    let p = people();
    let attendees = typical_attendees();
    let speakers = vec![
        speaker("100", Some("Coachee Person"), Some(true)),
        speaker("200", Some("Coach Person"), Some(true)),
    ];

    let result = attribute(
        &speakers,
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(Some(&attendees)),
    );

    assert_eq!(result[1].user_id, Some(p.coach_id));
    assert_eq!(result[0].user_id, Some(p.coachee_id));
}

#[test]
fn two_hosts_and_no_google_answer_identify_nobody() {
    let p = people();
    let speakers = vec![
        speaker("100", Some("Coachee Person"), Some(true)),
        speaker("200", Some("Coach Person"), Some(true)),
    ];

    let result = attribute(
        &speakers,
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(None),
    );

    assert!(result.iter().all(|a| a.user_id.is_none()));
}

#[test]
fn a_coach_on_another_account_without_host_identifies_nobody() {
    let p = people();
    let attendees = vec![
        anonymous("Refactor Coach"),
        signed_in("333333333333333333333", "Coach Person"),
        anonymous("Coachee Person"),
    ];
    let speakers = vec![
        speaker("100", Some("Coachee Person"), Some(false)),
        speaker("200", Some("Coach Person"), Some(false)),
    ];

    let result = attribute(
        &speakers,
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(Some(&attendees)),
    );

    assert!(result.iter().all(|a| a.user_id.is_none()));
}

#[test]
fn a_guest_using_the_coachs_name_cannot_be_the_google_answer_but_the_host_still_is() {
    let p = people();
    let attendees = vec![
        anonymous("Refactor Coach"),
        signed_in(COACH_GOOGLE_ID, "Coach Person"),
        anonymous("Coach Person"),
        anonymous("Coachee Person"),
    ];
    let speakers = vec![
        speaker("100", Some("Coachee Person"), Some(false)),
        speaker("200", Some("Coach Person"), Some(true)),
        speaker("300", Some("Coach Person"), Some(false)),
    ];

    let result = attribute(
        &speakers,
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(Some(&attendees)),
    );

    assert_eq!(result[1].user_id, Some(p.coach_id));
    assert_eq!(result[2].user_id, None, "the impostor is never the coach");
    assert_eq!(
        result[0].user_id, None,
        "two other speakers leave the coachee unknown"
    );
}

#[test]
fn without_the_coachs_google_id_only_the_host_answers() {
    let p = people();
    let attendees = typical_attendees();
    let no_account = Evidence {
        recorded_session_space: true,
        anchor_account_id: None,
        attendees: Some(&attendees),
    };

    let result = attribute(
        &typical_speakers(),
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &no_account,
    );

    assert_eq!(result[1].user_id, Some(p.coach_id));
}

#[test]
fn a_coach_who_rejoined_is_the_coach_both_times() {
    let p = people();
    let attendees = typical_attendees();
    let speakers = vec![
        speaker("100", Some("Coachee Person"), Some(false)),
        speaker("200", Some("Coach Person"), Some(true)),
        speaker("201", Some("Coach Person"), Some(true)),
    ];

    let result = attribute(
        &speakers,
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(Some(&attendees)),
    );

    assert_eq!(
        outcome(&result),
        vec![
            ("100", Some(p.coachee_id), Some(MatchSource::Elimination)),
            ("200", Some(p.coach_id), Some(MatchSource::Account)),
            ("201", Some(p.coach_id), Some(MatchSource::Account)),
        ]
    );
}

#[test]
fn a_coachee_who_rejoined_is_the_coachee_both_times() {
    let p = people();
    let speakers = vec![
        speaker("100", Some("Coachee Person"), Some(false)),
        speaker("200", Some("Coach Person"), Some(true)),
        speaker("101", Some("Coachee Person"), Some(false)),
    ];

    let result = attribute(
        &speakers,
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(None),
    );

    assert_eq!(result[0].user_id, Some(p.coachee_id));
    assert_eq!(result[2].user_id, Some(p.coachee_id));
}

#[test]
fn a_third_speaker_leaves_the_coachee_unknown() {
    let p = people();
    let speakers = vec![
        speaker("100", Some("Coachee Person"), Some(false)),
        speaker("200", Some("Coach Person"), Some(true)),
        speaker("300", Some("A Guest"), Some(false)),
    ];

    let result = attribute(
        &speakers,
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(None),
    );

    assert_eq!(result[1].user_id, Some(p.coach_id));
    assert_eq!(result[0].user_id, None);
    assert_eq!(result[2].user_id, None);
}

#[test]
fn a_coach_speaking_alone_leaves_no_coachee() {
    let p = people();
    let speakers = vec![speaker("200", Some("Coach Person"), Some(true))];

    let result = attribute(
        &speakers,
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(None),
    );

    assert_eq!(
        outcome(&result),
        vec![("200", Some(p.coach_id), Some(MatchSource::Account))]
    );
}

#[test]
fn without_a_coach_there_is_no_elimination() {
    let p = people();
    let speakers = vec![
        speaker("100", Some("Coachee Person"), Some(false)),
        speaker("200", Some("Coach Person"), Some(false)),
    ];

    let result = attribute(
        &speakers,
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(None),
    );

    assert!(result.iter().all(|a| a.user_id.is_none()));
}

#[test]
fn a_nameless_single_host_is_still_the_host_answer() {
    let p = people();
    let speakers = vec![
        speaker("100", Some("Coachee Person"), Some(false)),
        speaker("200", None, Some(true)),
    ];

    let result = attribute(
        &speakers,
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(None),
    );

    assert_eq!(result[1].user_id, Some(p.coach_id));
    assert_eq!(result[0].user_id, Some(p.coachee_id));
}

#[test]
fn elimination_groups_other_speakers_by_account_before_name() {
    let p = people();
    let mut same_account_renamed = speaker("101", Some("Renamed"), Some(false));
    same_account_renamed.account_id = Some("acct-1".to_string());
    let mut first = speaker("100", Some("Coachee Person"), Some(false));
    first.account_id = Some("acct-1".to_string());
    let speakers = vec![
        first,
        speaker("200", Some("Coach Person"), Some(true)),
        same_account_renamed,
    ];

    let result = attribute(
        &speakers,
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(None),
    );

    assert_eq!(result[0].user_id, Some(p.coachee_id));
    assert_eq!(result[2].user_id, Some(p.coachee_id));
}

#[test]
fn no_speakers_attribute_nothing() {
    let p = people();

    assert!(attribute(
        &[],
        p.coach_id,
        p.coachee_id,
        Kind::Coaching,
        &evidence(None)
    )
    .is_empty());
}

fn relationship(kind: Kind) -> coaching_relationships::Model {
    let now = Utc::now();
    coaching_relationships::Model {
        id: Id::new_v4(),
        organization_id: Id::new_v4(),
        coach_id: Id::new_v4(),
        coachee_id: Id::new_v4(),
        kind,
        slug: "pair".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

#[test]
fn a_colleague_who_made_the_link_from_the_coachee_seat_is_the_anchor() {
    let r = relationship(Kind::Colleague);

    assert_eq!(
        anchor_and_other(&r, Some(r.coachee_id)),
        Some((r.coachee_id, r.coach_id))
    );
}

#[test]
fn a_colleague_session_without_a_link_maker_has_no_anchor() {
    assert_eq!(anchor_and_other(&relationship(Kind::Colleague), None), None);
}

#[test]
fn a_colleague_session_whose_link_maker_is_not_a_participant_has_no_anchor() {
    assert_eq!(
        anchor_and_other(&relationship(Kind::Colleague), Some(Id::new_v4())),
        None
    );
}

#[test]
fn a_coaching_session_without_a_link_maker_anchors_on_the_coach() {
    let r = relationship(Kind::Coaching);

    assert_eq!(anchor_and_other(&r, None), Some((r.coach_id, r.coachee_id)));
}

#[test]
fn a_coaching_session_whose_coachee_made_the_link_anchors_on_the_coach() {
    let r = relationship(Kind::Coaching);

    assert_eq!(
        anchor_and_other(&r, Some(r.coachee_id)),
        Some((r.coach_id, r.coachee_id))
    );
}

/// Attributes `speakers` in a `kind` session whose link the coachee seat made.
fn attribute_linked_by_coachee(
    kind: Kind,
    speakers: &[Participant],
    evidence: &Evidence,
) -> (coaching_relationships::Model, Vec<Attribution>) {
    let r = relationship(kind);
    let (anchor, other) = anchor_and_other(&r, Some(r.coachee_id)).expect("an anchor");
    let result = attribute(speakers, anchor, other, kind, evidence);
    (r, result)
}

#[test]
fn a_coaching_session_whose_coachee_made_the_link_attributes_the_hosting_coach() {
    let (r, result) =
        attribute_linked_by_coachee(Kind::Coaching, &typical_speakers(), &evidence(None));

    assert_eq!(
        outcome(&result),
        vec![
            ("100", Some(r.coachee_id), Some(MatchSource::Elimination)),
            ("200", Some(r.coach_id), Some(MatchSource::Account)),
        ]
    );
}

/// Evidence naming the coachee-seat link maker's account, with `typical_attendees()`.
fn maker_evidence(attendees: Option<&[Attendee]>) -> Evidence<'_> {
    Evidence {
        anchor_account_id: Some(COACHEE_GOOGLE_ID),
        ..evidence(attendees)
    }
}

#[test]
fn a_colleague_session_with_only_a_host_answer_attributes_nobody() {
    let speakers = vec![
        speaker("100", Some("Coach Person"), Some(false)),
        speaker("200", Some("Coachee Person"), Some(true)),
    ];

    let (_, result) =
        attribute_linked_by_coachee(Kind::Colleague, &speakers, &maker_evidence(None));

    assert!(result
        .iter()
        .all(|a| a.user_id.is_none() && a.source.is_none()));
}

#[test]
fn a_colleague_link_makers_matched_account_attributes_them_and_the_other_by_elimination() {
    let attendees = typical_attendees();
    let speakers = vec![
        speaker("100", Some("Coach Person"), None),
        speaker("200", Some("Coachee Person"), None),
    ];

    let (r, result) = attribute_linked_by_coachee(
        Kind::Colleague,
        &speakers,
        &maker_evidence(Some(&attendees)),
    );

    assert_eq!(
        outcome(&result),
        vec![
            ("100", Some(r.coach_id), Some(MatchSource::Elimination)),
            ("200", Some(r.coachee_id), Some(MatchSource::Account)),
        ]
    );
}

#[test]
fn a_colleague_host_who_disagrees_with_the_account_attributes_nobody() {
    let attendees = typical_attendees();
    let speakers = vec![
        speaker("100", Some("Coach Person"), Some(true)),
        speaker("200", Some("Coachee Person"), Some(false)),
    ];

    let (_, result) = attribute_linked_by_coachee(
        Kind::Colleague,
        &speakers,
        &maker_evidence(Some(&attendees)),
    );

    assert!(result
        .iter()
        .all(|a| a.user_id.is_none() && a.source.is_none()));
}
