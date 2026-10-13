//! Action searcher: FTS over `actions.body`, anchored to a relationship
//! through the owning coaching session.

use async_trait::async_trait;
use sea_orm::entity::prelude::{DateTime, DateTimeWithTimeZone};
use sea_orm::{
    ColumnTrait, DatabaseConnection, FromQueryResult, QueryFilter, QuerySelect, QueryTrait,
    RelationTrait,
};

use super::{excerpt_title, ActionHit, Core, FtsQuery, GoalFilter, Hit, HitType, Request, Source};
use crate::error::Error;
use entity::actions::{Column, Entity, Relation};
use entity::status::Status;
use entity::{coaching_sessions, Id};

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

pub struct Searcher;

#[async_trait]
impl super::Searcher for Searcher {
    fn hit_type(&self) -> HitType {
        HitType::Action
    }

    async fn search(&self, db: &DatabaseConnection, req: &Request<'_>) -> Result<Vec<Hit>, Error> {
        let q = FtsQuery::new(
            req,
            Source {
                hit_type: HitType::Action,
                text_expr: TEXT_EXPR,
                id: Column::Id,
                created_at: Column::CreatedAt,
                updated_at: Column::UpdatedAt,
                relationship_id: coaching_sessions::Column::CoachingRelationshipId,
                path: vec![
                    Relation::CoachingSessions.def(),
                    coaching_sessions::Relation::CoachingRelationships.def(),
                ],
            },
        );
        let select = q
            .select::<Entity>()
            .column(Column::Body)
            .column(Column::GoalId)
            .column(Column::Status)
            .column(Column::DueBy)
            .column(Column::CoachingSessionId)
            .column_as(coaching_sessions::Column::Date, "session_date");

        let select = match req.filters.goal_filter {
            GoalFilter::All => select,
            GoalFilter::Linked => select.filter(Column::GoalId.is_not_null()),
            GoalFilter::Unlinked => select.filter(Column::GoalId.is_null()),
        };

        let select = select
            // `user_id` means creator for actions.
            .apply_if(req.filters.user_id, |q, id| q.filter(Column::UserId.eq(id)))
            .apply_if(req.filters.coaching_session_id, |q, id| {
                q.filter(Column::CoachingSessionId.eq(id))
            })
            .apply_if(req.filters.goal_id, |q, id| q.filter(Column::GoalId.eq(id)))
            .apply_if(req.filters.status.clone(), |q, s| {
                q.filter(Column::Status.eq(s))
            });

        let rows = q.finish(select).into_model::<Row>().all(db).await?;

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
                    session_title: String::new(),
                })
            })
            .collect())
    }
}
