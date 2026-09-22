//! Action searcher: FTS over `actions.body`, anchored to a relationship
//! through the owning coaching session.

use async_trait::async_trait;
use sea_orm::entity::prelude::{DateTime, DateTimeWithTimeZone};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, JoinType, Order, QueryFilter,
    QueryOrder, QuerySelect, QueryTrait, RelationTrait,
};

use super::{
    compile_query, cursor_condition, excerpt_title, ActionHit, Core, FtsExpressions, GoalFilter,
    Hit, HitType, Request, Searcher,
};
use crate::error::Error;
use entity::actions::{Column, Entity, Relation};
use entity::status::Status;
use entity::{coaching_relationships, coaching_sessions, Id};

/// Raw-text source; its tsvector form must stay semantically identical to
/// `idx_actions_body_fts`.
pub(super) const TEXT_EXPR: &str = r#"coalesce("actions"."body", '')"#;

#[derive(Debug, FromQueryResult)]
struct Row {
    id: Id,
    score: f32,
    body: Option<String>,
    goal_id: Option<Id>,
    status: Status,
    due_by: Option<DateTimeWithTimeZone>,
    coaching_session_id: Id,
    created_at: DateTimeWithTimeZone,
    updated_at: DateTimeWithTimeZone,
    coaching_relationship_id: Id,
    session_date: DateTime,
    organization_id: Id,
}

pub struct ActionSearcher;

#[async_trait]
impl Searcher for ActionSearcher {
    fn hit_type(&self) -> HitType {
        HitType::Action
    }

    async fn search(&self, db: &DatabaseConnection, req: &Request<'_>) -> Result<Vec<Hit>, Error> {
        let cq = compile_query(req.q);
        let fts = FtsExpressions::over(TEXT_EXPR);
        let select = Entity::find()
            .select_only()
            .column(Column::Id)
            .column_as(fts.score(&cq), "score")
            .column(Column::Body)
            .column(Column::GoalId)
            .column(Column::Status)
            .column(Column::DueBy)
            .column(Column::CoachingSessionId)
            .column(Column::CreatedAt)
            .column(Column::UpdatedAt)
            // FK path to the relationship id, plus the organization data join.
            .join(JoinType::InnerJoin, Relation::CoachingSessions.def())
            .column_as(
                coaching_sessions::Column::CoachingRelationshipId,
                "coaching_relationship_id",
            )
            .column_as(coaching_sessions::Column::Date, "session_date")
            .join(
                JoinType::InnerJoin,
                coaching_sessions::Relation::CoachingRelationships.def(),
            )
            .column(coaching_relationships::Column::OrganizationId)
            .filter(fts.match_condition(&cq));

        let select = match req.effective_relationship_ids(false) {
            Some(ids) => select.filter(
                coaching_sessions::Column::CoachingRelationshipId.is_in(ids.iter().copied()),
            ),
            None => select,
        };

        let select = match req.filters.goal_filter {
            GoalFilter::All => select,
            GoalFilter::Linked => select.filter(Column::GoalId.is_not_null()),
            GoalFilter::Unlinked => select.filter(Column::GoalId.is_null()),
        };

        let rows = select
            // `user_id` means creator for actions.
            .apply_if(req.filters.user_id, |q, id| q.filter(Column::UserId.eq(id)))
            .apply_if(req.filters.coaching_session_id, |q, id| {
                q.filter(Column::CoachingSessionId.eq(id))
            })
            .apply_if(req.filters.goal_id, |q, id| q.filter(Column::GoalId.eq(id)))
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
                    HitType::Action,
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
                Hit::Action(ActionHit {
                    core: Core {
                        id: r.id,
                        score: r.score,
                        title: excerpt_title(r.body.as_deref(), "Action"),
                        snippet: None,
                        created_at: r.created_at,
                        updated_at: Some(r.updated_at),
                        organization_id: r.organization_id,
                    },
                    coaching_session_id: r.coaching_session_id,
                    coaching_relationship_id: r.coaching_relationship_id,
                    goal_id: r.goal_id,
                    status: r.status,
                    due_by: r.due_by,
                    session_date: r.session_date,
                    session_display_title: String::new(),
                })
            })
            .collect())
    }
}
