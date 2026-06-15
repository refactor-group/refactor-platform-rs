//! Reusable RFC 5545 `.ics` invite builder.
//!
//! Pure: `build` is a function of its inputs with no DB, clock, or network reads.
//! The anchor zone is rendered as a `TZID` plus a spliced `VTIMEZONE` block so
//! RFC-strict clients resolve wall times correctly.

use crate::coaching_session::{Frequency, Recurrence};
use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use chrono::{Duration, NaiveDateTime, TimeZone, Utc, Weekday};
use icalendar::{Calendar, CalendarDateTime, Component, Event, EventLike, Property};
use log::warn;
use vtimezones_rs::VTIMEZONES;

#[cfg(test)]
#[path = "ical_tests.rs"]
mod tests;

/// VCALENDAR-level `METHOD`.
pub enum Method {
    Request,
    Cancel,
}

/// VEVENT `STATUS`.
pub enum EventStatus {
    Confirmed,
    Cancelled,
}

/// Inputs for one calendar invite.
pub struct IcsInvite<'a> {
    pub uid: String,
    pub sequence: i32,
    pub method: Method,
    pub status: EventStatus,
    pub summary: String,
    pub description: String,
    pub anchor_tz: chrono_tz::Tz,
    pub dtstamp: NaiveDateTime,
    pub start: NaiveDateTime,
    pub duration_minutes: i16,
    pub organizer: &'a entity::users::Model,
    pub attendee: &'a entity::users::Model,
    pub location_url: Option<String>,
    pub recurrence: Option<Recurrence>,
}

/// One open action rendered into the DESCRIPTION.
pub struct OpenAction {
    pub body: String,
    pub due_by: Option<NaiveDateTime>,
}

/// Plain inputs for the DESCRIPTION composer, decoupled from entities.
pub struct DescriptionParts<'a> {
    pub session_url: String,
    pub title: Option<&'a str>,
    pub topics: &'a [String],
    pub goal_titles: &'a [String],
    pub open_actions: &'a [OpenAction],
    pub anchor_tz: chrono_tz::Tz,
}

const UTC_BASIC: &str = "%Y%m%dT%H%M%SZ";

fn internal(msg: &str) -> Error {
    Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(msg.to_string())),
    }
}

/// Display name when set, else first and last name.
fn participant_name(u: &entity::users::Model) -> String {
    u.display_name
        .clone()
        .unwrap_or_else(|| format!("{} {}", u.first_name, u.last_name))
}

/// Look up the `VTIMEZONE` block for `tz` and slice out the inner span (CRLF-normalized).
fn vtimezone_block(tz: chrono_tz::Tz) -> Option<String> {
    let wrapped = VTIMEZONES.get(tz.name())?;
    let begin = wrapped.find("BEGIN:VTIMEZONE")?;
    let end_marker = "END:VTIMEZONE";
    let end = wrapped.find(end_marker)? + end_marker.len();
    let inner = &wrapped[begin..end];
    Some(inner.replace("\r\n", "\n").replace('\n', "\r\n"))
}

/// Insert the `VTIMEZONE` block immediately before the first `BEGIN:VEVENT`.
fn splice_vtimezone(serialized: &str, block: &str) -> Result<String, Error> {
    let marker = "BEGIN:VEVENT";
    let idx = serialized
        .find(marker)
        .ok_or_else(|| internal("no BEGIN:VEVENT in serialized calendar"))?;
    let (head, tail) = serialized.split_at(idx);
    Ok(format!("{head}{block}\r\n{tail}"))
}

/// Two-letter RFC 5545 weekday code.
fn weekday_code(wd: Weekday) -> &'static str {
    match wd {
        Weekday::Mon => "MO",
        Weekday::Tue => "TU",
        Weekday::Wed => "WE",
        Weekday::Thu => "TH",
        Weekday::Fri => "FR",
        Weekday::Sat => "SA",
        Weekday::Sun => "SU",
    }
}

/// Build the `RRULE:` value from a recurrence rule, mirroring the series materializer.
fn rrule_value(rule: &Recurrence) -> String {
    let (freq, base_interval) = match rule.frequency {
        Frequency::Daily => ("DAILY", 1),
        Frequency::Weekly => ("WEEKLY", 1),
        Frequency::Biweekly => ("WEEKLY", 2),
        Frequency::Monthly => ("MONTHLY", 1),
    };
    let effective_interval = base_interval * rule.interval;

    let mut parts = vec![format!("FREQ={freq}")];
    if effective_interval > 1 {
        parts.push(format!("INTERVAL={effective_interval}"));
    }
    if let Some(days) = &rule.by_weekdays {
        let codes: Vec<&str> = days.iter().map(|d| weekday_code(*d)).collect();
        parts.push(format!("BYDAY={}", codes.join(",")));
    }
    if let Some(n) = rule.count {
        parts.push(format!("COUNT={n}"));
    } else if let Some(dt) = rule.until {
        parts.push(format!("UNTIL={}", dt.format(UTC_BASIC)));
    }
    parts.join(";")
}

/// Build the full VCALENDAR text (with spliced VTIMEZONE when zoned) for one invite.
pub fn build(invite: &IcsInvite) -> Result<String, Error> {
    let method = match invite.method {
        Method::Request => "REQUEST",
        Method::Cancel => "CANCEL",
    };
    let status = match invite.status {
        EventStatus::Confirmed => "CONFIRMED",
        EventStatus::Cancelled => "CANCELLED",
    };

    let end = invite.start + Duration::minutes(invite.duration_minutes as i64);

    let block = if invite.anchor_tz == chrono_tz::UTC {
        None
    } else {
        vtimezone_block(invite.anchor_tz)
    };

    let mut event = Event::new();
    event
        .uid(&invite.uid)
        .summary(&invite.summary)
        .description(&invite.description)
        .add_property("SEQUENCE", invite.sequence.to_string())
        .add_property("DTSTAMP", invite.dtstamp.format(UTC_BASIC).to_string())
        .add_property("STATUS", status);

    match &block {
        Some(_) => {
            let start_local = Utc
                .from_utc_datetime(&invite.start)
                .with_timezone(&invite.anchor_tz)
                .naive_local();
            let end_local = Utc
                .from_utc_datetime(&end)
                .with_timezone(&invite.anchor_tz)
                .naive_local();
            event.starts(CalendarDateTime::from((start_local, invite.anchor_tz)));
            event.ends(CalendarDateTime::from((end_local, invite.anchor_tz)));
        }
        None => {
            if invite.anchor_tz != chrono_tz::UTC {
                warn!(
                    "no VTIMEZONE block for zone {}, falling back to UTC",
                    invite.anchor_tz.name()
                );
            }
            event.starts(Utc.from_utc_datetime(&invite.start));
            event.ends(Utc.from_utc_datetime(&end));
        }
    }

    event.append_property(
        Property::new("ORGANIZER", format!("mailto:{}", invite.organizer.email))
            .add_parameter("CN", &participant_name(invite.organizer))
            .done(),
    );
    event.append_property(
        Property::new("ATTENDEE", format!("mailto:{}", invite.attendee.email))
            .add_parameter("CN", &participant_name(invite.attendee))
            .add_parameter("ROLE", "REQ-PARTICIPANT")
            .add_parameter("PARTSTAT", "NEEDS-ACTION")
            .add_parameter("RSVP", "TRUE")
            .done(),
    );

    if let Some(url) = &invite.location_url {
        event.add_property("LOCATION", url);
        event.add_property("URL", url);
        event.append_property(
            Property::new("CONFERENCE", url)
                .add_parameter("VALUE", "URI")
                .add_parameter("FEATURE", "VIDEO")
                .add_parameter("LABEL", "Join")
                .done(),
        );
    }

    if let Some(rule) = &invite.recurrence {
        event.add_property("RRULE", rrule_value(rule));
    }

    let event = event.done();
    let calendar = Calendar::new()
        .append_property(("METHOD", method))
        .push(event)
        .done();

    let serialized = calendar.to_string();
    match block {
        Some(block) => splice_vtimezone(&serialized, &block),
        None => Ok(serialized),
    }
}

/// Compose the DESCRIPTION string. Each section is omitted when its data is empty.
pub fn compose_description(parts: &DescriptionParts) -> String {
    let mut sections: Vec<String> = Vec::new();

    if let Some(title) = parts.title.filter(|t| !t.is_empty()) {
        sections.push(title.to_string());
    }

    sections.push(format!("View this session: {}", parts.session_url));

    if !parts.topics.is_empty() {
        let mut block = String::from("Topics to discuss:");
        for topic in parts.topics {
            block.push_str(&format!("\n- {topic}"));
        }
        sections.push(block);
    }

    if !parts.goal_titles.is_empty() {
        let mut block = String::from("Goals you're working toward:");
        for title in parts.goal_titles {
            block.push_str(&format!("\n- {title}"));
        }
        sections.push(block);
    }

    if !parts.open_actions.is_empty() {
        let mut block = String::from("Actions due for this session:");
        for action in parts.open_actions {
            let suffix = match action.due_by {
                Some(due) => {
                    let local = Utc
                        .from_utc_datetime(&due)
                        .with_timezone(&parts.anchor_tz)
                        .format("%b %-d, %Y");
                    format!("(due {local})")
                }
                None => "(no due date)".to_string(),
            };
            block.push_str(&format!("\n- {} {suffix}", action.body));
        }
        sections.push(block);
    }

    sections.join("\n\n")
}
