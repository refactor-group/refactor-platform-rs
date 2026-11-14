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

#[cfg(test)]
#[path = "ical_oracle_tests.rs"]
mod oracle_tests;

/// VCALENDAR-level `METHOD`.
#[derive(PartialEq, Eq)]
pub enum Method {
    Request,
    Cancel,
}

/// VEVENT `STATUS`.
pub enum EventStatus {
    Confirmed,
    Cancelled,
}

/// One named calendar identity, either a user or the platform itself.
pub struct Participant<'a> {
    pub name: String,
    pub email: &'a str,
}

impl<'a> Participant<'a> {
    /// A participant with an explicit display name and address.
    pub fn new(name: &str, email: &'a str) -> Self {
        Self {
            name: name.to_string(),
            email,
        }
    }
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
    pub organizer: Participant<'a>,
    pub attendees: Vec<Participant<'a>>,
    pub location_url: Option<String>,
    pub recurrence: Option<Recurrence>,
    /// RFC 5545 `RECURRENCE-ID`: the original start of the occurrence this invite
    /// overrides. Mutually exclusive with `recurrence`.
    pub recurrence_id: Option<NaiveDateTime>,
    /// RFC 5545 `EXDATE`s (UTC, like `start`): series occurrences that no longer exist.
    /// Only emitted alongside an RRULE.
    pub exdates: Vec<NaiveDateTime>,
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
    pub coach_name: &'a str,
    pub organization_name: &'a str,
    pub topics: &'a [String],
    pub goal_titles: &'a [String],
    pub open_actions: &'a [OpenAction],
    pub anchor_tz: chrono_tz::Tz,
}

const UTC_BASIC: &str = "%Y%m%dT%H%M%SZ";
const LOCAL_BASIC: &str = "%Y%m%dT%H%M%S";

fn internal(msg: &str) -> Error {
    Error {
        source: None,
        error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(msg.to_string())),
    }
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

/// The zone an invite anchored to `tz` is actually written in: `tz` itself, or UTC when
/// no `VTIMEZONE` block exists for it. Series expansion must use this same zone.
pub fn emitted_zone(tz: chrono_tz::Tz) -> chrono_tz::Tz {
    match tz == chrono_tz::UTC || vtimezone_block(tz).is_some() {
        true => tz,
        false => chrono_tz::UTC,
    }
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
    let freq = match rule.frequency {
        Frequency::Daily => "DAILY",
        Frequency::Weekly | Frequency::Biweekly => "WEEKLY",
        Frequency::Monthly => "MONTHLY",
    };
    let effective_interval = rule.effective_interval();

    let mut parts = vec![format!("FREQ={freq}")];
    // Explicit, so every client aligns multi-week steps on Monday as the expander does.
    parts.push("WKST=MO".to_string());
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
    build_calendar(std::slice::from_ref(invite))
}

/// Build one VCALENDAR holding every invite as a VEVENT, in order, under a single
/// METHOD and a single VTIMEZONE. A series master and its overrides travel together.
pub fn build_calendar(invites: &[IcsInvite]) -> Result<String, Error> {
    let first = invites
        .first()
        .ok_or_else(|| internal("no invites to build a calendar from"))?;
    if invites.iter().any(|i| i.anchor_tz != first.anchor_tz) {
        return Err(internal(
            "invites in one calendar must share an anchor zone",
        ));
    }
    if invites.iter().any(|i| i.method != first.method) {
        return Err(internal("invites in one calendar must share a method"));
    }

    let method = match first.method {
        Method::Request => "REQUEST",
        Method::Cancel => "CANCEL",
    };

    let block = if first.anchor_tz == chrono_tz::UTC {
        None
    } else {
        vtimezone_block(first.anchor_tz)
    };
    if block.is_none() && first.anchor_tz != chrono_tz::UTC {
        warn!(
            "no VTIMEZONE block for zone {}, falling back to UTC",
            first.anchor_tz.name()
        );
    }
    let zone = block.as_ref().map(|_| first.anchor_tz);

    let mut calendar = Calendar::new();
    calendar.append_property(("METHOD", method));
    invites.iter().for_each(|invite| {
        calendar.push(event(invite, zone));
    });

    let serialized = calendar.done().to_string();
    match block {
        Some(block) => splice_vtimezone(&serialized, &block),
        None => Ok(serialized),
    }
}

/// A date-time property in the same form as DTSTART: `TZID` + local wall time when
/// zoned, UTC otherwise. Clients match RECURRENCE-ID and EXDATE against DTSTART.
fn occurrence_property(name: &str, at: NaiveDateTime, zone: Option<chrono_tz::Tz>) -> Property {
    match zone {
        Some(tz) => {
            let local = Utc.from_utc_datetime(&at).with_timezone(&tz).naive_local();
            Property::new(name, local.format(LOCAL_BASIC).to_string())
                .add_parameter("TZID", tz.name())
                .done()
        }
        None => Property::new(name, at.format(UTC_BASIC).to_string()),
    }
}

/// One VEVENT. `zone` is the anchor zone when a VTIMEZONE block accompanies it.
fn event(invite: &IcsInvite, zone: Option<chrono_tz::Tz>) -> Event {
    let status = match invite.status {
        EventStatus::Confirmed => "CONFIRMED",
        EventStatus::Cancelled => "CANCELLED",
    };

    let end = invite.start + Duration::minutes(invite.duration_minutes as i64);

    let mut event = Event::new();
    event
        .uid(&invite.uid)
        .summary(&invite.summary)
        .description(&invite.description)
        .add_property("SEQUENCE", invite.sequence.to_string())
        .add_property("DTSTAMP", invite.dtstamp.format(UTC_BASIC).to_string())
        .add_property("STATUS", status);

    match zone {
        Some(tz) => {
            let start_local = Utc
                .from_utc_datetime(&invite.start)
                .with_timezone(&tz)
                .naive_local();
            let end_local = Utc.from_utc_datetime(&end).with_timezone(&tz).naive_local();
            event.starts(CalendarDateTime::from((start_local, tz)));
            event.ends(CalendarDateTime::from((end_local, tz)));
        }
        None => {
            event.starts(Utc.from_utc_datetime(&invite.start));
            event.ends(Utc.from_utc_datetime(&end));
        }
    }
    if let Some(recurrence_id) = invite.recurrence_id {
        event.append_property(occurrence_property("RECURRENCE-ID", recurrence_id, zone));
    }

    event.append_property(
        Property::new("ORGANIZER", format!("mailto:{}", invite.organizer.email))
            .add_parameter("CN", &invite.organizer.name)
            .done(),
    );
    // Multi-property: plain `append_property` is keyed by name and would keep only the last.
    invite.attendees.iter().for_each(|attendee| {
        event.append_multi_property(
            Property::new("ATTENDEE", format!("mailto:{}", attendee.email))
                .add_parameter("CN", &attendee.name)
                .add_parameter("ROLE", "REQ-PARTICIPANT")
                .add_parameter("PARTSTAT", "NEEDS-ACTION")
                .add_parameter("RSVP", "TRUE")
                .done(),
        );
    });

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

    // An override instance addresses a single occurrence, so it carries no RRULE.
    let rrule = match (&invite.recurrence, invite.recurrence_id) {
        (Some(rule), None) => Some(rule),
        (Some(_), Some(_)) => {
            warn!("dropping RRULE from an invite that carries a RECURRENCE-ID");
            None
        }
        (None, _) => None,
    };
    // EXDATEs only mean something against an RRULE.
    match rrule {
        Some(rule) => {
            event.add_property("RRULE", rrule_value(rule));
            invite.exdates.iter().for_each(|exdate| {
                event.append_multi_property(occurrence_property("EXDATE", *exdate, zone));
            });
        }
        None if !invite.exdates.is_empty() => {
            warn!("dropping EXDATEs from an invite that carries no RRULE");
        }
        None => {}
    }

    event.done()
}

/// Compose the DESCRIPTION string. Each section is omitted when its data is empty.
pub fn compose_description(parts: &DescriptionParts) -> String {
    let mut sections: Vec<String> = Vec::new();

    if let Some(title) = parts.title.filter(|t| !t.is_empty()) {
        sections.push(title.to_string());
    }

    sections.push(format!(
        "Coach: {}\nOrganization: {}",
        parts.coach_name, parts.organization_name
    ));

    sections.push(format!("Join this session: {}", parts.session_url));

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
