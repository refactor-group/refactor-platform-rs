//! Finding who attended one Meet conference: the meeting code, the right record, its participants.

use chrono::{DateTime, Duration, TimeZone, Utc};
use mockito::{Matcher, Server};
use serde_json::json;

use super::*;
use crate::error::{DomainErrorKind, ExternalErrorKind};

fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 6, hour, minute, 0).unwrap()
}

fn record(name: &str, start: DateTime<Utc>, end: Option<DateTime<Utc>>) -> ConferenceRecord {
    ConferenceRecord {
        name: name.to_string(),
        start_time: start,
        end_time: end,
    }
}

fn record_json(name: &str, start: DateTime<Utc>, end: Option<DateTime<Utc>>) -> serde_json::Value {
    let mut value = json!({ "name": name, "startTime": start.to_rfc3339(), "space": "spaces/s1" });
    if let Some(end) = end {
        value["endTime"] = json!(end.to_rfc3339());
    }
    value
}

const CODE_FILTER: &str = "space.meeting_code=\"abc-mnop-xyz\"";

#[test]
fn meeting_code_comes_from_a_meet_url() {
    assert_eq!(
        meeting_code_from_url("https://meet.google.com/abc-mnop-xyz").as_deref(),
        Some("abc-mnop-xyz")
    );
    assert_eq!(
        meeting_code_from_url("https://meet.google.com/abc-mnop-xyz?authuser=0").as_deref(),
        Some("abc-mnop-xyz")
    );
    assert_eq!(
        meeting_code_from_url("https://meet.google.com/abc-mnop-xyz/").as_deref(),
        Some("abc-mnop-xyz")
    );
}

#[test]
fn non_meet_urls_have_no_meeting_code() {
    assert_eq!(meeting_code_from_url("https://zoom.us/j/123456"), None);
    assert_eq!(meeting_code_from_url("https://meet.google.com/"), None);
    assert_eq!(meeting_code_from_url("not a url"), None);
}

/// The one record overlapping the recording window is chosen, not an older one in the same space.
#[test]
fn the_record_overlapping_the_recording_is_chosen() {
    let records = vec![
        record(
            "conferenceRecords/last-week",
            at(9, 0) - Duration::days(7),
            Some(at(10, 0) - Duration::days(7)),
        ),
        record("conferenceRecords/today", at(9, 0), Some(at(10, 0))),
    ];

    let chosen = overlapping_record(records, at(9, 5), at(9, 55));

    assert_eq!(
        chosen.map(|r| r.name).as_deref(),
        Some("conferenceRecords/today")
    );
}

/// A conference still marked ongoing (no end time) counts if it started before the recording ended.
#[test]
fn an_ongoing_record_can_overlap() {
    let chosen = overlapping_record(
        vec![record("conferenceRecords/ongoing", at(9, 0), None)],
        at(9, 5),
        at(9, 55),
    );

    assert_eq!(
        chosen.map(|r| r.name).as_deref(),
        Some("conferenceRecords/ongoing")
    );
}

#[test]
fn no_overlapping_record_means_none() {
    let chosen = overlapping_record(
        vec![record(
            "conferenceRecords/earlier",
            at(7, 0),
            Some(at(8, 0)),
        )],
        at(9, 5),
        at(9, 55),
    );

    assert!(chosen.is_none());
}

/// Two candidate records are ambiguous, so neither is trusted.
#[test]
fn two_overlapping_records_mean_none() {
    let chosen = overlapping_record(
        vec![
            record("conferenceRecords/a", at(9, 0), Some(at(9, 30))),
            record("conferenceRecords/b", at(9, 20), Some(at(10, 0))),
        ],
        at(9, 5),
        at(9, 55),
    );

    assert!(chosen.is_none());
}

/// Signed-in, anonymous, and phone participants all come back; only signed-in ones carry a user.
#[tokio::test]
async fn participants_of_the_matching_conference_are_listed() -> Result<(), Error> {
    let mut server = Server::new_async().await;
    let records = server
        .mock("GET", "/conferenceRecords")
        .match_query(Matcher::UrlEncoded("filter".into(), CODE_FILTER.into()))
        .match_header("authorization", "Bearer coach-token")
        .with_status(200)
        .with_body(
            json!({ "conferenceRecords": [
                record_json("conferenceRecords/old", at(9, 0) - Duration::days(7), Some(at(10, 0) - Duration::days(7))),
                record_json("conferenceRecords/today", at(9, 0), Some(at(10, 0))),
            ] })
            .to_string(),
        )
        .create_async()
        .await;
    let participants = server
        .mock("GET", "/conferenceRecords/today/participants")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(
            json!({ "participants": [
                { "name": "conferenceRecords/today/participants/1",
                  "signedinUser": { "user": "users/111", "displayName": "Jim Hodapp" } },
                { "name": "conferenceRecords/today/participants/2",
                  "anonymousUser": { "displayName": "Caleb" } },
                { "name": "conferenceRecords/today/participants/3",
                  "phoneUser": { "displayName": "+1 555-***-**12" } },
            ] })
            .to_string(),
        )
        .create_async()
        .await;

    let client = Client::new("coach-token", &server.url())?;
    let listed = client
        .conference_participants("abc-mnop-xyz", at(9, 5), at(9, 55))
        .await?
        .expect("one conference overlaps the recording");

    records.assert_async().await;
    participants.assert_async().await;
    assert_eq!(
        listed,
        vec![
            Participant {
                user: Some("users/111".to_string()),
                display_name: Some("Jim Hodapp".to_string())
            },
            Participant {
                user: None,
                display_name: Some("Caleb".to_string())
            },
            Participant {
                user: None,
                display_name: Some("+1 555-***-**12".to_string())
            },
        ]
    );
    Ok(())
}

/// Every page of participants is read.
#[tokio::test]
async fn participant_pages_are_followed() -> Result<(), Error> {
    let mut server = Server::new_async().await;
    server
        .mock("GET", "/conferenceRecords")
        .match_query(Matcher::UrlEncoded("filter".into(), CODE_FILTER.into()))
        .with_status(200)
        .with_body(json!({ "conferenceRecords": [record_json("conferenceRecords/today", at(9, 0), Some(at(10, 0)))] }).to_string())
        .create_async()
        .await;
    let second_page = server
        .mock("GET", "/conferenceRecords/today/participants")
        .match_query(Matcher::UrlEncoded("pageToken".into(), "p2".into()))
        .with_status(200)
        .with_body(
            json!({ "participants": [
                { "name": "p/2", "anonymousUser": { "displayName": "Caleb" } },
            ] })
            .to_string(),
        )
        .create_async()
        .await;
    let first_page = server
        .mock("GET", "/conferenceRecords/today/participants")
        .match_query(Matcher::Missing)
        .with_status(200)
        .with_body(
            json!({ "participants": [
                { "name": "p/1", "signedinUser": { "user": "users/111", "displayName": "Jim" } },
            ], "nextPageToken": "p2" })
            .to_string(),
        )
        .create_async()
        .await;

    let client = Client::new("coach-token", &server.url())?;
    let listed = client
        .conference_participants("abc-mnop-xyz", at(9, 5), at(9, 55))
        .await?
        .expect("one conference overlaps the recording");

    first_page.assert_async().await;
    second_page.assert_async().await;
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[1].display_name.as_deref(), Some("Caleb"));
    Ok(())
}

/// Without exactly one overlapping conference, participants are never fetched.
#[tokio::test]
async fn no_matching_conference_means_no_participant_lookup() -> Result<(), Error> {
    let mut server = Server::new_async().await;
    server
        .mock("GET", "/conferenceRecords")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(json!({ "conferenceRecords": [record_json("conferenceRecords/old", at(7, 0), Some(at(8, 0)))] }).to_string())
        .create_async()
        .await;
    let participants = server
        .mock(
            "GET",
            Matcher::Regex("^/conferenceRecords/.+/participants".to_string()),
        )
        .expect(0)
        .create_async()
        .await;

    let client = Client::new("coach-token", &server.url())?;
    let listed = client
        .conference_participants("abc-mnop-xyz", at(9, 5), at(9, 55))
        .await?;

    assert!(listed.is_none());
    participants.assert_async().await;
    Ok(())
}

/// An empty listing (no `conferenceRecords` key at all) is no match, not an error.
#[tokio::test]
async fn an_empty_listing_means_no_match() -> Result<(), Error> {
    let mut server = Server::new_async().await;
    server
        .mock("GET", "/conferenceRecords")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let client = Client::new("coach-token", &server.url())?;

    assert!(client
        .conference_participants("abc-mnop-xyz", at(9, 5), at(9, 55))
        .await?
        .is_none());
    Ok(())
}

/// A rejected token surfaces as a revoked Google grant, as space creation already does.
#[tokio::test]
async fn an_unauthorized_token_is_reported_as_revoked() -> Result<(), Error> {
    let mut server = Server::new_async().await;
    server
        .mock("GET", "/conferenceRecords")
        .match_query(Matcher::Any)
        .with_status(401)
        .create_async()
        .await;

    let client = Client::new("coach-token", &server.url())?;
    let result = client
        .conference_participants("abc-mnop-xyz", at(9, 5), at(9, 55))
        .await;

    assert!(matches!(
        result.map_err(|e| e.error_kind),
        Err(DomainErrorKind::External(ExternalErrorKind::OauthTokenRevoked(provider))) if provider == "google"
    ));
    Ok(())
}
