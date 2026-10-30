//! Agreement searcher: FTS over `agreements.body`, anchored to a relationship
//! through the owning coaching session.

use async_trait::async_trait;
use sea_orm::entity::prelude::{DateTime, DateTimeWithTimeZone};
use sea_orm::{
    ColumnTrait, DatabaseConnection, FromQueryResult, QueryFilter, QuerySelect, QueryTrait,
    RelationTrait,
};

use super::{excerpt_title, AgreementHit, Core, FtsQuery, Hit, HitType, Request, Source};
use crate::error::Error;
use entity::agreements::{Column, Entity, Relation};
use entity::{coaching_sessions, Id};

/// Raw-text source; its tsvector form must stay semantically identical to
/// `idx_agreements_body_fts`.
pub(super) const TEXT_EXPR: &str = r#"coalesce("agreements"."body", '')"#;

#[derive(Debug, FromQueryResult)]
struct Row {
    id: Id,
    score: f32,
    body: Option<String>,
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
        HitType::Agreement
    }

    async fn search(&self, db: &DatabaseConnection, req: &Request<'_>) -> Result<Vec<Hit>, Error> {
        let q = FtsQuery::new(
            req,
            Source {
                hit_type: HitType::Agreement,
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
            .column(Column::CoachingSessionId)
            .column_as(coaching_sessions::Column::Date, "session_date")
            // `user_id` means creator for agreements.
            .apply_if(req.filters.user_id, |q, id| q.filter(Column::UserId.eq(id)))
            .apply_if(req.filters.coaching_session_id, |q, id| {
                q.filter(Column::CoachingSessionId.eq(id))
            });

        let rows = q.finish(select).into_model::<Row>().all(db).await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                Hit::Agreement(AgreementHit {
                    core: Core {
                        id: r.id,
                        score: r.score,
                        title: excerpt_title(r.body.as_deref(), "Agreement"),
                        snippet: None,
                        created_at: r.created_at,
                        updated_at: Some(r.updated_at),
                        organization_id: r.organization_id,
                    },
                    coaching_session_id: r.coaching_session_id,
                    coaching_relationship_id: r.coaching_relationship_id,
                    session_date: r.session_date,
                    session_title: String::new(),
                })
            })
            .collect())
    }
}
