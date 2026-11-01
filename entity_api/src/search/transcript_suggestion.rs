//! Transcript suggestion searcher: FTS over the proposal body of *pending*
//! suggestions, anchored to a relationship through the owning coaching session.
//! Accepted suggestions are found as the actions they became; dismissed ones
//! were rejected. Both are lifecycle filtering, not authorization.

use async_trait::async_trait;
use sea_orm::entity::prelude::{DateTime, DateTimeWithTimeZone};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseConnection, FromQueryResult, QueryFilter, QuerySelect, QueryTrait,
    RelationTrait,
};

use super::{
    excerpt_title, Core, FtsQuery, Hit, HitType, Request, Source, TranscriptSuggestionHit,
};
use crate::error::Error;
use entity::transcript_suggestion::{Column, Entity, Relation};
use entity::transcript_suggestion_target::Target;
use entity::{coaching_sessions, Id};

/// Raw-text source; its tsvector form must stay semantically identical to
/// `idx_transcript_suggestions_pending_fts`.
pub(super) const TEXT_EXPR: &str =
    r#"coalesce("transcript_suggestions"."proposal" ->> 'body', '')"#;

/// The partial index predicate, mirroring the pending listing in
/// `transcript_suggestion::find_by_session`. Literal SQL on purpose: a bound
/// `status` parameter can't be proven to imply the index predicate under a
/// generic plan, and the planner would silently stop using the index.
const PENDING_PREDICATE: &str = r#""transcript_suggestions"."status" = 'pending' AND "transcript_suggestions"."created_entity_id" IS NULL"#;

#[derive(Debug, FromQueryResult)]
struct Row {
    id: Id,
    score: f32,
    body: String,
    target: Target,
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
        HitType::TranscriptSuggestion
    }

    async fn search(&self, db: &DatabaseConnection, req: &Request<'_>) -> Result<Vec<Hit>, Error> {
        // Suggestions are model-authored, so "by user" means session
        // participant, as it does for sessions.
        let q = FtsQuery::new(
            req,
            Source {
                hit_type: HitType::TranscriptSuggestion,
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
        )
        .participant_scoped();
        let select = q
            .select::<Entity>()
            .column_as(Expr::cust(TEXT_EXPR), "body")
            .column(Column::Target)
            .column(Column::CoachingSessionId)
            .column_as(coaching_sessions::Column::Date, "session_date")
            .filter(Expr::cust(PENDING_PREDICATE))
            .apply_if(req.filters.coaching_session_id, |q, id| {
                q.filter(Column::CoachingSessionId.eq(id))
            });

        let rows = q.finish(select).into_model::<Row>().all(db).await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                Hit::TranscriptSuggestion(TranscriptSuggestionHit {
                    core: Core {
                        id: r.id,
                        score: r.score,
                        title: excerpt_title(Some(&r.body), title_fallback(r.target)),
                        snippet: None,
                        created_at: r.created_at,
                        updated_at: Some(r.updated_at),
                        organization_id: r.organization_id,
                    },
                    target: r.target,
                    coaching_session_id: r.coaching_session_id,
                    coaching_relationship_id: r.coaching_relationship_id,
                    session_date: r.session_date,
                    session_title: String::new(),
                })
            })
            .collect())
    }
}

/// Row label when the proposal body is blank; exhaustive so a new target
/// must name its own.
fn title_fallback(target: Target) -> &'static str {
    match target {
        Target::Action => "Suggested action",
    }
}
