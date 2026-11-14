//! Fixture parsing, and checks that every checked-in fixture is usable.

use std::collections::{BTreeMap, HashSet};

use chrono::{NaiveDate, NaiveTime, Weekday};

use super::case::{load_all, parse_transcript, Set, Variant};
use super::report::read_baseline;
use crate::transcript_export::SpeakerRole;

const SOURCE: &str = "\
# A comment line.
# session: Wednesday 2026-10-07
# next_session: Wednesday 2026-10-21 15:30
# timezone: Europe/Berlin
# week_end: Thursday
# coach: Jim Hodapp
# coachee: Alex Rivera
# existing a1: Update the onboarding doc

[s1 00:05] Coach: Good to see you.
[s1b 01:02] Coachee: I'll send it: today.
[s2 02:10] Priya Shah: Happy to help.
";

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

#[test]
fn parses_the_header_and_lines() {
    let t = parse_transcript(SOURCE).expect("parses");

    assert_eq!(t.session_date, date(2026, 10, 7));
    assert_eq!(
        t.next_session,
        Some((
            date(2026, 10, 21),
            NaiveTime::from_hms_opt(15, 30, 0).expect("time")
        ))
    );
    assert_eq!(t.timezone, chrono_tz::Europe::Berlin);
    assert_eq!(t.week_end, Weekday::Thu);
    assert_eq!(
        (t.coach_name.as_str(), t.coachee_name.as_str()),
        ("Jim Hodapp", "Alex Rivera")
    );
    assert_eq!(
        t.existing,
        vec![("a1".to_string(), "Update the onboarding doc".to_string())]
    );
    let labels: Vec<_> = t.lines.iter().map(|l| l.label.as_str()).collect();
    assert_eq!(labels, ["s1", "s1b", "s2"]);
    assert_eq!(t.lines[1].start_ms, 62_000);
    assert_eq!(t.lines[1].text, "I'll send it: today.");
}

#[test]
fn defaults_and_no_next_session() {
    let t = parse_transcript(
        "# session: 2026-10-07\n# next_session: none\n# coach: A\n# coachee: B\n[s1 00:01] Coach: Hi.\n",
    )
    .expect("parses");

    assert_eq!(t.next_session, None);
    assert_eq!(t.timezone, chrono_tz::America::Chicago);
    assert_eq!(t.week_end, Weekday::Fri);
}

#[test]
fn rejects_a_malformed_line_and_a_missing_header() {
    let bad_line = parse_transcript("# session: 2026-10-07\n# coach: A\n# coachee: B\nhello\n");
    assert!(bad_line.is_err_and(|e| e.starts_with("line 4")));
    assert!(parse_transcript("# coach: A\n# coachee: B\n").is_err());
}

#[test]
fn session_maps_speakers_and_labels_back() {
    let t = parse_transcript(SOURCE).expect("parses");
    let input = t.input();
    let session = t.session(&input);

    let roles: Vec<_> = session.segments.iter().map(|s| s.speaker_role).collect();
    assert_eq!(
        roles,
        [Some(SpeakerRole::Coach), Some(SpeakerRole::Coachee), None]
    );
    assert_eq!(session.segments[2].speaker_label, "Priya Shah");
    assert_eq!(
        session.next_session.map(|n| n.date),
        Some(date(2026, 10, 21))
    );
    assert_eq!(t.segment_label(&input, session.segments[1].id), Some("s1b"));
    assert_eq!(t.existing_label(&input, session.existing[0].id), Some("a1"));
}

#[test]
fn every_fixture_loads_and_names_real_lines() {
    let variants = load_all().expect("fixtures load");
    assert!(!variants.is_empty());

    for v in &variants {
        let labels: HashSet<&str> = v
            .transcript
            .lines
            .iter()
            .map(|l| l.label.as_str())
            .collect();
        assert_eq!(
            labels.len(),
            v.transcript.lines.len(),
            "{}: repeated line label",
            v.id()
        );
        let existing: HashSet<&str> = v
            .transcript
            .existing
            .iter()
            .map(|(l, _)| l.as_str())
            .collect();
        for e in &v.expected {
            assert!(
                e.segments.iter().any(|s| labels.contains(s.as_str())),
                "{}: no line of {:?} ({}) is in this transcript",
                v.id(),
                e.segments,
                e.note
            );
            assert!(
                e.duplicate_of
                    .as_deref()
                    .is_none_or(|d| existing.contains(d)),
                "{}: unknown existing action {:?}",
                v.id(),
                e.duplicate_of
            );
            assert!(
                !e.due_dates.is_empty(),
                "{}: {} lists no due date",
                v.id(),
                e.note
            );
        }
    }
}

#[test]
fn every_expected_label_exists_in_some_variant_of_its_case() {
    let variants = load_all().expect("fixtures load");
    for (case, group) in by_case(&variants) {
        let labels: HashSet<&str> = group
            .iter()
            .flat_map(|v| v.transcript.lines.iter().map(|l| l.label.as_str()))
            .collect();
        for v in &group {
            for label in v.expected.iter().flat_map(|e| &e.segments) {
                assert!(
                    labels.contains(label.as_str()),
                    "{case}: no variant has line {label}"
                );
            }
        }
    }
}

#[test]
fn every_clean_case_has_two_wordings() {
    let variants = load_all().expect("fixtures load");
    for (case, group) in by_case(&variants) {
        if group[0].set == Set::Clean {
            assert!(
                group.len() >= 2,
                "{case} needs at least two wording variants"
            );
        }
    }
}

#[test]
fn baseline_names_only_existing_variants() {
    let ids: HashSet<String> = load_all()
        .expect("fixtures load")
        .iter()
        .map(Variant::id)
        .collect();
    for id in read_baseline().expect("baseline reads").keys() {
        assert!(
            ids.contains(id),
            "baseline.json names a variant that no longer exists: {id}"
        );
    }
}

fn by_case(variants: &[Variant]) -> BTreeMap<String, Vec<&Variant>> {
    variants.iter().fold(BTreeMap::new(), |mut cases, v| {
        cases.entry(v.case_id()).or_default().push(v);
        cases
    })
}
