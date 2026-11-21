//! One sample of each composed email, for snapshots, layout checks and previews.

use crate::email_composition::series::Scope;
use crate::email_composition::session::CancelReason;
use crate::email_composition::{
    calendar_attachments, compose, Actor, Bodies, Brand, Email, Parties, Person, SeriesCancelled,
    SeriesRescheduled, SeriesScheduled, SessionCancelled, SessionReminder, SessionRescheduled,
    SessionScheduled, When,
};
use crate::email_outbox::{Attachment, OutgoingEmail};
use crate::gateway::ical::Method;

pub(crate) fn brand() -> Brand {
    Brand::new("https://app.example.com")
}

pub(crate) fn parties() -> Parties {
    Parties {
        recipient: Person {
            first_name: "Jane".into(),
            full_name: "Jane Doe".into(),
            email: "jane@example.com".into(),
        },
        other: Person {
            first_name: "Alex".into(),
            full_name: "Alex Smith".into(),
            email: "alex@example.com".into(),
        },
        session_label: "coaching session",
        session_label_plural: "coaching sessions",
        organization_name: "Acme Corp".into(),
    }
}

pub(crate) fn when(date: &str, day: &str, time: &str) -> When {
    When {
        date: date.into(),
        day: day.into(),
        time: time.into(),
    }
}

fn march_4() -> When {
    when("Wednesday, March 4, 2026", "Wed, Mar 4", "10:00 AM")
}

const SESSION_URL: &str = "https://app.example.com/coaching-sessions/1";

/// A one-part calendar attachment with `method`.
pub(crate) fn calendar(method: &Method) -> Vec<Attachment> {
    calendar_attachments([("BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n", method)])
}

pub(crate) fn session_scheduled() -> SessionScheduled {
    SessionScheduled {
        parties: parties(),
        by: Actor::Other,
        when: march_4(),
        duration: "1 hour".into(),
        url: SESSION_URL.into(),
        calendar: calendar(&Method::Request),
    }
}

pub(crate) fn session_rescheduled() -> SessionRescheduled {
    SessionRescheduled {
        parties: parties(),
        when: march_4(),
        previous: Some(when("Tuesday, March 3, 2026", "Tue, Mar 3", "4:00 PM")),
        duration: "1 hour".into(),
        url: SESSION_URL.into(),
        calendar: calendar(&Method::Request),
    }
}

pub(crate) fn session_cancelled() -> SessionCancelled {
    SessionCancelled {
        parties: parties(),
        when: march_4(),
        reason: CancelReason::Cancelled { by: Actor::Other },
        calendar: calendar(&Method::Cancel),
    }
}

pub(crate) fn session_reminder() -> SessionReminder {
    SessionReminder {
        parties: parties(),
        title: Some("Quarterly planning".into()),
        when: march_4(),
        duration: "1 hour".into(),
        url: SESSION_URL.into(),
        meeting_url: Some("https://meet.example.com/alex-jane".into()),
    }
}

pub(crate) fn series_scheduled() -> SeriesScheduled {
    SeriesScheduled {
        parties: parties(),
        by: Actor::Other,
        first: march_4(),
        last_date: "Wednesday, May 27, 2026".into(),
        duration: "1 hour".into(),
        cadence: "Weekly on Wednesday".into(),
        count: 13,
        url: SESSION_URL.into(),
        calendar: calendar(&Method::Request),
    }
}

pub(crate) fn series_rescheduled() -> SeriesRescheduled {
    SeriesRescheduled {
        parties: parties(),
        by: Actor::Other,
        scope: Scope::WholeSeries {
            previous_start: Some(when("Tuesday, March 3, 2026", "Tue, Mar 3", "4:00 PM")),
            previous_cadence: Some("Weekly on Tuesday".into()),
        },
        starts: march_4(),
        cadence: "Weekly on Wednesday".into(),
        duration: "1 hour".into(),
        last_date: "Wednesday, May 27, 2026".into(),
        count: 13,
        url: SESSION_URL.into(),
        calendar: calendar(&Method::Request),
    }
}

pub(crate) fn series_cancelled() -> SeriesCancelled {
    SeriesCancelled {
        parties: parties(),
        by: Actor::Other,
        count: 13,
        first_date: "Wednesday, March 4, 2026".into(),
        last_date: "Wednesday, May 27, 2026".into(),
        calendar: calendar(&Method::Cancel),
    }
}

pub(crate) fn series_rescheduled_from_session() -> SeriesRescheduled {
    SeriesRescheduled {
        scope: Scope::FromSession {
            first_moved_date: "Wednesday, March 4, 2026".into(),
            previous_cadence: "Weekly on Tuesday".into(),
            previous_time: "4:00 PM".into(),
        },
        count: 12,
        ..series_rescheduled()
    }
}

/// Every kind's sample, and the series reschedule from one session on, composed with
/// `brand`, beside its name.
pub(crate) fn every_email(brand: &Brand) -> Vec<(&'static str, OutgoingEmail)> {
    fn composed<E: Email>(email: &E, brand: &Brand) -> (&'static str, OutgoingEmail) {
        (E::KIND, compose(email, brand).expect("the sample composes"))
    }
    vec![
        composed(&session_scheduled(), brand),
        composed(&session_rescheduled(), brand),
        composed(&session_cancelled(), brand),
        composed(&session_reminder(), brand),
        composed(&series_scheduled(), brand),
        composed(&series_rescheduled(), brand),
        (
            "series_rescheduled_from_session",
            compose(&series_rescheduled_from_session(), brand).expect("the sample composes"),
        ),
        composed(&series_cancelled(), brand),
    ]
}

/// `email`'s HTML and text bodies, rendered with [`brand`].
pub(crate) fn bodies(email: &impl Bodies) -> (String, String) {
    let brand = brand();
    (
        email.html(&brand).expect("the HTML renders"),
        email.text(&brand).expect("the text renders"),
    )
}

/// Everything `email` says, for a snapshot: its subject, its preheader and its text.
pub(crate) fn copy<E: Email>(email: &E) -> String {
    let composed = compose(email, &brand()).expect("the email composes");
    format!(
        "Subject: {}\nPreheader: {}\n\n{}\n",
        composed.subject,
        preheader(&composed.html),
        composed.text
    )
}

/// The hidden preheader of a composed email's HTML, without its padding.
fn preheader(html: &str) -> &str {
    html.split_once(r#"<div style="display: none;"#)
        .and_then(|(_, rest)| rest.split_once('>'))
        .and_then(|(_, rest)| rest.split_once('\u{200c}'))
        .map(|(preheader, _)| preheader)
        .expect("the layout has a preheader")
}
