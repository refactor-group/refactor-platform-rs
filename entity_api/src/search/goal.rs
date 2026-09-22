//! Goal searcher: FTS over `goals.title || ' ' || goals.body`.

use async_trait::async_trait;
use sea_orm::entity::prelude::DateTimeWithTimeZone;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, JoinType, Order, QueryFilter,
    QueryOrder, QuerySelect, QueryTrait, RelationTrait,
};

use super::{
    compile_query, cursor_condition, excerpt_title, Core, FtsExpressions, GoalHit, Hit, HitType,
    Request, Searcher,
};
use crate::error::Error;
use entity::coaching_relationships;
use entity::goals::{Column, Entity, Relation};
use entity::status::Status;
use entity::Id;

/// Raw-text source; its tsvector form must stay semantically identical to
/// `idx_goals_fts`.
pub(super) const TEXT_EXPR: &str =
    r#"coalesce("goals"."title",'') || ' ' || coalesce("goals"."body",'')"#;

#[derive(Debug, FromQueryResult)]
struct Row {
    id: Id,
    score: f32,
    title: Option<String>,
    body: Option<String>,
    status: Status,
    created_in_session_id: Option<Id>,
    created_at: DateTimeWithTimeZone,
    updated_at: DateTimeWithTimeZone,
    organization_id: Id,
    coaching_relationship_id: Id,
}

pub struct GoalSearcher;

#[async_trait]
impl Searcher for GoalSearcher {
    fn hit_type(&self) -> HitType {
        HitType::Goal
    }

    async fn search(&self, db: &DatabaseConnection, req: &Request<'_>) -> Result<Vec<Hit>, Error> {
        let cq = compile_query(req.q);
        let fts = FtsExpressions::over(TEXT_EXPR);
        let select = Entity::find()
            .select_only()
            .column(Column::Id)
            .column_as(fts.score(&cq), "score")
            .column(Column::Title)
            .column(Column::Body)
            .column(Column::Status)
            .column(Column::CreatedInSessionId)
            .column(Column::CreatedAt)
            .column(Column::UpdatedAt)
            .column(Column::CoachingRelationshipId)
            // Data join only — organization_id for the hit core, never a scope test.
            .join(JoinType::InnerJoin, Relation::CoachingRelationships.def())
            .column(coaching_relationships::Column::OrganizationId)
            .filter(fts.match_condition(&cq));

        let select = match req.effective_relationship_ids(false) {
            Some(ids) => select.filter(Column::CoachingRelationshipId.is_in(ids.iter().copied())),
            None => select,
        };

        let rows = select
            // `user_id` means creator for goals.
            .apply_if(req.filters.user_id, |q, id| q.filter(Column::UserId.eq(id)))
            // `goal_id` narrows the goals type to the goal itself.
            .apply_if(req.filters.goal_id, |q, id| q.filter(Column::Id.eq(id)))
            .apply_if(req.filters.status.clone(), |q, s| {
                q.filter(Column::Status.eq(s))
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
                    HitType::Goal,
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
                let title = r
                    .title
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| excerpt_title(r.body.as_deref(), "Goal"));
                Hit::Goal(GoalHit {
                    core: Core {
                        id: r.id,
                        score: r.score,
                        title,
                        snippet: None,
                        created_at: r.created_at,
                        updated_at: Some(r.updated_at),
                        organization_id: r.organization_id,
                    },
                    coaching_relationship_id: r.coaching_relationship_id,
                    status: r.status,
                    created_in_session_id: r.created_in_session_id,
                })
            })
            .collect())
    }
}
