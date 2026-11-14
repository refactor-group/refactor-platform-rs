//! Post-merge snippet hydration.
//!
//! `ts_headline` re-parses the raw document per row and cannot use the GIN
//! index, so it never runs in the searcher pass: the domain calls
//! [`resolve_snippets`] with exactly the returned page. Every entity type on
//! the page is one arm of a single `UNION ALL`, so a mixed page costs one
//! round trip over at most a page's worth of ids, however many types it spans.

use std::collections::HashMap;

use sea_orm::sea_query::{Alias, Expr, Query, SelectStatement, UnionType};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult};

use super::{
    action, agreement, coaching_session, compile_query, goal, topic, transcript_suggestion,
};
use super::{CompiledQuery, Hit, HitType};
use crate::error::Error;
use entity::{actions, agreements, coaching_session_topics, coaching_sessions, goals, Id};

/// Control-character markers that stored text can never contain (they are
/// stripped from the document first), so a match is never forgeable.
const START_SEL: char = '\u{2}';
const STOP_SEL: char = '\u{3}';
const HEADLINE_OPTIONS: &str = "StartSel=\u{2}, StopSel=\u{3}";

#[derive(Debug, FromQueryResult)]
struct SnippetRow {
    id: Id,
    /// The arm's `HitType::code`, so rows map back to a type after the union.
    hit_type_code: i16,
    snippet: String,
}

/// Marked-up snippets for the hits of the returned page, keyed by type and
/// id. `raw_q` is the same raw query text the searchers ran, so markers land
/// on the same matches. Hits whose document no longer exists are absent.
pub async fn resolve_snippets(
    db: &DatabaseConnection,
    raw_q: &str,
    hits: &[Hit],
) -> Result<HashMap<(HitType, Id), String>, Error> {
    let Some(union_query) = snippet_statement(&compile_query(raw_q), &ids_by_type(hits)) else {
        return Ok(HashMap::new());
    };
    let rows = SnippetRow::find_by_statement(db.get_database_backend().build(&union_query))
        .all(db)
        .await?;
    Ok(index_by_type_and_id(rows))
}

/// Write the resolved snippets onto the page. A hit with no row is left as
/// the searcher produced it (`None` today), so a future type that snippets in
/// its own pass is not wiped here.
pub fn apply_snippets(hits: &mut [Hit], mut snippets: HashMap<(HitType, Id), String>) {
    hits.iter_mut().for_each(|hit| {
        if let Some(snippet) = snippets.remove(&(hit.hit_type(), hit.core().id)) {
            hit.core_mut().snippet = Some(snippet);
        }
    });
}

/// The page's ids grouped by type, in `HitType::ALL` order, types absent from
/// the page omitted.
fn ids_by_type(hits: &[Hit]) -> Vec<(HitType, Vec<Id>)> {
    HitType::ALL
        .into_iter()
        .map(|hit_type| {
            let ids = hits
                .iter()
                .filter(|h| h.hit_type() == hit_type)
                .map(|h| h.core().id)
                .collect::<Vec<_>>();
            (hit_type, ids)
        })
        .filter(|(_, ids)| !ids.is_empty())
        .collect()
}

/// One `UNION ALL` of a `ts_headline` arm per type present, or `None` for an
/// empty page so no query runs.
fn snippet_statement(cq: &CompiledQuery, groups: &[(HitType, Vec<Id>)]) -> Option<SelectStatement> {
    let mut arms = groups
        .iter()
        .map(|(hit_type, ids)| arm_for(*hit_type, cq, ids));
    let mut union_query = arms.next()?;
    union_query.unions(arms.map(|arm| (UnionType::All, arm)));
    Some(union_query)
}

fn index_by_type_and_id(rows: Vec<SnippetRow>) -> HashMap<(HitType, Id), String> {
    rows.into_iter()
        .filter_map(|row| {
            let hit_type = HitType::from_code(u8::try_from(row.hit_type_code).ok()?)?;
            Some(((hit_type, row.id), to_marked_html(&row.snippet)))
        })
        .collect()
}

/// HTML-escape a raw headline and swap its control markers for
/// `<mark>…</mark>`, so the only tags in a snippet are the ones we add.
fn to_marked_html(headline: &str) -> String {
    headline
        .chars()
        .fold(String::with_capacity(headline.len()), |mut html, ch| {
            match ch {
                START_SEL => html.push_str("<mark>"),
                STOP_SEL => html.push_str("</mark>"),
                '&' => html.push_str("&amp;"),
                '<' => html.push_str("&lt;"),
                '>' => html.push_str("&gt;"),
                ch => html.push(ch),
            }
            html
        })
}

/// The union arm for one type: dispatches to the entity, id column and text
/// expression that differ per type.
fn arm_for(hit_type: HitType, cq: &CompiledQuery, ids: &[Id]) -> SelectStatement {
    match hit_type {
        HitType::Action => {
            build_arm::<actions::Entity>(actions::Column::Id, action::TEXT_EXPR, hit_type, cq, ids)
        }
        HitType::Agreement => build_arm::<agreements::Entity>(
            agreements::Column::Id,
            agreement::TEXT_EXPR,
            hit_type,
            cq,
            ids,
        ),
        HitType::CoachingSession => build_arm::<coaching_sessions::Entity>(
            coaching_sessions::Column::Id,
            coaching_session::TEXT_EXPR,
            hit_type,
            cq,
            ids,
        ),
        HitType::Goal => {
            build_arm::<goals::Entity>(goals::Column::Id, goal::TEXT_EXPR, hit_type, cq, ids)
        }
        HitType::Topic => build_arm::<coaching_session_topics::Entity>(
            coaching_session_topics::Column::Id,
            topic::TEXT_EXPR,
            hit_type,
            cq,
            ids,
        ),
        HitType::TranscriptSuggestion => build_arm::<entity::transcript_suggestion::Entity>(
            entity::transcript_suggestion::Column::Id,
            transcript_suggestion::TEXT_EXPR,
            hit_type,
            cq,
            ids,
        ),
    }
}

/// `SELECT id, <type code>, ts_headline(...) FROM <table> WHERE id IN (...)`
/// for one type; the union's column shape, so every arm must match it.
fn build_arm<E: EntityTrait>(
    id_column: E::Column,
    text_expr: &str,
    hit_type: HitType,
    cq: &CompiledQuery,
    ids: &[Id],
) -> SelectStatement {
    let (query_sql, mut binds) = cq.sql_and_binds();
    let options_placeholder = binds.len() + 1;
    binds.push(HEADLINE_OPTIONS.into());
    // Strip the marker characters from the document so stored text can't fake them.
    let headline_expr = Expr::cust_with_values(
        format!(
            "ts_headline('english', translate({text_expr}, chr(2) || chr(3), ''), \
             {query_sql}, ${options_placeholder})"
        ),
        binds,
    );
    Query::select()
        .expr_as(Expr::col(id_column), Alias::new("id"))
        .expr_as(
            Expr::val(i16::from(hit_type.code())),
            Alias::new("hit_type_code"),
        )
        .expr_as(headline_expr, Alias::new("snippet"))
        .from(E::default().table_ref())
        .and_where(id_column.is_in(ids.iter().copied()))
        .take()
}

#[cfg(test)]
#[path = "snippet_tests.rs"]
mod tests;
