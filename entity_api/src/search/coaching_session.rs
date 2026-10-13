//! Coaching-session searcher: FTS over `coaching_sessions.title`.

use async_trait::async_trait;
use sea_orm::entity::prelude::{DateTime, DateTimeWithTimeZone};
use sea_orm::{
    ColumnTrait, DatabaseConnection, FromQueryResult, QueryFilter, QuerySelect, QueryTrait,
    RelationTrait,
};

use super::{Core, FtsQuery, Hit, HitType, Request, SessionHit, Source};
use crate::error::Error;
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

pub struct Searcher;

#[async_trait]
impl super::Searcher for Searcher {
    fn hit_type(&self) -> HitType {
        HitType::CoachingSession
    }

    async fn search(&self, db: &DatabaseConnection, req: &Request<'_>) -> Result<Vec<Hit>, Error> {
        let q = FtsQuery::new(
            req,
            Source {
                hit_type: HitType::CoachingSession,
                text_expr: TEXT_EXPR,
                id: Column::Id,
                created_at: Column::CreatedAt,
                updated_at: Column::UpdatedAt,
                relationship_id: Column::CoachingRelationshipId,
                path: vec![Relation::CoachingRelationships.def()],
            },
        )
        // "By user" means participant for sessions, so the domain pre-narrowed set applies.
        .participant_scoped();
        let select = q
            .select::<Entity>()
            .column(Column::Date)
            .apply_if(req.filters.coaching_session_id, |q, id| {
                q.filter(Column::Id.eq(id))
            });

        let rows = q.finish(select).into_model::<Row>().all(db).await?;

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
                })
            })
            .collect())
    }
}
