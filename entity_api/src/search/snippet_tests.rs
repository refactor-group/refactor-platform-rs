use chrono::Utc;
use sea_orm::DbBackend;

use super::super::{ActionHit, Core, GoalHit};
use super::*;
use entity::status::Status;

fn core(id: Id) -> Core {
    Core {
        id,
        score: 0.5,
        title: "t".to_string(),
        snippet: None,
        created_at: Utc::now().into(),
        updated_at: None,
        organization_id: Id::new_v4(),
    }
}

fn goal_hit(id: Id) -> Hit {
    Hit::Goal(GoalHit {
        core: core(id),
        coaching_relationship_id: Id::new_v4(),
        status: Status::NotStarted,
        created_in_session_id: None,
    })
}

fn action_hit(id: Id) -> Hit {
    Hit::Action(ActionHit {
        core: core(id),
        coaching_session_id: Id::new_v4(),
        coaching_relationship_id: Id::new_v4(),
        goal_id: None,
        status: Status::InProgress,
        due_by: None,
        session_date: Utc::now().naive_utc(),
        session_title: String::new(),
    })
}

fn sql(statement: &SelectStatement) -> String {
    DbBackend::Postgres.build(statement).sql
}

#[test]
fn markers_become_mark_tags() {
    assert_eq!(
        to_marked_html("the \u{2}quarterly\u{3} review"),
        "the <mark>quarterly</mark> review"
    );
}

#[test]
fn stored_mark_tags_are_escaped_not_trusted() {
    assert_eq!(
        to_marked_html("<mark>fake</mark> \u{2}real\u{3}"),
        "&lt;mark&gt;fake&lt;/mark&gt; <mark>real</mark>"
    );
}

#[test]
fn ampersands_are_escaped_so_entities_round_trip() {
    assert_eq!(
        to_marked_html("a &lt; b & c > d"),
        "a &amp;lt; b &amp; c &gt; d"
    );
}

#[test]
fn headline_strips_marker_characters_from_the_document() {
    let sql = sql(&arm_for(
        HitType::Goal,
        &compile_query("quarterly"),
        &[Id::new_v4()],
    ));
    assert!(
        sql.contains(&format!(
            "translate({}, chr(2) || chr(3), '')",
            goal::TEXT_EXPR
        )),
        "{sql}"
    );
}

#[test]
fn a_mixed_page_is_one_union_over_the_types_present() {
    let hits = vec![
        action_hit(Id::new_v4()),
        goal_hit(Id::new_v4()),
        goal_hit(Id::new_v4()),
    ];

    let statement = snippet_statement(&compile_query("quarterly"), &ids_by_type(&hits))
        .expect("a non-empty page builds a statement");
    let sql = sql(&statement);

    assert_eq!(sql.matches("UNION ALL").count(), 1, "{sql}");
    assert_eq!(sql.matches("ts_headline").count(), 2, "{sql}");
    assert!(
        sql.contains(action::TEXT_EXPR) && sql.contains(goal::TEXT_EXPR),
        "{sql}"
    );
    assert!(!sql.contains(agreement::TEXT_EXPR), "{sql}");
}

#[test]
fn an_empty_page_builds_no_statement() {
    assert!(snippet_statement(&compile_query("quarterly"), &ids_by_type(&[])).is_none());
}

#[test]
fn rows_map_back_to_their_type_and_unknown_codes_are_dropped() {
    let action_id = Id::new_v4();
    let goal_id = Id::new_v4();
    let rows = vec![
        SnippetRow {
            id: action_id,
            hit_type_code: i16::from(HitType::Action.code()),
            snippet: "the \u{2}quarterly\u{3} action".to_string(),
        },
        SnippetRow {
            id: goal_id,
            hit_type_code: i16::from(HitType::Goal.code()),
            snippet: "the \u{2}quarterly\u{3} goal".to_string(),
        },
        SnippetRow {
            id: Id::new_v4(),
            hit_type_code: 250,
            snippet: "never".to_string(),
        },
    ];

    let snippets = index_by_type_and_id(rows);

    assert_eq!(
        snippets,
        HashMap::from([
            (
                (HitType::Action, action_id),
                "the <mark>quarterly</mark> action".to_string()
            ),
            (
                (HitType::Goal, goal_id),
                "the <mark>quarterly</mark> goal".to_string()
            ),
        ])
    );
}

#[test]
fn applying_snippets_leaves_hits_without_a_row_at_none() {
    let action_id = Id::new_v4();
    let goal_a = Id::new_v4();
    let goal_b = Id::new_v4();
    let mut hits = vec![action_hit(action_id), goal_hit(goal_a), goal_hit(goal_b)];
    // Same id under the wrong type must not leak across types.
    let snippets = HashMap::from([
        ((HitType::Action, action_id), "action".to_string()),
        ((HitType::Goal, goal_a), "goal".to_string()),
        ((HitType::Action, goal_b), "wrong type".to_string()),
    ]);

    apply_snippets(&mut hits, snippets);

    assert_eq!(hits[0].core().snippet.as_deref(), Some("action"));
    assert_eq!(hits[1].core().snippet.as_deref(), Some("goal"));
    assert_eq!(hits[2].core().snippet, None);
}
