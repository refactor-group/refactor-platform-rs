//! Goal searcher: FTS over `goals.title || ' ' || goals.body`.

use async_trait::async_trait;
use sea_orm::entity::prelude::DateTimeWithTimeZone;
use sea_orm::{
    ColumnTrait, DatabaseConnection, FromQueryResult, QueryFilter, QuerySelect, QueryTrait,
    RelationTrait,
};

use super::{excerpt_title, Core, FtsQuery, GoalHit, Hit, HitType, Request, Source};
use crate::error::Error;
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

pub struct Searcher;

#[async_trait]
impl super::Searcher for Searcher {
    fn hit_type(&self) -> HitType {
        HitType::Goal
    }

    async fn search(&self, db: &DatabaseConnection, req: &Request<'_>) -> Result<Vec<Hit>, Error> {
        let q = FtsQuery::new(
            req,
            Source {
                hit_type: HitType::Goal,
                text_expr: TEXT_EXPR,
                id: Column::Id,
                created_at: Column::CreatedAt,
                updated_at: Column::UpdatedAt,
                relationship_id: Column::CoachingRelationshipId,
                path: vec![Relation::CoachingRelationships.def()],
            },
        );
        let select = q
            .select::<Entity>()
            .column(Column::Title)
            .column(Column::Body)
            .column(Column::Status)
            .column(Column::CreatedInSessionId)
            // `user_id` means creator for goals.
            .apply_if(req.filters.user_id, |q, id| q.filter(Column::UserId.eq(id)))
            // `goal_id` narrows the goals type to the goal itself.
            .apply_if(req.filters.goal_id, |q, id| q.filter(Column::Id.eq(id)))
            .apply_if(req.filters.status.clone(), |q, s| {
                q.filter(Column::Status.eq(s))
            });

        let rows = q.finish(select).into_model::<Row>().all(db).await?;

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
