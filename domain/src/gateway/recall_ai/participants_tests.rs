//! Recall participants survive parsing, and words group by participant rather than by name.

use serde_json::{json, Value};

use super::*;

/// One Recall transcript entry: a participant and the words they spoke, times in seconds.
fn entry(participant: Value, words: &[(&str, f64, f64)]) -> Value {
    json!({
        "participant": participant,
        "language_code": "en",
        "words": words
            .iter()
            .map(|(text, start, end)| json!({
                "text": text,
                "start_timestamp": { "relative": start, "absolute": null },
                "end_timestamp": { "relative": end, "absolute": null },
            }))
            .collect::<Vec<_>>(),
    })
}

fn parse(entries: Vec<Value>) -> Vec<ParticipantEntry> {
    serde_json::from_value(Value::Array(entries)).expect("the fixture matches Recall's shape")
}

fn zoom_coach() -> Value {
    json!({
        "id": 100,
        "name": "J. Hodapp",
        "is_host": true,
        "platform": "desktop",
        "email": null,
        "extra_data": { "zoom": { "os": 2, "guest": false, "user_guid": "GUID-1", "conf_user_id": "conf-coach" } }
    })
}

fn meet_coachee() -> Value {
    json!({
        "id": 200,
        "name": "Caleb",
        "is_host": false,
        "platform": "unknown",
        "email": null,
        "extra_data": { "google_meet": { "static_participant_id": "spaces/abc/devices/1" } }
    })
}

/// Every field Recall reports is kept, and the Zoom account id is lifted out of `extra_data`.
#[test]
fn participants_keep_every_reported_field() {
    let entries = parse(vec![entry(zoom_coach(), &[("Morning", 0.0, 0.4)])]);

    let participants = participants_of(&entries);

    assert_eq!(participants.len(), 1);
    let coach = &participants[0];
    assert_eq!(coach.provider_id, "100");
    assert_eq!(coach.display_name.as_deref(), Some("J. Hodapp"));
    assert_eq!(coach.is_host, Some(true));
    assert_eq!(coach.platform.as_deref(), Some("desktop"));
    assert_eq!(coach.account_id.as_deref(), Some("conf-coach"));
    assert_eq!(
        coach.extra_data,
        Some(
            json!({ "zoom": { "os": 2, "guest": false, "user_guid": "GUID-1", "conf_user_id": "conf-coach" } })
        )
    );
}

/// Meet exposes no stable account id, so none is invented from `static_participant_id`.
#[test]
fn meet_participants_have_no_account_id() {
    let entries = parse(vec![entry(meet_coachee(), &[("Hi", 0.0, 0.3)])]);

    let participants = participants_of(&entries);

    assert_eq!(participants[0].account_id, None);
    assert!(participants[0].extra_data.is_some());
}

/// A participant with no `extra_data` at all still parses, with no account id.
#[test]
fn missing_extra_data_means_no_account_id() {
    let entries = parse(vec![entry(
        json!({ "id": 300, "name": "Dial-in", "is_host": null, "platform": null, "email": null, "extra_data": null }),
        &[("Hello", 0.0, 0.3)],
    )]);

    let participants = participants_of(&entries);

    assert_eq!(participants[0].account_id, None);
    assert_eq!(participants[0].extra_data, None);
}

/// A blank name is no name.
#[test]
fn a_blank_name_becomes_none() {
    let entries = parse(vec![entry(
        json!({ "id": 400, "name": "   ", "is_host": false, "platform": "phone", "email": null, "extra_data": null }),
        &[("Hello", 0.0, 0.3)],
    )]);

    assert_eq!(participants_of(&entries)[0].display_name, None);
}

/// A participant spread across several entries is listed once, in first-appearance order.
#[test]
fn participants_are_listed_once_in_first_appearance_order() {
    let entries = parse(vec![
        entry(meet_coachee(), &[("Hi", 0.0, 0.3)]),
        entry(zoom_coach(), &[("Morning", 1.0, 1.4)]),
        entry(meet_coachee(), &[("Again", 5.0, 5.3)]),
    ]);

    let ids: Vec<String> = participants_of(&entries)
        .into_iter()
        .map(|participant| participant.provider_id)
        .collect();

    assert_eq!(ids, vec!["200".to_string(), "100".to_string()]);
}

/// Two people with the same display name are two speakers, never merged into one segment.
#[test]
fn same_name_different_participants_stay_separate() {
    let first = json!({ "id": 1, "name": "Jim", "is_host": true, "platform": "desktop", "email": null, "extra_data": null });
    let second = json!({ "id": 2, "name": "Jim", "is_host": false, "platform": "desktop", "email": null, "extra_data": null });
    let entries = parse(vec![
        entry(first, &[("Hello", 0.0, 0.4)]),
        entry(second, &[("there", 0.5, 0.9)]),
    ]);

    let segments = coalesce_entries(entries);

    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].participant_id.as_deref(), Some("1"));
    assert_eq!(segments[1].participant_id.as_deref(), Some("2"));
    assert_eq!(segments[0].speaker, "Jim");
    assert_eq!(segments[1].speaker, "Jim");
}

/// Every segment names the participant that spoke it, and keeps the display name as its label.
#[test]
fn segments_carry_their_participant() {
    let entries = parse(vec![
        entry(zoom_coach(), &[("Good", 0.0, 0.3), ("morning", 0.35, 0.8)]),
        entry(meet_coachee(), &[("Hi", 3.0, 3.3)]),
    ]);

    let segments = coalesce_entries(entries);

    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].participant_id.as_deref(), Some("100"));
    assert_eq!(segments[0].speaker, "J. Hodapp");
    assert_eq!(segments[0].text, "Good morning");
    assert_eq!(segments[1].participant_id.as_deref(), Some("200"));
    assert_eq!(segments[1].speaker, "Caleb");
}

/// Without a Recall id, a participant is keyed by name, and a nameless one by a fixed key.
#[test]
fn participants_without_an_id_get_a_stable_key() {
    let named = json!({ "id": null, "name": "Guest", "is_host": null, "platform": null, "email": null, "extra_data": null });
    let nameless = json!({ "id": null, "name": null, "is_host": null, "platform": null, "email": null, "extra_data": null });
    let entries = parse(vec![
        entry(named, &[("Hi", 0.0, 0.3)]),
        entry(nameless, &[("Hello", 3.0, 3.3)]),
    ]);

    let ids: Vec<String> = participants_of(&entries)
        .into_iter()
        .map(|participant| participant.provider_id)
        .collect();
    let segments = coalesce_entries(entries);

    assert_eq!(ids, vec!["name:Guest".to_string(), "unknown".to_string()]);
    assert_eq!(segments[0].participant_id.as_deref(), Some("name:Guest"));
    assert_eq!(segments[1].participant_id.as_deref(), Some("unknown"));
}

/// Every segment's participant appears in the participant list.
#[test]
fn every_segment_participant_is_listed() {
    let entries = parse(vec![
        entry(zoom_coach(), &[("Good", 0.0, 0.3)]),
        entry(meet_coachee(), &[("Hi", 1.0, 1.3)]),
        entry(zoom_coach(), &[("Bye", 9.0, 9.3)]),
    ]);

    let listed: Vec<String> = participants_of(&entries)
        .into_iter()
        .map(|participant| participant.provider_id)
        .collect();
    let segments = coalesce_entries(entries);

    assert!(segments.iter().all(|segment| segment
        .participant_id
        .as_ref()
        .is_some_and(|id| listed.contains(id))));
}
