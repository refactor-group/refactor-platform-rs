//! Topic searcher: FTS over `coaching_session_topics.body`, anchored to a
//! relationship through the owning coaching session. Soft-deleted topics are
//! excluded (lifecycle filtering, not authorization).

use async_trait::async_trait;
use sea_orm::entity::prelude::DateTimeWithTimeZone;
use sea_orm::{
    ColumnTrait, DatabaseConnection, FromQueryResult, QueryFilter, QuerySelect, QueryTrait,
    RelationTrait,
};

use super::{excerpt_title, Core, FtsQuery, Hit, HitType, Request, Source, TopicHit};
use crate::error::Error;
use entity::coaching_session_topics::{Column, Entity, Relation};
use entity::topic_priority::Priority;
use entity::topic_status::Status;
use entity::{coaching_sessions, Id};

/// Raw-text source; its tsvector form must stay semantically identical to
/// `idx_coaching_session_topics_body_fts`.
pub(super) const TEXT_EXPR: &str = r#"coalesce("coaching_session_topics"."body", '')"#;

#[derive(Debug, FromQueryResult)]
struct Row {
    id: Id,
    score: f32,
    body: String,
    status: Status,
    priority: Option<Priority>,
    coaching_session_id: Id,
    created_at: DateTimeWithTimeZone,
    updated_at: DateTimeWithTimeZone,
    coaching_relationship_id: Id,
    organization_id: Id,
}

pub struct Searcher;

#[async_trait]
impl super::Searcher for Searcher {
    fn hit_type(&self) -> HitType {
        HitType::Topic
    }

    async fn search(&self, db: &DatabaseConnection, req: &Request<'_>) -> Result<Vec<Hit>, Error> {
        let q = FtsQuery::new(
            req,
            Source {
                hit_type: HitType::Topic,
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
            .column(Column::Status)
            .column(Column::Priority)
            .column(Column::CoachingSessionId)
            .filter(Column::DeletedAt.is_null())
            // `user_id` means creator for topics.
            .apply_if(req.filters.user_id, |q, id| q.filter(Column::UserId.eq(id)))
            .apply_if(req.filters.coaching_session_id, |q, id| {
                q.filter(Column::CoachingSessionId.eq(id))
            })
            .apply_if(req.filters.topic_status.clone(), |q, s| {
                q.filter(Column::Status.eq(s))
            });

        let rows = q.finish(select).into_model::<Row>().all(db).await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                Hit::Topic(TopicHit {
                    core: Core {
                        id: r.id,
                        score: r.score,
                        title: excerpt_title(Some(&r.body), "Topic"),
                        snippet: None,
                        created_at: r.created_at,
                        updated_at: Some(r.updated_at),
                        organization_id: r.organization_id,
                    },
                    coaching_session_id: r.coaching_session_id,
                    coaching_relationship_id: r.coaching_relationship_id,
                    status: r.status,
                    priority: r.priority,
                })
            })
            .collect())
    }
}
