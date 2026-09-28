//! Post-merge snippet hydration.
//!
//! `ts_headline` re-parses the raw document per row and cannot use the GIN
//! index, so it never runs in the searcher pass: the domain calls
//! [`hydrate_snippets`] with exactly the returned page, and each entity type
//! present costs one batched query over at most a page's worth of ids.

use std::collections::HashMap;

use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter, QuerySelect,
};

use super::{action, agreement, coaching_session, compile_query, goal, topic};
use super::{CompiledQuery, Hit, HitType};
use crate::error::Error;
use entity::{actions, agreements, coaching_session_topics, coaching_sessions, goals, Id};

/// `<mark>` markers are plain-text delimiters for the FE to split on, not HTML.
const HEADLINE_OPTIONS: &str = "StartSel=<mark>, StopSel=</mark>";

#[derive(Debug, FromQueryResult)]
struct SnippetRow {
    id: Id,
    snippet: String,
}

/// Hydrate `snippet` on every hit of the returned page, one batched
/// `ts_headline` query per entity type present. `raw_q` is the same raw query
/// text the searchers ran, so markers land on the same matches.
pub async fn hydrate_snippets(
    db: &DatabaseConnection,
    raw_q: &str,
    hits: &mut [Hit],
) -> Result<(), Error> {
    let cq = compile_query(raw_q);

    for hit_type in [
        HitType::Action,
        HitType::Agreement,
        HitType::CoachingSession,
        HitType::Goal,
        HitType::Topic,
    ] {
        let ids: Vec<Id> = hits
            .iter()
            .filter(|h| h.hit_type() == hit_type)
            .map(|h| h.core().id)
            .collect();
        if ids.is_empty() {
            continue;
        }
        let snippets = load_snippets(db, hit_type, &cq, &ids).await?;
        for hit in hits.iter_mut().filter(|h| h.hit_type() == hit_type) {
            let core = hit.core_mut();
            core.snippet = snippets.get(&core.id).cloned();
        }
    }
    Ok(())
}

async fn load_snippets(
    db: &DatabaseConnection,
    hit_type: HitType,
    cq: &CompiledQuery,
    ids: &[Id],
) -> Result<HashMap<Id, String>, Error> {
    let rows = match hit_type {
        HitType::Action => {
            select_snippets::<actions::Entity>(actions::Column::Id, action::TEXT_EXPR, cq, ids)
                .all(db)
                .await?
        }
        HitType::Agreement => {
            select_snippets::<agreements::Entity>(
                agreements::Column::Id,
                agreement::TEXT_EXPR,
                cq,
                ids,
            )
            .all(db)
            .await?
        }
        HitType::CoachingSession => {
            select_snippets::<coaching_sessions::Entity>(
                coaching_sessions::Column::Id,
                coaching_session::TEXT_EXPR,
                cq,
                ids,
            )
            .all(db)
            .await?
        }
        HitType::Goal => {
            select_snippets::<goals::Entity>(goals::Column::Id, goal::TEXT_EXPR, cq, ids)
                .all(db)
                .await?
        }
        HitType::Topic => {
            select_snippets::<coaching_session_topics::Entity>(
                coaching_session_topics::Column::Id,
                topic::TEXT_EXPR,
                cq,
                ids,
            )
            .all(db)
            .await?
        }
    };
    Ok(rows.into_iter().map(|r| (r.id, r.snippet)).collect())
}

fn select_snippets<E: EntityTrait>(
    id_column: E::Column,
    text_expr: &str,
    cq: &CompiledQuery,
    ids: &[Id],
) -> sea_orm::Selector<sea_orm::SelectModel<SnippetRow>> {
    let (query_sql, mut binds) = cq.sql_and_binds();
    let options_placeholder = binds.len() + 1;
    binds.push(HEADLINE_OPTIONS.into());
    let headline = Expr::cust_with_values(
        format!("ts_headline('english', {text_expr}, {query_sql}, ${options_placeholder})"),
        binds,
    );
    E::find()
        .select_only()
        .column(id_column)
        .column_as(headline, "snippet")
        .filter(id_column.is_in(ids.iter().copied()))
        .into_model::<SnippetRow>()
}
