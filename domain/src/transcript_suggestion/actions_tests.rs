//! Verification of the model's action candidates into stored proposals.

use chrono::{NaiveDate, TimeZone, Utc, Weekday};
use serde_json::{json, Value};

use super::*;
use crate::coaching_relationship_kind::{self, Kind};
use crate::transcript_export::LabeledSegment;
use crate::transcript_suggestion::{verify_output, Existing, NextSession, Timezones};

const S2_TEXT: &str = "Um, yeah, I'll schedule a one-on-one with my manager by Friday.";

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

fn segment(role: coaching_relationship_kind::Role, start_ms: i32, text: &str) -> LabeledSegment {
    LabeledSegment {
        id: Id::new_v4(),
        transcription_id: Id::nil(),
        speaker_label: format!("{role:?}"),
        speaker_user_id: None,
        speaker_role: Some(role),
        text: text.to_string(),
        start_ms,
        end_ms: start_ms + 1000,
        confidence: None,
        sentiment: None,
        created_at: Utc::now().fixed_offset(),
    }
}

fn segments() -> Vec<LabeledSegment> {
    vec![
        segment(
            coaching_relationship_kind::Role::Coach,
            1_000,
            "What will you do this week?",
        ),
        segment(coaching_relationship_kind::Role::Coachee, 5_000, S2_TEXT),
        segment(
            coaching_relationship_kind::Role::Coach,
            9_000,
            "Great, and I'll send you the article.",
        ),
    ]
}

/// 10:00 CDT on Wednesday 2026-10-21.
fn next_session_start() -> DateTimeWithTimeZone {
    Utc.with_ymd_and_hms(2026, 10, 21, 15, 0, 0)
        .single()
        .expect("valid time")
        .fixed_offset()
}

fn session(segments: &[LabeledSegment]) -> Session<'_> {
    Session {
        coaching_session_id: Id::new_v4(),
        people: People {
            coach_id: Id::new_v4(),
            coachee_id: Id::new_v4(),
        },
        coach_name: "Jim Hodapp".to_string(),
        coachee_name: "Alex Rivera".to_string(),
        anchor_timezone: chrono_tz::America::Chicago,
        timezones: Timezones {
            coach: chrono_tz::America::Chicago,
            coachee: chrono_tz::America::Chicago,
        },
        due_in_assignee_zone: Kind::Coaching.rules().due_in_assignee_zone(),
        names_people_in_prompt: Kind::Coaching.rules().names_people_in_prompt(),
        start: Utc
            .with_ymd_and_hms(2026, 10, 7, 15, 0, 0)
            .single()
            .expect("valid time")
            .fixed_offset(),
        session_date: day(2026, 10, 7),
        next_session: Some(NextSession {
            date: day(2026, 10, 21),
            start: next_session_start(),
        }),
        week_end: Weekday::Fri,
        segments,
        existing: vec![Existing {
            id: Id::new_v4(),
            text: "Read the article".to_string(),
        }],
    }
}

fn valid() -> Candidate {
    Candidate {
        body: "Schedule a one-on-one".to_string(),
        assignees: vec!["coachee".to_string()],
        due_date: Some("2026-10-09".to_string()),
        due_phrase: Some("by Friday".to_string()),
        source_segment: "s2".to_string(),
        quote: "I'll schedule a one-on-one with my manager".to_string(),
        duplicate_of: None,
    }
}

fn valid_json() -> Value {
    json!({
        "body": "Schedule a one-on-one",
        "assignees": ["coachee"],
        "due_date": "2026-10-09",
        "due_phrase": "by Friday",
        "source_segment": "s2",
        "quote": "I'll schedule a one-on-one with my manager",
        "duplicate_of": null
    })
}

fn verify(candidate: Candidate, session: &Session) -> Option<Verified<ActionProposal>> {
    Actions::verify(candidate, session, &Refs::new(session))
}

fn noon_cdt(d: u32) -> DateTimeWithTimeZone {
    Utc.with_ymd_and_hms(2026, 10, d, 17, 0, 0)
        .single()
        .expect("valid time")
        .fixed_offset()
}

#[test]
fn a_valid_candidate_becomes_a_proposal() {
    let segments = segments();
    let session = session(&segments);
    let verified = verify(valid(), &session).expect("verified");
    let action = &verified.proposal.action.action;

    assert_eq!(action.body.as_deref(), Some("Schedule a one-on-one"));
    assert_eq!(
        verified.proposal.action.assignee_ids,
        vec![session.people.coachee_id]
    );
    assert_eq!(action.due_by, Some(noon_cdt(9)));
    assert_eq!(action.status, Status::NotStarted);
    assert_eq!(action.coaching_session_id, session.coaching_session_id);
    assert_eq!(action.goal_id, None);
    assert_eq!(verified.segment_id, segments[1].id);
    assert_eq!(verified.quote, "I'll schedule a one-on-one with my manager");
    assert_eq!(verified.duplicate_of_id, None);
    assert_eq!(verified.proposal.due_phrase.as_deref(), Some("by Friday"));
}

#[test]
fn an_unknown_source_segment_is_dropped() {
    let segments = segments();
    let candidate = Candidate {
        source_segment: "s9".to_string(),
        ..valid()
    };
    assert!(verify(candidate, &session(&segments)).is_none());
}

#[test]
fn a_quote_not_in_the_segment_falls_back_to_the_segment_text() {
    let segments = segments();
    let candidate = Candidate {
        quote: "I will schedule a one-on-one with my manager".to_string(),
        ..valid()
    };
    let verified = verify(candidate, &session(&segments)).expect("verified");
    assert_eq!(
        verified.quote,
        S2_TEXT.chars().take(200).collect::<String>()
    );
}

#[test]
fn assignees_are_mapped_and_deduplicated_in_order() {
    let segments = segments();
    let session = session(&segments);
    let candidate = Candidate {
        assignees: ["coach", "coachee", "coach"].map(str::to_string).to_vec(),
        ..valid()
    };
    let verified = verify(candidate, &session).expect("verified");
    assert_eq!(
        verified.proposal.action.assignee_ids,
        vec![session.people.coach_id, session.people.coachee_id]
    );
}

#[test]
fn due_dates_are_reconciled_with_the_phrase() {
    let segments = segments();
    let session = session(&segments);
    let due = |due_date: &str, due_phrase: Option<&str>| {
        verify(
            Candidate {
                due_date: Some(due_date.to_string()),
                due_phrase: due_phrase.map(str::to_string),
                ..valid()
            },
            &session,
        )
        .expect("verified")
        .proposal
    };

    assert_eq!(
        due("2026-10-10", Some("by Friday")).action.action.due_by,
        Some(noon_cdt(9))
    );
    let next_friday = due("2026-10-16", Some("next Friday"));
    assert_eq!(next_friday.action.action.due_by, None);
    assert_eq!(next_friday.due_phrase.as_deref(), Some("next Friday"));
    assert_eq!(due("garbage", None).action.action.due_by, None);
}

#[test]
fn duplicate_of_resolves_existing_refs() {
    let segments = segments();
    let session = session(&segments);
    let duplicate = |r: &str| {
        verify(
            Candidate {
                duplicate_of: Some(r.to_string()),
                ..valid()
            },
            &session,
        )
        .expect("verified")
        .duplicate_of_id
    };
    assert_eq!(duplicate("a1"), Some(session.existing[0].id));
    assert_eq!(duplicate("a9"), None);
}

#[test]
fn bodies_must_be_present_and_bounded() {
    let segments = segments();
    let session = session(&segments);
    let with_body = |body: String| verify(Candidate { body, ..valid() }, &session);
    assert!(with_body(String::new()).is_none());
    assert!(with_body("   \n ".to_string()).is_none());
    assert!(with_body("x".repeat(2001)).is_none());
    assert!(with_body("x".repeat(2000)).is_some());
}

#[test]
fn verify_output_keeps_at_most_fifty_valid_items_in_order() {
    let segments = segments();
    let session = session(&segments);
    let refs = Refs::new(&session);
    let bodies_kept_from = |count: usize| {
        let items: Vec<Value> = (1..=count)
            .map(|i| {
                let mut item = valid_json();
                item["body"] = json!(format!("Action {i}"));
                item
            })
            .collect();
        verify_output::<Actions>(&json!({ "suggestions": items }), &session, &refs)
            .expect("verified")
            .into_iter()
            .filter_map(|v| v.proposal.action.action.body)
            .collect::<Vec<_>>()
    };
    let numbered = |count: usize| {
        (1..=count)
            .map(|i| format!("Action {i}"))
            .collect::<Vec<_>>()
    };

    assert_eq!(bodies_kept_from(24), numbered(24));
    assert_eq!(bodies_kept_from(52), numbered(50));
}

#[test]
fn verify_output_skips_items_that_do_not_parse() {
    let segments = segments();
    let session = session(&segments);
    let refs = Refs::new(&session);
    let mut missing_quote = valid_json();
    missing_quote
        .as_object_mut()
        .expect("object")
        .remove("quote");

    let output = json!({ "suggestions": [missing_quote, valid_json(), valid_json()] });
    let verified = verify_output::<Actions>(&output, &session, &refs).expect("verified");
    assert_eq!(verified.len(), 2);
}

#[test]
fn verify_output_rejects_an_answer_without_a_suggestions_array() {
    let segments = segments();
    let session = session(&segments);
    let refs = Refs::new(&session);
    assert!(verify_output::<Actions>(&json!({ "items": [] }), &session, &refs).is_err());
    assert!(verify_output::<Actions>(&json!([]), &session, &refs).is_err());
}

#[test]
fn a_proposal_round_trips_and_reads_as_an_action() {
    let segments = segments();
    let session = session(&segments);
    let proposal = verify(valid(), &session).expect("verified").proposal;

    let value = serde_json::to_value(&proposal).expect("serialize");
    let back: ActionProposal = serde_json::from_value(value.clone()).expect("proposal");
    assert_eq!(back.action.action.body, proposal.action.action.body);
    assert_eq!(back.action.assignee_ids, proposal.action.assignee_ids);
    assert_eq!(back.action.action.due_by, proposal.action.action.due_by);
    assert_eq!(back.due_phrase, proposal.due_phrase);

    let action: Model = serde_json::from_value(value).expect("action");
    assert_eq!(action.body, proposal.action.action.body);
    assert_eq!(action.due_by, proposal.action.action.due_by);
    assert_eq!(action.coaching_session_id, session.coaching_session_id);
}

#[test]
fn check_rejects_outsiders_and_blank_bodies() {
    let segments = segments();
    let session = session(&segments);
    let proposal = verify(valid(), &session).expect("verified").proposal;
    assert!(Actions::check(&proposal, session.people).is_ok());
    assert_eq!(Actions::assignees(&proposal), [session.people.coachee_id]);

    let mut outsider = proposal.clone();
    outsider.action.assignee_ids.push(Id::new_v4());
    assert!(Actions::check(&outsider, session.people).is_err());

    let mut blank = proposal;
    blank.action.action.body = Some("  ".to_string());
    assert!(Actions::check(&blank, session.people).is_err());
}

#[test]
fn the_output_schema_is_strict() {
    let segments = segments();
    let session = session(&segments);
    let schema = Actions::output_schema(&session);
    let items = &schema["properties"]["suggestions"]["items"];
    assert_eq!(
        items["required"],
        json!([
            "body",
            "assignees",
            "due_date",
            "due_phrase",
            "source_segment",
            "quote",
            "duplicate_of"
        ])
    );
    assert_eq!(schema["additionalProperties"], json!(false));
    assert_eq!(items["additionalProperties"], json!(false));
    assert_eq!(Actions::schema_name(), "action_suggestions");
    assert_eq!(Actions::TARGET, Target::Action);
    assert!(!Actions::instructions(&session).is_empty());
}

#[test]
fn a_deadline_on_the_session_day_is_due_at_the_end_of_that_day() {
    let segments = segments();
    let session = session(&segments);
    let candidate = Candidate {
        due_date: Some("2026-10-07".to_string()),
        due_phrase: Some("after our call".to_string()),
        ..valid()
    };

    let verified = verify(candidate, &session).expect("verified");

    let end_of_day_cdt = Utc
        .with_ymd_and_hms(2026, 10, 8, 4, 59, 0)
        .single()
        .expect("valid time")
        .fixed_offset();
    assert_eq!(verified.proposal.action.action.due_by, Some(end_of_day_cdt));
}

#[test]
fn a_deadline_of_the_next_session_is_due_when_it_starts() {
    let segments = segments();
    let session = session(&segments);
    for phrase in ["before our next session", "by next time"] {
        let candidate = Candidate {
            due_date: Some("2026-10-21".to_string()),
            due_phrase: Some(phrase.to_string()),
            ..valid()
        };

        let verified = verify(candidate, &session).expect("verified");

        assert_eq!(
            verified.proposal.action.action.due_by,
            Some(next_session_start()),
            "{phrase}"
        );
    }
}

#[test]
fn the_next_sessions_day_said_another_way_stays_at_noon() {
    let segments = segments();
    let session = session(&segments);
    let candidate = Candidate {
        due_date: Some("2026-10-21".to_string()),
        due_phrase: Some("October 21st".to_string()),
        ..valid()
    };

    let verified = verify(candidate, &session).expect("verified");

    assert_eq!(verified.proposal.action.action.due_by, Some(noon_cdt(21)));
}

#[test]
fn a_deadline_on_a_later_day_stays_at_noon() {
    let segments = segments();
    let session = session(&segments);

    let verified = verify(valid(), &session).expect("verified");

    assert_eq!(verified.proposal.action.action.due_by, Some(noon_cdt(9)));
}

/// A colleague session scheduled by A, in the coachee seat in `a_tz`, starting at `start`; B holds
/// the coach seat in `b_tz`.
fn colleague_session(
    segments: &[LabeledSegment],
    start: DateTimeWithTimeZone,
    a_tz: Tz,
    b_tz: Tz,
) -> Session<'_> {
    Session {
        anchor_timezone: a_tz,
        timezones: Timezones {
            coach: b_tz,
            coachee: a_tz,
        },
        due_in_assignee_zone: Kind::Colleague.rules().due_in_assignee_zone(),
        start,
        session_date: start.with_timezone(&a_tz).date_naive(),
        next_session: None,
        ..session(segments)
    }
}

const A: &str = "coachee";
const B: &str = "coach";

fn local(tz: Tz, y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTimeWithTimeZone {
    tz.with_ymd_and_hms(y, m, d, h, min, 0)
        .single()
        .expect("valid time")
        .fixed_offset()
}

fn due_by(session: &Session, assignees: Vec<&str>, due_date: &str) -> Option<DateTimeWithTimeZone> {
    let candidate = Candidate {
        assignees: assignees.into_iter().map(str::to_string).collect(),
        due_date: Some(due_date.to_string()),
        due_phrase: None,
        ..valid()
    };
    verify(candidate, session)
        .expect("verified")
        .proposal
        .action
        .action
        .due_by
}

fn chicago_and_tokyo(segments: &[LabeledSegment]) -> Session<'_> {
    colleague_session(
        segments,
        local(chrono_tz::America::Chicago, 2026, 11, 18, 10, 0),
        chrono_tz::America::Chicago,
        chrono_tz::Asia::Tokyo,
    )
}

#[test]
fn a_colleague_suggestion_for_the_other_person_is_due_in_their_timezone() {
    let segments = segments();
    let session = chicago_and_tokyo(&segments);

    assert_eq!(
        due_by(&session, vec![B], "2026-11-25"),
        Some(local(chrono_tz::Asia::Tokyo, 2026, 11, 25, 12, 0))
    );
}

#[test]
fn a_colleague_suggestion_for_the_scheduler_is_due_in_their_timezone() {
    let segments = segments();
    let session = chicago_and_tokyo(&segments);

    assert_eq!(
        due_by(&session, vec![A], "2026-11-25"),
        Some(local(chrono_tz::America::Chicago, 2026, 11, 25, 12, 0))
    );
}

#[test]
fn a_colleague_suggestion_for_both_or_nobody_is_due_in_the_schedulers_timezone() {
    let segments = segments();
    let session = chicago_and_tokyo(&segments);
    let scheduler_noon = Some(local(chrono_tz::America::Chicago, 2026, 11, 25, 12, 0));

    assert_eq!(due_by(&session, vec![A, B], "2026-11-25"), scheduler_noon);
    assert_eq!(due_by(&session, vec![], "2026-11-25"), scheduler_noon);
}

#[test]
fn the_session_day_is_judged_in_the_assignees_timezone() {
    let segments = segments();
    let session = colleague_session(
        &segments,
        local(chrono_tz::America::Chicago, 2026, 11, 24, 23, 30),
        chrono_tz::America::Chicago,
        chrono_tz::Asia::Tokyo,
    );

    assert_eq!(
        due_by(&session, vec![B], "2026-11-25"),
        Some(local(chrono_tz::Asia::Tokyo, 2026, 11, 25, 23, 59))
    );
}

#[test]
fn a_deadline_after_both_zones_leave_summer_time_uses_each_assignees_winter_offset() {
    let segments = segments();
    let session = colleague_session(
        &segments,
        local(chrono_tz::America::New_York, 2026, 10, 29, 10, 0),
        chrono_tz::America::New_York,
        chrono_tz::Europe::London,
    );

    let for_b = due_by(&session, vec![B], "2026-11-02").expect("due");
    assert_eq!(for_b, local(chrono_tz::Europe::London, 2026, 11, 2, 12, 0));
    assert_eq!(for_b.offset().local_minus_utc(), 0);

    let for_a = due_by(&session, vec![A], "2026-11-02").expect("due");
    assert_eq!(
        for_a,
        local(chrono_tz::America::New_York, 2026, 11, 2, 12, 0)
    );
    assert_eq!(for_a.offset().local_minus_utc(), -5 * 3600);
}

#[test]
fn a_coaching_suggestion_for_the_coachee_stays_in_the_coachs_timezone() {
    let segments = segments();
    let session = Session {
        timezones: Timezones {
            coach: chrono_tz::America::Chicago,
            coachee: chrono_tz::Asia::Tokyo,
        },
        ..session(&segments)
    };

    assert_eq!(
        due_by(&session, vec!["coachee"], "2026-10-09"),
        Some(noon_cdt(9))
    );
}

/// A colleague session between Alice Smith in the coach seat and Bob Lee in the coachee seat.
fn alice_and_bob(segments: &[LabeledSegment]) -> Session<'_> {
    let rules = Kind::Colleague.rules();
    Session {
        coach_name: "Alice Smith".to_string(),
        coachee_name: "Bob Lee".to_string(),
        due_in_assignee_zone: rules.due_in_assignee_zone(),
        names_people_in_prompt: rules.names_people_in_prompt(),
        ..session(segments)
    }
}

fn assignee_ids(session: &Session, assignees: &[&str]) -> Option<Vec<Id>> {
    let candidate = Candidate {
        assignees: assignees.iter().map(|a| a.to_string()).collect(),
        ..valid()
    };
    verify(candidate, session).map(|v| v.proposal.action.assignee_ids)
}

#[test]
fn a_coaching_session_gets_the_coaching_prompt_unchanged() {
    let segments = segments();
    assert_eq!(
        Actions::instructions(&session(&segments)),
        include_str!("actions_prompt.md")
    );
}

#[test]
fn each_coaching_passage_the_colleague_prompt_replaces_occurs_once() {
    let prompt = include_str!("actions_prompt.md");
    for (coaching, _) in COLLEAGUE_PASSAGES {
        assert_eq!(prompt.matches(coaching).count(), 1, "{coaching}");
    }
}

#[test]
fn a_colleague_prompt_names_the_two_people_and_never_says_coach() {
    let segments = segments();
    let prompt = Actions::instructions(&alice_and_bob(&segments));

    for passage in [
        "You read the transcript of a session between two colleagues, Alice Smith and Bob Lee, and list the actions that\neither of them committed to doing after the session.",
        "- Alice Smith or Bob Lee says they will do something after the session:",
        "- Agreements about how the two of them work together",
        "- Scheduling the next session or meeting, and anything about the recording or the meeting tool.",
        "- Commitments by anyone other than Alice Smith and Bob Lee (other attendees, managers, other colleagues).",
        "- assignees: [\"Alice Smith\"], [\"Bob Lee\"], or [\"Alice Smith\", \"Bob Lee\"] when they both committed to the same thing.",
    ] {
        assert!(prompt.contains(passage), "{passage}");
    }
    assert!(!prompt.to_lowercase().contains("coach"));
}

#[test]
fn the_assignee_enum_names_the_people_for_colleagues_and_the_roles_for_coaching() {
    let segments = segments();
    let assignee_enum = |session: &Session| {
        Actions::output_schema(session)["properties"]["suggestions"]["items"]["properties"]
            ["assignees"]["items"]["enum"]
            .clone()
    };

    assert_eq!(
        assignee_enum(&alice_and_bob(&segments)),
        json!(["Alice Smith", "Bob Lee"])
    );
    assert_eq!(
        assignee_enum(&session(&segments)),
        json!(["coach", "coachee"])
    );
}

#[test]
fn colleague_assignees_are_named_people_and_coaching_assignees_are_roles() {
    let segments = segments();
    let colleagues = alice_and_bob(&segments);
    let (alice, bob) = (colleagues.people.coach_id, colleagues.people.coachee_id);

    assert_eq!(assignee_ids(&colleagues, &["Bob Lee"]), Some(vec![bob]));
    assert_eq!(
        assignee_ids(&colleagues, &["Alice Smith", "Bob Lee"]),
        Some(vec![alice, bob])
    );
    assert_eq!(assignee_ids(&colleagues, &["coach"]), None);

    let coaching = session(&segments);
    assert_eq!(assignee_ids(&coaching, &["Alice Smith"]), None);
    assert_eq!(
        assignee_ids(&coaching, &["coachee"]),
        Some(vec![coaching.people.coachee_id])
    );
}
