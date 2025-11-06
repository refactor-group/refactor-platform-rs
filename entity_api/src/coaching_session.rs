use super::error::{EntityApiErrorKind, Error};
use entity::{
    agreements, coaching_relationships,
    coaching_sessions::{self, ActiveModel, Entity, Model, Relation},
    organizations, overarching_goals, users, Id,
};
use log::debug;
use sea_orm::{entity::prelude::*, DatabaseConnection, JoinType, QueryOrder, QuerySelect, Set, TryIntoModel};
use std::collections::HashMap;

pub async fn create(
    db: &DatabaseConnection,
    coaching_session_model: Model,
) -> Result<Model, Error> {
    debug!("New Coaching Session Model to be inserted: {coaching_session_model:?}");

    let now = chrono::Utc::now();

    let coaching_session_active_model: ActiveModel = ActiveModel {
        coaching_relationship_id: Set(coaching_session_model.coaching_relationship_id),
        date: Set(coaching_session_model.date),
        collab_document_name: Set(coaching_session_model.collab_document_name),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(coaching_session_active_model
        .save(db)
        .await?
        .try_into_model()?)
}

pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

pub async fn find_by_id_with_coaching_relationship(
    db: &DatabaseConnection,
    id: Id,
) -> Result<(Model, coaching_relationships::Model), Error> {
    if let Some(results) = Entity::find_by_id(id)
        .find_also_related(coaching_relationships::Entity)
        .one(db)
        .await?
    {
        if let Some(coaching_relationship) = results.1 {
            return Ok((results.0, coaching_relationship));
        }
    }
    Err(Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

pub async fn delete(db: &impl ConnectionTrait, coaching_session_id: Id) -> Result<(), Error> {
    Entity::delete_by_id(coaching_session_id).exec(db).await?;
    Ok(())
}

pub async fn find_by_user(db: &impl ConnectionTrait, user_id: Id) -> Result<Vec<Model>, Error> {
    let sessions = Entity::find()
        .join(JoinType::InnerJoin, Relation::CoachingRelationships.def())
        .filter(
            coaching_relationships::Column::CoachId
                .eq(user_id)
                .or(coaching_relationships::Column::CoacheeId.eq(user_id)),
        )
        .all(db)
        .await?;

    Ok(sessions)
}

/// Public API response type: a single coaching session with its optional related resources.
///
/// # Purpose
/// This struct solves the N+1 query problem when fetching coaching sessions with their
/// related data. Instead of making separate database queries for each session's relationships,
/// users, organizations, etc., this struct enables batch loading all related resources in a
/// single efficient operation.
///
/// **Contrast with `RelatedData`:** This holds specific related data for ONE session (returned to clients),
/// while `RelatedData` holds ALL related data in lookup tables (internal only).
///
/// # Usage Pattern
/// Clients can request specific related resources via query parameters (e.g., `?include=relationship,organization`),
/// and the `find_by_user_with_includes` function will:
/// 1. Fetch all coaching sessions for the user
/// 2. Batch load requested related resources into `RelatedData` using `IN` queries
/// 3. Assemble each `EnrichedSession` by looking up its specific related data
///
/// # Serialization Behavior
/// - The base `session` fields are flattened into the JSON root using `#[serde(flatten)]`
/// - Optional related resources are only included in JSON when present (`skip_serializing_if`)
/// - This allows the same struct to represent sessions with varying levels of detail
///
/// # Example JSON Output
/// ```json
/// {
///   "id": "session-123",
///   "date": "2025-01-15",
///   "relationship": { "id": "rel-456", ... },  // Only if included
///   "coach": { "id": "user-789", ... },        // Only if included
///   "organization": { "id": "org-101", ... }    // Only if included
/// }
/// ```
#[derive(Debug, Clone, serde::Serialize)]
pub struct EnrichedSession {
    #[serde(flatten)]
    pub session: Model,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<coaching_relationships::Model>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coach: Option<users::Model>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coachee: Option<users::Model>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<organizations::Model>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overarching_goal: Option<overarching_goals::Model>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agreement: Option<agreements::Model>,
}

/// Configuration for which related resources to include when fetching coaching sessions.
///
/// # Purpose
/// This struct acts as a feature flag configuration for batch loading related resources.
/// It controls which additional database queries are executed beyond fetching the base
/// coaching sessions, enabling clients to request only the data they need.
///
/// # Design Rationale
/// Using boolean flags instead of an enum allows for:
/// - Multiple resources to be requested simultaneously
/// - Fine-grained control over query execution
/// - Easy addition of new optional resources without breaking changes
/// - Zero-cost abstraction (Copy trait) for passing options around
///
/// # Relationship Dependencies
/// Some resources have dependencies on others due to database foreign key relationships:
/// - `organization` requires `relationship` because organizations are linked via coaching_relationships
/// - The `validate()` method enforces these constraints at the API boundary
///
/// # Usage Example
/// ```rust
/// // Create options requesting relationship and organization data
/// let mut options = IncludeOptions::none();
/// options.relationship = true;
/// options.organization = true;
/// options.validate()?; // Passes: organization depends on relationship
///
/// // This would fail validation:
/// let mut invalid = IncludeOptions::none();
/// invalid.organization = true;  // Without relationship: true
/// invalid.validate()?; // Error: organization requires relationship
/// ```
#[derive(Debug, Clone, Copy)]
pub struct IncludeOptions {
    pub relationship: bool,
    pub organization: bool,
    pub goal: bool,
    pub agreements: bool,
}

impl IncludeOptions {
    /// Creates an `IncludeOptions` with all resources disabled.
    ///
    /// This is the baseline configuration - only the base coaching session data
    /// will be fetched without any related resources.
    pub fn none() -> Self {
        Self {
            relationship: false,
            organization: false,
            goal: false,
            agreements: false,
        }
    }

    /// Returns true if any option requires loading coaching_relationships data.
    ///
    /// This helper method determines whether we need to execute the batch query
    /// for coaching_relationships. Currently, both the `relationship` and `organization`
    /// options require this data (organization is accessed via relationship.organization_id).
    pub fn needs_relationships(&self) -> bool {
        self.relationship || self.organization
    }

    /// Validates that the include options form a valid dependency graph.
    ///
    /// # Validation Rules
    /// - `organization = true` requires `relationship = true`
    ///   (organizations are accessed through coaching_relationships)
    ///
    /// # Errors
    /// Returns `EntityApiErrorKind::InvalidQueryTerm` if validation fails.
    ///
    /// # Why This Matters
    /// Early validation at the entity_api layer prevents invalid database queries
    /// and provides clear error messages to clients about invalid include combinations.
    pub fn validate(&self) -> Result<(), Error> {
        // organization requires relationship (can't get org without relationship)
        if self.organization && !self.relationship {
            return Err(Error {
                source: None,
                error_kind: EntityApiErrorKind::InvalidQueryTerm,
            });
        }
        Ok(())
    }
}

/// Sort field for coaching sessions
#[derive(Debug, Clone, Copy)]
pub enum SortField {
    Date,
    CreatedAt,
    UpdatedAt,
}

/// Sort order for queries
#[derive(Debug, Clone, Copy)]
pub enum SortOrder {
    Asc,
    Desc,
}

/// Find sessions by user with optional date filtering, sorting, and related data includes
pub async fn find_by_user_with_includes(
    db: &impl ConnectionTrait,
    user_id: Id,
    from_date: Option<chrono::NaiveDate>,
    to_date: Option<chrono::NaiveDate>,
    sort_by: Option<SortField>,
    sort_order: Option<SortOrder>,
    includes: IncludeOptions,
) -> Result<Vec<EnrichedSession>, Error> {
    // Validate include options
    includes.validate()?;

    // Load base sessions with date filtering and sorting
    let sessions = load_sessions_for_user(db, user_id, from_date, to_date, sort_by, sort_order).await?;

    // Early return if no includes requested
    if !includes.needs_relationships() && !includes.goal && !includes.agreements {
        return Ok(sessions.into_iter().map(EnrichedSession::from_session).collect());
    }

    // Load all related data in efficient batches
    let related_data = load_related_data(db, &sessions, includes).await?;

    // Assemble enriched sessions
    Ok(sessions
        .into_iter()
        .map(|session| assemble_enriched_session(session, &related_data))
        .collect())
}

/// Load base sessions filtered by user, optional date range, and sorting
async fn load_sessions_for_user(
    db: &impl ConnectionTrait,
    user_id: Id,
    from_date: Option<chrono::NaiveDate>,
    to_date: Option<chrono::NaiveDate>,
    sort_by: Option<SortField>,
    sort_order: Option<SortOrder>,
) -> Result<Vec<Model>, Error> {
    let mut query = Entity::find()
        .join(JoinType::InnerJoin, Relation::CoachingRelationships.def())
        .filter(
            coaching_relationships::Column::CoachId
                .eq(user_id)
                .or(coaching_relationships::Column::CoacheeId.eq(user_id)),
        );

    if let Some(from) = from_date {
        query = query.filter(coaching_sessions::Column::Date.gte(from));
    }

    if let Some(to) = to_date {
        // Use next day with less-than for inclusive end date
        let end_of_day = to.succ_opt().unwrap_or(to);
        query = query.filter(coaching_sessions::Column::Date.lt(end_of_day));
    }

    // Apply sorting if both field and order are provided
    if let (Some(field), Some(order)) = (sort_by, sort_order) {
        let sea_order = match order {
            SortOrder::Asc => sea_orm::Order::Asc,
            SortOrder::Desc => sea_orm::Order::Desc,
        };

        query = match field {
            SortField::Date => query.order_by(coaching_sessions::Column::Date, sea_order),
            SortField::CreatedAt => query.order_by(coaching_sessions::Column::CreatedAt, sea_order),
            SortField::UpdatedAt => query.order_by(coaching_sessions::Column::UpdatedAt, sea_order),
        };
    }

    query.all(db).await.map_err(Into::into)
}

/// Internal lookup tables for batch-loaded related data (not serialized).
///
/// This struct stores ALL related resources in HashMaps for O(1) lookup during assembly.
/// Contrast with `EnrichedSession`, which holds the specific related data for a single session.
///
/// **Usage:** Temporary container used only within `find_by_user_with_includes`:
/// 1. Load phase: Execute bulk queries, populate these HashMaps
/// 2. Assembly phase: For each session, lookup its specific related data by ID
/// 3. Discard: This struct is not returned to clients
#[derive(Debug, Default)]
struct RelatedData {
    relationships: HashMap<Id, coaching_relationships::Model>,
    coaches: HashMap<Id, users::Model>,
    coachees: HashMap<Id, users::Model>,
    organizations: HashMap<Id, organizations::Model>,
    goals: HashMap<Id, overarching_goals::Model>,
    agreements: HashMap<Id, agreements::Model>,
}

/// Load all requested related data in efficient batches
async fn load_related_data(
    db: &impl ConnectionTrait,
    sessions: &[Model],
    includes: IncludeOptions,
) -> Result<RelatedData, Error> {
    let mut data = RelatedData::default();

    // Extract IDs for batch loading
    let relationship_ids: Vec<Id> = sessions.iter().map(|s| s.coaching_relationship_id).collect();
    let session_ids: Vec<Id> = sessions.iter().map(|s| s.id).collect();

    // Load relationships (needed for both relationship and organization includes)
    if includes.needs_relationships() {
        data.relationships = batch_load_relationships(db, &relationship_ids).await?;
    }

    // Load users (coaches and coachees) if relationship is included
    if includes.relationship {
        let coach_ids: Vec<Id> = data.relationships.values().map(|r| r.coach_id).collect();
        let coachee_ids: Vec<Id> = data.relationships.values().map(|r| r.coachee_id).collect();

        data.coaches = batch_load_users(db, &coach_ids).await?;
        data.coachees = batch_load_users(db, &coachee_ids).await?;
    }

    // Load organizations if requested
    if includes.organization {
        let org_ids: Vec<Id> = data.relationships.values().map(|r| r.organization_id).collect();
        data.organizations = batch_load_organizations(db, &org_ids).await?;
    }

    // Load goals by session_id
    if includes.goal {
        data.goals = batch_load_goals(db, &session_ids).await?;
    }

    // Load agreements by session_id
    if includes.agreements {
        data.agreements = batch_load_agreements(db, &session_ids).await?;
    }

    Ok(data)
}

/// Batch load coaching relationships by IDs
async fn batch_load_relationships(
    db: &impl ConnectionTrait,
    ids: &[Id],
) -> Result<HashMap<Id, coaching_relationships::Model>, Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    Ok(coaching_relationships::Entity::find()
        .filter(coaching_relationships::Column::Id.is_in(ids.iter().copied()))
        .all(db)
        .await?
        .into_iter()
        .map(|r| (r.id, r))
        .collect())
}

/// Batch load users by IDs
async fn batch_load_users(
    db: &impl ConnectionTrait,
    ids: &[Id],
) -> Result<HashMap<Id, users::Model>, Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    Ok(users::Entity::find()
        .filter(users::Column::Id.is_in(ids.iter().copied()))
        .all(db)
        .await?
        .into_iter()
        .map(|u| (u.id, u))
        .collect())
}

/// Batch load organizations by IDs
async fn batch_load_organizations(
    db: &impl ConnectionTrait,
    ids: &[Id],
) -> Result<HashMap<Id, organizations::Model>, Error> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    Ok(organizations::Entity::find()
        .filter(organizations::Column::Id.is_in(ids.iter().copied()))
        .all(db)
        .await?
        .into_iter()
        .map(|o| (o.id, o))
        .collect())
}

/// Batch load overarching goals by session IDs
async fn batch_load_goals(
    db: &impl ConnectionTrait,
    session_ids: &[Id],
) -> Result<HashMap<Id, overarching_goals::Model>, Error> {
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }

    Ok(overarching_goals::Entity::find()
        .filter(overarching_goals::Column::CoachingSessionId.is_in(session_ids.iter().copied()))
        .all(db)
        .await?
        .into_iter()
        .map(|g| (g.coaching_session_id, g))
        .collect())
}

/// Batch load agreements by session IDs
async fn batch_load_agreements(
    db: &impl ConnectionTrait,
    session_ids: &[Id],
) -> Result<HashMap<Id, agreements::Model>, Error> {
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }

    Ok(agreements::Entity::find()
        .filter(agreements::Column::CoachingSessionId.is_in(session_ids.iter().copied()))
        .all(db)
        .await?
        .into_iter()
        .map(|a| (a.coaching_session_id, a))
        .collect())
}

/// Assemble an enriched session from base session and related data
fn assemble_enriched_session(session: Model, related: &RelatedData) -> EnrichedSession {
    let relationship = related.relationships.get(&session.coaching_relationship_id).cloned();

    let (coach, coachee) = relationship
        .as_ref()
        .map(|rel| {
            (
                related.coaches.get(&rel.coach_id).cloned(),
                related.coachees.get(&rel.coachee_id).cloned(),
            )
        })
        .unwrap_or((None, None));

    let organization = relationship
        .as_ref()
        .and_then(|rel| related.organizations.get(&rel.organization_id).cloned());

    let overarching_goal = related.goals.get(&session.id).cloned();
    let agreement = related.agreements.get(&session.id).cloned();

    EnrichedSession {
        session,
        relationship,
        coach,
        coachee,
        organization,
        overarching_goal,
        agreement,
    }
}

impl EnrichedSession {
    /// Create an enriched session from just the base session model
    fn from_session(session: Model) -> Self {
        Self {
            session,
            relationship: None,
            coach: None,
            coachee: None,
            organization: None,
            overarching_goal: None,
            agreement: None,
        }
    }
}

#[cfg(test)]
// We need to gate seaORM's mock feature behind conditional compilation because
// the feature removes the Clone trait implementation from seaORM's DatabaseConnection.
// see https://github.com/SeaQL/sea-orm/issues/830
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::Id;
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

    #[tokio::test]
    async fn create_returns_a_new_coaching_session_model() -> Result<(), Error> {
        let now = chrono::Utc::now();

        let coaching_session_model = Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            date: chrono::Local::now().naive_utc(),
            collab_document_name: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![coaching_session_model.clone()]])
            .into_connection();

        let coaching_session = create(&db, coaching_session_model.clone().into()).await?;

        assert_eq!(coaching_session.id, coaching_session_model.id);

        Ok(())
    }

    #[tokio::test]
    async fn find_by_id_returns_a_single_record() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let coaching_session_id = Id::new_v4();
        let _ = find_by_id(&db, coaching_session_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."collab_document_name", "coaching_sessions"."date", "coaching_sessions"."created_at", "coaching_sessions"."updated_at" FROM "refactor_platform"."coaching_sessions" WHERE "coaching_sessions"."id" = $1 LIMIT $2"#,
                [
                    coaching_session_id.into(),
                    sea_orm::Value::BigUnsigned(Some(1))
                ]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_id_with_coaching_relationship_returns_a_single_record() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let coaching_session_id = Id::new_v4();
        let _ = find_by_id_with_coaching_relationship(&db, coaching_session_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_sessions"."id" AS "A_id", "coaching_sessions"."coaching_relationship_id" AS "A_coaching_relationship_id", "coaching_sessions"."collab_document_name" AS "A_collab_document_name", "coaching_sessions"."date" AS "A_date", "coaching_sessions"."created_at" AS "A_created_at", "coaching_sessions"."updated_at" AS "A_updated_at", "coaching_relationships"."id" AS "B_id", "coaching_relationships"."organization_id" AS "B_organization_id", "coaching_relationships"."coach_id" AS "B_coach_id", "coaching_relationships"."coachee_id" AS "B_coachee_id", "coaching_relationships"."slug" AS "B_slug", "coaching_relationships"."created_at" AS "B_created_at", "coaching_relationships"."updated_at" AS "B_updated_at" FROM "refactor_platform"."coaching_sessions" LEFT JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE "coaching_sessions"."id" = $1 LIMIT $2"#,
                [
                    coaching_session_id.into(),
                    sea_orm::Value::BigUnsigned(Some(1))
                ]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn delete_deletes_a_single_record() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let coaching_session_id = Id::new_v4();
        let _ = delete(&db, coaching_session_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"DELETE FROM "refactor_platform"."coaching_sessions" WHERE "coaching_sessions"."id" = $1"#,
                [coaching_session_id.into(),]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_user_returns_sessions_where_user_is_coach_or_coachee() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let user_id = Id::new_v4();
        let _ = find_by_user(&db, user_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."collab_document_name", "coaching_sessions"."date", "coaching_sessions"."created_at", "coaching_sessions"."updated_at" FROM "refactor_platform"."coaching_sessions" INNER JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE "coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2"#,
                [user_id.into(), user_id.into()]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_user_with_includes_no_includes_returns_basic_sessions() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let user_id = Id::new_v4();
        let session_id = Id::new_v4();
        let relationship_id = Id::new_v4();

        let session = Model {
            id: session_id,
            coaching_relationship_id: relationship_id,
            date: chrono::Local::now().naive_utc(),
            collab_document_name: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![session.clone()]])
            .into_connection();

        let includes = IncludeOptions::none();
        let results = find_by_user_with_includes(&db, user_id, None, None, None, None, includes).await?;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session.id, session_id);
        assert!(results[0].relationship.is_none());
        assert!(results[0].organization.is_none());
        assert!(results[0].overarching_goal.is_none());
        assert!(results[0].agreement.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn find_by_user_with_includes_with_date_filters() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let user_id = Id::new_v4();
        let from_date = chrono::NaiveDate::from_ymd_opt(2025, 10, 26).unwrap();
        let to_date = chrono::NaiveDate::from_ymd_opt(2025, 10, 27).unwrap();

        // Create a session within the date range
        let session = Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            date: from_date.into(),
            collab_document_name: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![session.clone()]])
            .into_connection();

        let includes = IncludeOptions::none();
        let results = find_by_user_with_includes(&db, user_id, Some(from_date), Some(to_date), None, None, includes).await?;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session.id, session.id);
        assert_eq!(results[0].session.date, from_date.into());

        Ok(())
    }

    #[tokio::test]
    async fn include_options_needs_relationships_returns_true_when_relationship_included() {
        let includes = IncludeOptions {
            relationship: true,
            organization: false,
            goal: false,
            agreements: false,
        };
        assert!(includes.needs_relationships());
    }

    #[tokio::test]
    async fn include_options_needs_relationships_returns_true_when_organization_included() {
        let includes = IncludeOptions {
            relationship: false,
            organization: true,
            goal: false,
            agreements: false,
        };
        assert!(includes.needs_relationships());
    }

    #[tokio::test]
    async fn include_options_needs_relationships_returns_false_when_only_goals() {
        let includes = IncludeOptions {
            relationship: false,
            organization: false,
            goal: true,
            agreements: false,
        };
        assert!(!includes.needs_relationships());
    }

    #[tokio::test]
    async fn enriched_session_from_session_creates_empty_enrichment() {
        let now = chrono::Utc::now();
        let session = Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            date: chrono::Local::now().naive_utc(),
            collab_document_name: None,
            created_at: now.into(),
            updated_at: now.into(),
        };

        let enriched = EnrichedSession::from_session(session.clone());

        assert_eq!(enriched.session.id, session.id);
        assert!(enriched.relationship.is_none());
        assert!(enriched.coach.is_none());
        assert!(enriched.coachee.is_none());
        assert!(enriched.organization.is_none());
        assert!(enriched.overarching_goal.is_none());
        assert!(enriched.agreement.is_none());
    }

    #[test]
    fn validate_allows_organization_with_relationship() {
        let includes = IncludeOptions {
            relationship: true,
            organization: true,
            goal: false,
            agreements: false,
        };
        assert!(includes.validate().is_ok());
    }

    #[test]
    fn validate_rejects_organization_without_relationship() {
        let includes = IncludeOptions {
            relationship: false,
            organization: true,
            goal: false,
            agreements: false,
        };
        assert!(includes.validate().is_err());
    }

    #[test]
    fn validate_allows_goal_alone() {
        let includes = IncludeOptions {
            relationship: false,
            organization: false,
            goal: true,
            agreements: false,
        };
        assert!(includes.validate().is_ok());
    }

    #[test]
    fn validate_allows_all_includes() {
        let includes = IncludeOptions {
            relationship: true,
            organization: true,
            goal: true,
            agreements: true,
        };
        assert!(includes.validate().is_ok());
    }

    #[test]
    fn validate_allows_none() {
        let includes = IncludeOptions::none();
        assert!(includes.validate().is_ok());
    }
}
