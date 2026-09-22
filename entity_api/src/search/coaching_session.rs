//! Coaching-session searcher: FTS over `coaching_sessions.title`.

use async_trait::async_trait;
use sea_orm::entity::prelude::{DateTime, DateTimeWithTimeZone};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, JoinType, Order, QueryFilter,
    QueryOrder, QuerySelect, QueryTrait, RelationTrait,
};

use super::{
    compile_query, cursor_condition, Core, FtsExpressions, Hit, HitType, Request, Searcher,
    SessionHit,
};
use crate::error::Error;
use entity::coaching_relationships;
use entity::coaching_sessions::{Column, Entity, Relation};
use entity::Id;

/// Raw-text source; its tsvector form must stay semantically identical to
/// `idx_coaching_sessions_title_fts`.
pub(super) const TEXT_EXPR: &str = r#"coalesce("coaching_sessions"."title", '')"#;

#[derive(Debug, FromQueryResult)]
struct Row {
    id: Id,
    score: f32,
    created_at: DateTimeWithTimeZone,
    updated_at: DateTimeWithTimeZone,
    organization_id: Id,
    coaching_relationship_id: Id,
    date: DateTime,
}

pub struct SessionSearcher;

#[async_trait]
impl Searcher for SessionSearcher {
    fn hit_type(&self) -> HitType {
        HitType::CoachingSession
    }

    async fn search(&self, db: &DatabaseConnection, req: &Request<'_>) -> Result<Vec<Hit>, Error> {
        let cq = compile_query(req.q);
        let fts = FtsExpressions::over(TEXT_EXPR);
        let select = Entity::find()
            .select_only()
            .column(Column::Id)
            .column_as(fts.score(&cq), "score")
            .column(Column::CreatedAt)
            .column(Column::UpdatedAt)
            .column(Column::CoachingRelationshipId)
            .column(Column::Date)
            // Data join only — organization_id for the hit core, never a scope test.
            .join(JoinType::InnerJoin, Relation::CoachingRelationships.def())
            .column(coaching_relationships::Column::OrganizationId)
            .filter(fts.match_condition(&cq));

        // "By user" means participant for sessions, so the domain pre-narrowed set applies.
        let select = match req.effective_relationship_ids(true) {
            Some(ids) => select.filter(Column::CoachingRelationshipId.is_in(ids.iter().copied())),
            None => select,
        };

        let rows = select
            .apply_if(req.filters.coaching_session_id, |q, id| {
                q.filter(Column::Id.eq(id))
            })
            .apply_if(req.filters.created.from, |q, v| {
                q.filter(Column::CreatedAt.gte(v))
            })
            .apply_if(req.filters.created.to_exclusive, |q, v| {
                q.filter(Column::CreatedAt.lt(v))
            })
            .apply_if(req.filters.updated.from, |q, v| {
                q.filter(Column::UpdatedAt.gte(v))
            })
            .apply_if(req.filters.updated.to_exclusive, |q, v| {
                q.filter(Column::UpdatedAt.lt(v))
            })
            .apply_if(req.cursor, |q, c| {
                q.filter(cursor_condition(
                    || fts.score(&cq),
                    Column::Id,
                    HitType::CoachingSession,
                    c,
                ))
            })
            .order_by(Expr::cust(r#""score""#), Order::Desc)
            .order_by(Column::Id, Order::Asc)
            .limit(req.fetch)
            .into_model::<Row>()
            .all(db)
            .await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                Hit::CoachingSession(SessionHit {
                    core: Core {
                        id: r.id,
                        score: r.score,
                        // Hydrated post-merge: composed display title with the
                        // dated fallback.
                        title: String::new(),
                        snippet: None,
                        created_at: r.created_at,
                        updated_at: Some(r.updated_at),
                        organization_id: r.organization_id,
                    },
                    coaching_relationship_id: r.coaching_relationship_id,
                    date: r.date,
                    display_title: String::new(),
                })
            })
            .collect())
    }
}
