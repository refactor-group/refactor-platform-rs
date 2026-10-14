//! Scoring a run against its expected suggestions.

use chrono::NaiveDate;

use super::case::{parse_transcript, Expected, Transcript};
use super::score::{has_note, score, Finding, Got};
use crate::transcript_suggestion::actions::Role;

fn transcript() -> Transcript {
    parse_transcript(
        "# session: 2026-10-07\n# next_session: 2026-10-21\n# coach: Jim Hodapp\n# coachee: Alex Rivera\n\
         [s1 00:05] Coachee: I'll call the vendor about the renewal. And I'll draft the hiring plan.\n\
         [s2 00:12] Coachee: The hiring plan by Monday.\n\
         [s3 00:20] Coach: I'll send you the article.\n",
    )
    .expect("parses")
}

fn date(d: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(2026, 10, d)
}

fn expected() -> Vec<Expected> {
    vec![
        Expected {
            segments: vec!["s1".into()],
            assignees: vec![vec![Role::Coachee]],
            due_dates: vec![None],
            duplicate_of: None,
            note: "vendor".into(),
        },
        Expected {
            segments: vec!["s1".into(), "s2".into()],
            assignees: vec![vec![Role::Coachee]],
            due_dates: vec![date(12)],
            duplicate_of: None,
            note: "hiring plan".into(),
        },
        Expected {
            segments: vec!["s3".into()],
            assignees: vec![vec![Role::Coach]],
            due_dates: vec![None, date(7)],
            duplicate_of: None,
            note: "article".into(),
        },
    ]
}

fn got(
    segment: &str,
    assignees: Vec<Role>,
    due: Option<NaiveDate>,
    quote: &str,
    body: &str,
) -> Got {
    Got {
        segment: segment.into(),
        assignees,
        due,
        due_phrase: None,
        duplicate_of: None,
        quote: quote.into(),
        body: body.into(),
    }
}

fn errors(findings: &[Finding]) -> usize {
    findings.iter().map(Finding::errors).sum()
}

#[test]
fn a_perfect_answer_has_no_errors_in_any_order() {
    let answer = vec![
        got(
            "s1",
            vec![Role::Coachee],
            date(12),
            "draft the hiring plan",
            "Draft the hiring plan",
        ),
        got(
            "s3",
            vec![Role::Coach],
            None,
            "send you the article",
            "Send the article",
        ),
        got(
            "s1",
            vec![Role::Coachee],
            None,
            "call the vendor",
            "Call the vendor",
        ),
    ];

    let findings = score(&expected(), &answer, &transcript());

    assert_eq!(errors(&findings), 0, "{findings:?}");
    assert!(findings.iter().all(|f| matches!(f, Finding::Ok(_))));
}

#[test]
fn reports_misses_extras_and_wrong_fields() {
    let answer = vec![
        got(
            "s1",
            vec![Role::Coach, Role::Coachee],
            date(9),
            "a made-up quote",
            "Call the vendor",
        ),
        got(
            "s2",
            vec![Role::Coach],
            None,
            "The hiring plan",
            "Something else",
        ),
    ];

    let findings = score(&expected(), &answer, &transcript());

    let Finding::Wrong(_, problems) = &findings[0] else {
        panic!("expected a wrong finding, got {findings:?}");
    };
    assert_eq!(
        problems.len(),
        3,
        "assignees, due date and quote: {problems:?}"
    );
    assert!(matches!(findings[1], Finding::Wrong(_, ref p) if p.len() == 2));
    assert!(matches!(findings[2], Finding::Miss(ref what) if what.contains("article")));
    assert_eq!(errors(&findings), 6);

    let extra = score(
        &expected()[2..],
        &[got("s1", vec![Role::Coachee], None, "x", "Call")],
        &transcript(),
    );
    assert!(matches!(extra[0], Finding::Extra(_)));
    assert!(matches!(extra[1], Finding::Miss(_)));
}

#[test]
fn accepts_any_listed_assignee_set() {
    let mut either = expected()[2].clone();
    either.assignees = vec![vec![Role::Coach, Role::Coachee], vec![Role::Coach]];
    let answer = |people| {
        got(
            "s3",
            people,
            None,
            "send you the article",
            "Send the article",
        )
    };

    for people in [vec![Role::Coachee, Role::Coach], vec![Role::Coach]] {
        assert_eq!(
            errors(&score(&[either.clone()], &[answer(people)], &transcript())),
            0
        );
    }
    assert_eq!(
        errors(&score(
            &[either],
            &[answer(vec![Role::Coachee])],
            &transcript()
        )),
        1
    );
}

#[test]
fn the_model_order_never_changes_the_score() {
    let t = parse_transcript(
        "# session: 2026-10-07\n# next_session: none\n# coach: A\n# coachee: B\n\
         [s6 00:01] Coachee: I'll do the stakeholder map before next time.\n\
         [s7 00:02] Coachee: And I'll talk to Dana about the on-call rotation.\n\
         [s8 00:03] Coach: Recap: the stakeholder map, and talking to Dana.\n",
    )
    .expect("parses");
    let expect = |segments: &[&str], note: &str| Expected {
        segments: segments.iter().map(|s| s.to_string()).collect(),
        assignees: vec![vec![Role::Coachee]],
        due_dates: vec![None],
        duplicate_of: None,
        note: note.into(),
    };
    let expected = [
        expect(&["s6", "s8"], "stakeholder map"),
        expect(&["s7", "s8"], "Dana"),
    ];
    let dana = got(
        "s8",
        vec![Role::Coachee],
        None,
        "talking to Dana",
        "Talk to Dana",
    );
    let map = got(
        "s6",
        vec![Role::Coachee],
        None,
        "the stakeholder map",
        "Do the stakeholder map",
    );

    for answer in [[dana.clone(), map.clone()], [map, dana]] {
        assert_eq!(errors(&score(&expected, &answer, &t)), 0);
    }
}

#[test]
fn pairs_for_the_fewest_problems() {
    // Undated first, it could take the dated expectation; the best pairing gives each its own.
    let answer = [
        got(
            "s1",
            vec![Role::Coachee],
            None,
            "call the vendor",
            "Call the vendor",
        ),
        got(
            "s1",
            vec![Role::Coachee],
            date(12),
            "draft the hiring plan",
            "Draft the hiring plan",
        ),
    ];

    let findings = score(&expected()[..2], &answer, &transcript());

    assert_eq!(errors(&findings), 0, "{findings:?}");
}

#[test]
fn a_lone_answer_pairs_with_its_line_among_many_expectations() {
    let mut many = expected();
    many.extend(expected());
    let answer = [got(
        "s3",
        vec![Role::Coach],
        date(9),
        "send you the article",
        "Send the article",
    )];

    let findings = score(&many, &answer, &transcript());

    assert!(
        matches!(&findings[0], Finding::Wrong(_, p) if p.len() == 1),
        "{findings:?}"
    );
    assert_eq!(errors(&findings), 1 + 5);
}

#[test]
fn checks_the_duplicate_flag() {
    let mut item = got(
        "s3",
        vec![Role::Coach],
        None,
        "send you the article",
        "Send the article",
    );
    item.duplicate_of = Some("a1".into());

    let findings = score(&expected()[2..], &[item], &transcript());

    assert!(matches!(&findings[0], Finding::Wrong(_, p) if p[0].starts_with("duplicate_of")));
}

#[test]
fn an_empty_case_passes_only_with_no_suggestions() {
    assert!(score(&[], &[], &transcript()).is_empty());
    let extra = score(
        &[],
        &[got("s3", vec![Role::Coach], None, "article", "Send it")],
        &transcript(),
    );
    assert_eq!(errors(&extra), 1);
}

#[test]
fn flags_notes_and_placeholders_in_the_body() {
    assert!(has_note("Send the [document] to Priya"));
    assert!(has_note("Call the vendor (details unclear)"));
    assert!(has_note("Send the report TBD"));
    assert!(has_note("Email the recruiter?"));
    assert!(!has_note("Send the revised budget to Priya"));

    let findings = score(
        &expected()[2..],
        &[got(
            "s3",
            vec![Role::Coach],
            None,
            "send you the article",
            "Send the article (title not specified)",
        )],
        &transcript(),
    );
    assert!(matches!(findings[0], Finding::Ok(_)));
    assert!(matches!(findings[1], Finding::Note(_)));
    assert_eq!(errors(&findings), 1);
}
