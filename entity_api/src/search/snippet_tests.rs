use super::*;

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
    let sql = select_snippets::<goals::Entity>(
        goals::Column::Id,
        goal::TEXT_EXPR,
        &compile_query("quarterly"),
        &[Id::new_v4()],
    )
    .into_statement(sea_orm::DbBackend::Postgres)
    .sql;
    assert!(
        sql.contains(&format!(
            "translate({}, chr(2) || chr(3), '')",
            goal::TEXT_EXPR
        )),
        "{sql}"
    );
}
