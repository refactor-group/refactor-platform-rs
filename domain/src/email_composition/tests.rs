//! The layout every email shares, and what composing one sets.

use std::fs;
use std::path::PathBuf;

use super::*;
use crate::error::DomainErrorKind;
use crate::test_utils::email_composition::{
    brand, calendar, every_email, parties, session_cancelled, session_reminder, session_scheduled,
};

/// The layout's contract, through one email: structure, inlined styles and links.
#[test]
fn the_layout_renders_as_snapshotted() {
    let email = compose(&session_scheduled(), &brand()).expect("the email composes");

    insta::assert_snapshot!("layout_session_scheduled", email.html);
}

/// A user named `first` `last`, with a display name the composed emails don't use.
fn user(first: &str, last: &str) -> users::Model {
    let now = chrono::Utc::now().fixed_offset();
    users::Model {
        id: Id::new_v4(),
        email: "pat@example.com".into(),
        first_name: first.into(),
        last_name: last.into(),
        display_name: Some("Pat the Coach".into()),
        password: None,
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".into(),
        default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
        working_hours_start: crate::working_hours::default_start(),
        working_hours_end: crate::working_hours::default_end(),
        roles: Vec::new(),
        invite_status: None,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn a_person_falls_back_through_blank_names() {
    let names = |first, last| {
        let person = Person::from(&user(first, last));
        (person.first_name, person.full_name)
    };

    assert_eq!(names(" Pat ", " Lee "), ("Pat".into(), "Pat Lee".into()));
    assert_eq!(names("", "Lee"), ("Pat the Coach".into(), "Lee".into()));
    assert_eq!(
        names(" ", ""),
        ("Pat the Coach".into(), "pat@example.com".into())
    );
}

#[test]
fn compose_sends_from_the_brand_to_the_recipient_with_the_calendar() {
    let email = compose(&session_cancelled(), &brand()).expect("the email composes");

    assert_eq!(
        email.from,
        Mailbox {
            email: "hello@mail.myrefactor.com".into(),
            name: Some("Refactor Coach".into()),
        }
    );
    assert_eq!(
        email.reply_to,
        Some(Mailbox {
            email: "hello@refactorcoach.com".into(),
            name: None,
        })
    );
    assert_eq!(
        email.to,
        [Mailbox {
            email: "jane@example.com".into(),
            name: Some("Jane Doe".into()),
        }]
    );
    assert_eq!(email.subject, session_cancelled().subject());
    assert!(
        email.attachments == calendar(&Method::Cancel),
        "the calendar cancellation rides along"
    );
}

#[test]
fn compose_inlines_the_stylesheet_and_keeps_its_media_rules() {
    let html = compose(&session_scheduled(), &brand())
        .expect("the email composes")
        .html;

    assert!(
        html.contains(r#"<td class="content" style="font-family: Inter,"#),
        "{html}"
    );
    assert!(
        !html.contains(".content p {"),
        "inlined rules leave the stylesheet: {html}"
    );
    assert!(
        html.contains("@media (prefers-color-scheme: dark)"),
        "{html}"
    );
    assert!(
        html.contains("@media only screen and (max-width: 620px)"),
        "{html}"
    );
}

#[test]
fn user_text_is_escaped_in_html_and_verbatim_in_text() {
    const HOSTILE: &str = r#"<b>&"'"#;
    let email = SessionReminder {
        parties: Parties {
            other: Person {
                first_name: HOSTILE.into(),
                full_name: HOSTILE.into(),
                email: "alex@example.com".into(),
            },
            organization_name: HOSTILE.into(),
            ..parties()
        },
        title: Some(HOSTILE.into()),
        ..session_reminder()
    };

    let composed = compose(&email, &brand()).expect("the email composes");

    assert!(!composed.html.contains("<b>"), "{}", composed.html);
    assert!(
        composed.html.contains("&lt;b&gt;&amp;"),
        "{}",
        composed.html
    );
    assert!(composed.text.contains(HOSTILE), "{}", composed.text);
    assert!(composed.subject.contains(HOSTILE), "{}", composed.subject);
}

#[test]
fn every_link_in_the_html_is_in_the_text() {
    for (kind, email) in every_email(&brand()) {
        let hrefs: Vec<&str> = email
            .html
            .split("<a ")
            .skip(1)
            .filter_map(|tag| tag.split_once(r#"href=""#))
            .filter_map(|(_, rest)| rest.split('"').next())
            .collect();

        assert!(hrefs.len() >= 3, "{kind} links its footer: {hrefs:?}");
        for href in hrefs {
            assert!(
                email.text.contains(href.trim_start_matches("mailto:")),
                "{kind}'s text lacks {href}"
            );
        }
    }
}

#[test]
fn the_brand_needs_a_frontend_and_links_under_it() {
    let missing = Brand::from_config(&Config::default()).expect_err("no frontend is configured");
    assert!(matches!(
        missing.error_kind,
        DomainErrorKind::Internal(InternalErrorKind::Config)
    ));

    let brand = Brand::from_config(&Config::from_args([
        "test",
        "--frontend-base-url=https://app.example.com/",
    ]))
    .expect("a frontend is configured");
    assert_eq!(
        brand.logo_url,
        "https://app.example.com/22JHC_Monogram_RGB_small.png"
    );
    assert_eq!(brand.privacy_url, "https://app.example.com/privacy");
}

#[test]
fn calendar_parts_are_numbered_after_the_first() {
    let parts = calendar_attachments([
        ("A", &Method::Request),
        ("B", &Method::Cancel),
        ("C", &Method::Cancel),
    ]);

    assert_eq!(
        parts
            .iter()
            .map(|part| (
                part.filename.as_str(),
                part.content_type.as_str(),
                part.content.as_str()
            ))
            .collect::<Vec<_>>(),
        [
            (
                "invite.ics",
                "text/calendar; method=REQUEST; charset=UTF-8",
                "QQ=="
            ),
            (
                "invite-2.ics",
                "text/calendar; method=CANCEL; charset=UTF-8",
                "Qg=="
            ),
            (
                "invite-3.ics",
                "text/calendar; method=CANCEL; charset=UTF-8",
                "Qw=="
            ),
        ]
    );
}

/// Writes each kind's sample to `target/email-preview/<kind>.html` for a browser. Run with
/// `EMAIL_PREVIEW=<frontend base URL> cargo test -p domain write_previews -- --ignored`.
#[test]
#[ignore = "writes previews for a browser; checks nothing"]
fn write_previews() {
    let frontend =
        std::env::var("EMAIL_PREVIEW").unwrap_or_else(|_| "https://app.myrefactor.com".to_string());
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/email-preview");
    fs::create_dir_all(&dir).expect("the preview directory is created");
    for (kind, email) in every_email(&Brand::new(&frontend)) {
        fs::write(dir.join(format!("{kind}.html")), email.html).expect("the preview is written");
    }
}
