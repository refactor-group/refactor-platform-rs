//! Throwaway spike (phase 0): prove a spliced VTIMEZONE renders correctly in
//! RFC-strict clients. Emits target/ics-spike/invite.ics for human import-testing.

use chrono::{NaiveDate, TimeZone, Utc};
use chrono_tz::America::New_York;
use icalendar::{Calendar, CalendarDateTime, Component, Event, EventLike, Property};
use std::error::Error;
use std::fs;
use std::path::Path;
use vtimezones_rs::VTIMEZONES;

const TZID: &str = "America/New_York";

fn main() -> Result<(), Box<dyn Error>> {
    // 3:00 PM ET on a date inside US EDT, 60 min duration.
    let start_local = NaiveDate::from_ymd_opt(2026, 9, 15)
        .ok_or("invalid start date")?
        .and_hms_opt(15, 0, 0)
        .ok_or("invalid start time")?;
    let end_local = NaiveDate::from_ymd_opt(2026, 9, 15)
        .ok_or("invalid end date")?
        .and_hms_opt(16, 0, 0)
        .ok_or("invalid end time")?;

    // WithTimezone variant: NaiveDateTime + TZID serializes as DTSTART;TZID=...:local.
    let start = CalendarDateTime::from((start_local, New_York));
    let end = CalendarDateTime::from((end_local, New_York));

    let dtstamp = Utc
        .with_ymd_and_hms(2026, 6, 14, 12, 0, 0)
        .single()
        .ok_or("invalid dtstamp")?
        .format("%Y%m%dT%H%M%SZ")
        .to_string();

    let event = Event::new()
        .uid("ics-spike-0@myrefactor.com")
        .summary("Coaching Session: Spike Org")
        .starts(start)
        .ends(end)
        .add_property("DTSTAMP", dtstamp)
        .add_property("SEQUENCE", "0")
        .append_property(
            Property::new("ORGANIZER", "mailto:coach@example.com")
                .add_parameter("CN", "Coach Example")
                .done(),
        )
        .append_property(
            Property::new("ATTENDEE", "mailto:coachee@example.com")
                .add_parameter("CN", "Coachee Example")
                .add_parameter("RSVP", "TRUE")
                .done(),
        )
        .done();

    let calendar = Calendar::new()
        .append_property(("METHOD", "REQUEST"))
        .push(event)
        .done();

    let serialized = calendar.to_string();

    let vtimezone = VTIMEZONES
        .get(TZID)
        .ok_or("VTIMEZONE block missing for America/New_York")?;

    let spliced = splice_vtimezone(&serialized, vtimezone)?;

    let out_dir = Path::new("target/ics-spike");
    fs::create_dir_all(out_dir)?;
    fs::write(out_dir.join("invite.ics"), &spliced)?;

    println!("{spliced}");
    Ok(())
}

/// Insert the VTIMEZONE block immediately before the first BEGIN:VEVENT,
/// preserving RFC 5545 CRLF line endings throughout.
///
/// vtimezones-rs 0.3 returns a full BEGIN:VCALENDAR-wrapped value, so we extract
/// the inner BEGIN:VTIMEZONE..END:VTIMEZONE span before splicing.
fn splice_vtimezone(serialized: &str, wrapped: &str) -> Result<String, Box<dyn Error>> {
    let block = extract_vtimezone(wrapped)?;

    let marker = "BEGIN:VEVENT";
    let idx = serialized
        .find(marker)
        .ok_or("no BEGIN:VEVENT found in serialized calendar")?;

    let (head, tail) = serialized.split_at(idx);
    Ok(format!("{head}{block}\r\n{tail}"))
}

/// Slice out the BEGIN:VTIMEZONE..END:VTIMEZONE span and normalize it to CRLF.
fn extract_vtimezone(wrapped: &str) -> Result<String, Box<dyn Error>> {
    let begin = wrapped
        .find("BEGIN:VTIMEZONE")
        .ok_or("no BEGIN:VTIMEZONE in vtimezones-rs value")?;
    let end_marker = "END:VTIMEZONE";
    let end = wrapped
        .find(end_marker)
        .ok_or("no END:VTIMEZONE in vtimezones-rs value")?
        + end_marker.len();

    let inner = &wrapped[begin..end];
    Ok(inner.replace("\r\n", "\n").replace('\n', "\r\n"))
}
