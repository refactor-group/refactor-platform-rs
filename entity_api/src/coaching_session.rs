use super::error::{EntityApiErrorKind, Error};
use crate::duration::Duration;
use crate::mutate::UpdateMap;
use entity::{
    agreements, coaching_relationships, coaching_session_topics,
    coaching_sessions::{self, ActiveModel, Column, Entity, Model, Relation},
    goals, organizations,
    provider::Provider,
    users, Id,
};
use log::debug;
use sea_orm::{
    entity::prelude::*, sea_query::Expr, ActiveValue::Unchanged, ConnectionTrait, DatabaseBackend,
    DatabaseConnection, FromQueryResult, JoinType, Order, QueryOrder, QuerySelect, QueryTrait,
    Select, Set, Statement, TryIntoModel, Value,
};
use serde::Serialize;
use std::collections::HashMap;
use utoipa::ToSchema;

/// Resolve the duration for a new coaching session via the defaulting cascade.
///
/// - `Some(d)` returns `d` directly (already validated by the type).
/// - `None` loads the coach and uses their
///   `default_coaching_session_duration_minutes` (valid by DB invariant — the
///   column is `NOT NULL DEFAULT 60` and the only writes go through
///   `Duration::new`).
pub async fn resolve_duration(
    db: &impl ConnectionTrait,
    coach_id: Id,
    requested: Option<Duration>,
) -> Result<Duration, Error> {
    if let Some(d) = requested {
        return Ok(d);
    }
    let coach = crate::user::find_by_id(db, coach_id).await?;
    Ok(Duration::from_minutes_unchecked(
        coach.default_coaching_session_duration_minutes,
    ))
}

/// Validate a duration-bearing entry of an update map if present.
///
/// - `Ok(Some(dur))` — key is present and in range.
/// - `Ok(None)` — key is absent, or the `Value` variant is not `SmallInt`
///   (an upstream `IntoUpdateMap` bug; the SQL layer catches it).
/// - `Err(_)` — key present but outside `1..=480`. The entity-level
///   `OutOfRange` is wrapped in `EntityApiError` here (rather than
///   returned bare) so domain callers propagate via the standard
///   `From<EntityApiError> for domain::Error` boundary, which short-
///   circuits `EntityApiErrorKind::OutOfRange` to
///   `DomainErrorKind::Validation` → 422.
///
/// `key` is the update-map key holding the duration. Reused for both
/// `coaching_sessions.duration_minutes` and
/// `users.default_coaching_session_duration_minutes`.
pub fn validate_duration_in_update_map(
    update_map: &UpdateMap,
    key: &str,
) -> Result<Option<Duration>, Error> {
    let Some(Value::SmallInt(Some(n))) = update_map.get_value(key) else {
        return Ok(None);
    };
    Duration::try_from(*n).map(Some).map_err(Error::from)
}

/// Insert a new coaching session.
///
/// `requested_duration` resolves via the defaulting cascade (see
/// `resolve_duration`). Any value already on `coaching_session_model.duration_minutes`
/// is ignored — the resolved `Duration` wins.
pub async fn create(
    db: &impl ConnectionTrait,
    coaching_session_model: Model,
    coach_id: Id,
    requested_duration: Option<Duration>,
) -> Result<Model, Error> {
    debug!("New Coaching Session Model to be inserted: {coaching_session_model:?}");

    let duration = resolve_duration(db, coach_id, requested_duration).await?;
    let now = chrono::Utc::now();

    let coaching_session_active_model: ActiveModel = ActiveModel {
        coaching_relationship_id: Set(coaching_session_model.coaching_relationship_id),
        date: Set(coaching_session_model.date),
        duration_minutes: Set(duration.minutes()),
        collab_document_name: Set(coaching_session_model.collab_document_name),
        meeting_url: Set(coaching_session_model.meeting_url),
        provider: Set(coaching_session_model.provider),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        hydrated_at: Set(coaching_session_model.hydrated_at),
        ..Default::default()
    };

    Ok(coaching_session_active_model
        .save(db)
        .await?
        .try_into_model()?)
}

/// Bulk-insert a recurring series of coaching sessions in a single round-trip.
/// All lazy fields (`provider`, `collab_document_name`, `meeting_url`,
/// `hydrated_at`) are NULL; population happens on first read.
///
/// `requested_duration` resolves once via the cascade and applies to every
/// materialized session.
pub async fn bulk_create_recurring(
    db: &impl ConnectionTrait,
    coaching_relationship_id: Id,
    coach_id: Id,
    dates: Vec<chrono::NaiveDateTime>,
    requested_duration: Option<Duration>,
) -> Result<Vec<Model>, Error> {
    debug!(
        "Bulk-creating {} recurring sessions on relationship {}",
        dates.len(),
        coaching_relationship_id
    );

    if dates.is_empty() {
        return Ok(Vec::new());
    }

    let duration = resolve_duration(db, coach_id, requested_duration).await?;
    let duration_minutes_i16 = duration.minutes();
    let now = chrono::Utc::now();
    let active_models: Vec<ActiveModel> = dates
        .into_iter()
        .map(|date| ActiveModel {
            coaching_relationship_id: Set(coaching_relationship_id),
            date: Set(date),
            duration_minutes: Set(duration_minutes_i16),
            collab_document_name: Set(None),
            meeting_url: Set(None),
            provider: Set(None),
            hydrated_at: Set(None),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
            ..Default::default()
        })
        .collect();

    Ok(Entity::insert_many(active_models)
        .exec_with_returning_many(db)
        .await?)
}

pub async fn find_by_id(db: &impl ConnectionTrait, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Returns the coach and coachee user IDs for a coaching session.
///
/// Used by webhook handlers to determine which users to notify via SSE when
/// recording or transcription state changes. Performs a single join query.
pub async fn find_participant_ids(
    db: &DatabaseConnection,
    coaching_session_id: Id,
) -> Result<Vec<Id>, Error> {
    let (_, relationship) = find_by_id_with_coaching_relationship(db, coaching_session_id).await?;
    Ok(vec![relationship.coach_id, relationship.coachee_id])
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

/// Acquires a transaction-scoped Postgres advisory lock keyed on the session
/// id. Other callers requesting the same lock block until this transaction
/// commits or rolls back. Used by the lazy-hydration path to serialize
/// concurrent first-load requests for the same session.
pub async fn acquire_advisory_lock(
    txn: &impl ConnectionTrait,
    session_id: Id,
) -> Result<(), Error> {
    let key = advisory_lock_key(session_id);
    txn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT pg_advisory_xact_lock($1)",
        [key.into()],
    ))
    .await?;
    Ok(())
}

fn advisory_lock_key(session_id: Id) -> i64 {
    let bytes = session_id.as_bytes();
    let upper = u64::from_be_bytes(bytes[..8].try_into().unwrap());
    let lower = u64::from_be_bytes(bytes[8..].try_into().unwrap());
    (upper ^ lower) as i64
}

/// Persists the resolved deferred fields and stamps `hydrated_at = NOW()`.
/// Called at the end of the lazy-hydration sequence to commit the row's
/// hydrated state. `target` carries the desired final values for the lazy
/// fields; immutable columns are preserved.
pub async fn mark_hydrated(txn: &impl ConnectionTrait, target: &Model) -> Result<Model, Error> {
    let now = chrono::Utc::now();
    let active_model = ActiveModel {
        id: Unchanged(target.id),
        coaching_relationship_id: Unchanged(target.coaching_relationship_id),
        date: Unchanged(target.date),
        duration_minutes: Unchanged(target.duration_minutes),
        collab_document_name: Set(target.collab_document_name.clone()),
        meeting_url: Set(target.meeting_url.clone()),
        provider: Set(target.provider),
        created_at: Unchanged(target.created_at),
        updated_at: Set(now.into()),
        hydrated_at: Set(Some(now.into())),
    };
    Ok(active_model.update(txn).await?.try_into_model()?)
}

pub async fn update_meeting(
    db: &DatabaseConnection,
    id: Id,
    meeting_url: String,
    provider: Provider,
) -> Result<Model, Error> {
    let session = find_by_id(db, id).await?;
    let mut active_model: ActiveModel = session.into();
    active_model.meeting_url = Set(Some(meeting_url));
    active_model.provider = Set(Some(provider));
    active_model.updated_at = Set(chrono::Utc::now().into());

    Ok(active_model.save(db).await?.try_into_model()?)
}

/// Find the most recent meeting URL for a coaching relationship and provider.
///
/// Searches all sessions in the given relationship for one that has a `meeting_url`
/// with the specified `provider`, returning the URL from the most recently created match.
/// Returns `None` if no session in the relationship has a meeting URL for that provider.
pub async fn find_meeting_url_by_relationship_and_provider(
    db: &impl ConnectionTrait,
    coaching_relationship_id: Id,
    provider: Provider,
) -> Result<Option<String>, Error> {
    Ok(Entity::find()
        .filter(Column::CoachingRelationshipId.eq(coaching_relationship_id))
        .filter(Column::Provider.eq(provider))
        .filter(Column::MeetingUrl.is_not_null())
        .order_by_desc(Column::CreatedAt)
        .one(db)
        .await?
        .and_then(|session| session.meeting_url))
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

/// One row of the monthly count aggregation for the user's coaching sessions.
///
/// `month` is `"YYYY-MM"` in the caller-supplied timezone's local calendar; a
/// session at `2026-06-01T02:00:00Z` falls in `"2026-05"` under
/// `America/Los_Angeles` and `"2026-06"` under `Europe/Berlin`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromQueryResult, ToSchema)]
pub struct CountByMonth {
    pub month: String,
    pub count: i64,
}

/// Counts coaching sessions per calendar month for relationships the user
/// participates in (coach or coachee), grouped by `date_trunc('month', date AT
/// TIME ZONE tz_name)`. Months with zero sessions are absent from the result.
///
/// `tz_name` must be a canonical IANA identifier; Postgres `AT TIME ZONE`
/// accepts it directly. Validate upstream (web layer) before calling.
///
/// Date bounds are half-open: `from_date` inclusive, `to_date` inclusive at
/// calendar-day precision (rows up through 23:59:59.999 on `to_date` count).
/// Mirrors the pattern in [`find_by_user_with_includes`].
pub async fn find_counts_by_month_for_user(
    db: &impl ConnectionTrait,
    user_id: Id,
    from_date: chrono::NaiveDate,
    to_date: chrono::NaiveDate,
    tz_name: &str,
    coaching_relationship_id: Option<Id>,
) -> Result<Vec<CountByMonth>, Error> {
    let to_exclusive = to_date.succ_opt().ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::Other("to_date is out of range".to_string()),
    })?;

    // Timezone-shifted month bucket. Postgres accepts canonical IANA names
    // directly via `AT TIME ZONE`; the upstream parse to `chrono_tz::Tz`
    // guarantees the value here is valid.
    //
    // GROUP BY and ORDER BY reference the SELECT alias `"month"` rather than
    // cloning the expression. Cloning would allocate a fresh `$N` placeholder
    // per appearance, and Postgres's grouped-column check compares expression
    // text before bind substitution -- so `$1::text` in SELECT and `$6::text`
    // in GROUP BY are treated as different expressions and the query fails
    // with SQLSTATE 42803. The alias keeps the binding to a single `$1`.
    let month_expr = Expr::cust_with_values(
        r#"to_char(date_trunc('month', "coaching_sessions"."date" AT TIME ZONE $1::text), 'YYYY-MM')"#,
        [tz_name.to_string()],
    );

    let rows = Entity::find()
        .select_only()
        .column_as(month_expr, "month")
        .column_as(Expr::cust("COUNT(*)::bigint"), "count")
        .join(JoinType::InnerJoin, Relation::CoachingRelationships.def())
        .filter(Column::Date.gte(from_date))
        .filter(Column::Date.lt(to_exclusive))
        .filter(
            coaching_relationships::Column::CoachId
                .eq(user_id)
                .or(coaching_relationships::Column::CoacheeId.eq(user_id)),
        )
        .apply_if(coaching_relationship_id, |q: Select<Entity>, rel_id| {
            q.filter(Column::CoachingRelationshipId.eq(rel_id))
        })
        .group_by(Expr::cust(r#""month""#))
        .order_by(Expr::cust(r#""month""#), Order::Asc)
        .into_model::<CountByMonth>()
        .all(db)
        .await?;
    Ok(rows)
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
#[derive(Debug, Clone, serde::Serialize, ToSchema)]
#[schema(as = domain::coaching_session::EnrichedSession)]
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
    pub goals: Option<Vec<goals::Model>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agreement: Option<agreements::Model>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topics: Option<Vec<coaching_session_topics::Model>>,
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
/// use entity_api::coaching_session::IncludeOptions;
///
/// // Create options requesting relationship and organization data
/// let mut options = IncludeOptions::none();
/// options.relationship = true;
/// options.organization = true;
/// options.validate().unwrap(); // Passes: organization depends on relationship
///
/// // This would fail validation:
/// let mut invalid = IncludeOptions::none();
/// invalid.organization = true;  // Without relationship: true
/// assert!(invalid.validate().is_err()); // Error: organization requires relationship
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct IncludeOptions {
    pub relationship: bool,
    pub organization: bool,
    pub goal: bool,
    pub agreements: bool,
    pub topics: bool,
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
            topics: false,
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
/// Query options for finding coaching sessions by user.
///
/// Groups filtering, sorting, and include parameters into a single argument
/// to keep the `find_by_user_with_includes` function signature clean.
#[derive(Debug, Default)]
pub struct SessionQueryOptions {
    /// Filter sessions to only those in this coaching relationship
    pub coaching_relationship_id: Option<Id>,
    /// Filter sessions starting from this date (inclusive). Interpreted in
    /// `tz` when present; otherwise UTC.
    pub from_date: Option<chrono::NaiveDate>,
    /// Filter sessions up to this date (inclusive at calendar-day precision).
    /// Interpreted in `tz` when present; otherwise UTC.
    pub to_date: Option<chrono::NaiveDate>,
    /// Optional canonical IANA timezone name for evaluating `from_date` and
    /// `to_date` as calendar-day boundaries in that zone. `None` = UTC-naive.
    /// Validate upstream (web layer); this layer trusts the value.
    pub tz: Option<String>,
    /// Column to sort results by
    pub sort_column: Option<coaching_sessions::Column>,
    /// Sort direction (ascending or descending)
    pub sort_order: Option<sea_orm::Order>,
    /// Which related resources to include in the response
    pub includes: IncludeOptions,
}

/// Find sessions by user with optional date filtering, sorting, and related data includes
pub async fn find_by_user_with_includes(
    db: &impl ConnectionTrait,
    user_id: Id,
    options: SessionQueryOptions,
) -> Result<Vec<EnrichedSession>, Error> {
    // Validate include options
    options.includes.validate()?;

    // Build the optional date-bound filters up-front so the query chain can
    // stay a single fluent `apply_if` pipeline. When `tz` is supplied, each
    // bound is wrapped in an `AT TIME ZONE` shift, mirroring the expression
    // used by `find_counts_by_month_for_user`; otherwise the legacy
    // `Column::Date.gte/.lt` form is preserved so unchanged callers behave
    // byte-identically.
    let tz = options.tz.as_deref();
    let lower_bound_filter = options.from_date.map(|from| match tz {
        Some(tz_name) => Expr::cust_with_values(
            r#""coaching_sessions"."date" >= ($1::timestamp AT TIME ZONE $2::text) AT TIME ZONE 'UTC'"#,
            [
                sea_orm::Value::from(from),
                sea_orm::Value::from(tz_name.to_string()),
            ],
        ),
        None => coaching_sessions::Column::Date.gte(from),
    });
    let upper_bound_filter = options.to_date.map(|to| {
        let end_of_day = to.succ_opt().unwrap_or(to);
        match tz {
            Some(tz_name) => Expr::cust_with_values(
                r#""coaching_sessions"."date" < ($1::timestamp AT TIME ZONE $2::text) AT TIME ZONE 'UTC'"#,
                [
                    sea_orm::Value::from(end_of_day),
                    sea_orm::Value::from(tz_name.to_string()),
                ],
            ),
            None => coaching_sessions::Column::Date.lt(end_of_day),
        }
    });

    // Single fluent builder chain. Each optional knob lands as an `apply_if`
    // so the "is the knob present" branch never escapes into the surrounding
    // function body.
    let query = Entity::find()
        .join(JoinType::InnerJoin, Relation::CoachingRelationships.def())
        .filter(
            coaching_relationships::Column::CoachId
                .eq(user_id)
                .or(coaching_relationships::Column::CoacheeId.eq(user_id)),
        )
        .apply_if(
            options.coaching_relationship_id,
            |q: Select<Entity>, rel_id| {
                q.filter(coaching_sessions::Column::CoachingRelationshipId.eq(rel_id))
            },
        )
        .apply_if(lower_bound_filter, |q: Select<Entity>, expr| q.filter(expr))
        .apply_if(upper_bound_filter, |q: Select<Entity>, expr| q.filter(expr))
        .apply_if(
            options.sort_column.zip(options.sort_order),
            |q: Select<Entity>, (col, ord)| q.order_by(col, ord),
        );

    // Execute query to load base sessions
    let sessions = query.all(db).await?;

    // Early return if no includes requested
    if !options.includes.needs_relationships()
        && !options.includes.goal
        && !options.includes.agreements
        && !options.includes.topics
    {
        return Ok(sessions
            .into_iter()
            .map(EnrichedSession::from_session)
            .collect());
    }

    // Load all related data in efficient batches
    let related_data = load_related_data(db, &sessions, options.includes).await?;

    // Assemble enriched sessions
    Ok(sessions
        .into_iter()
        .map(|session| assemble_enriched_session(session, &related_data, options.includes))
        .collect())
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
    goals: HashMap<Id, Vec<goals::Model>>,
    agreements: HashMap<Id, agreements::Model>,
    topics: HashMap<Id, Vec<coaching_session_topics::Model>>,
}

/// Load all requested related data in efficient batches
async fn load_related_data(
    db: &impl ConnectionTrait,
    sessions: &[Model],
    includes: IncludeOptions,
) -> Result<RelatedData, Error> {
    let mut data = RelatedData::default();

    // Extract IDs for batch loading
    let relationship_ids: Vec<Id> = sessions
        .iter()
        .map(|s| s.coaching_relationship_id)
        .collect();
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
        let org_ids: Vec<Id> = data
            .relationships
            .values()
            .map(|r| r.organization_id)
            .collect();
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

    // Load topics by session_id
    if includes.topics {
        data.topics = batch_load_topics(db, &session_ids).await?;
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

/// Batch load goals by session IDs via the coaching_sessions_goals join table.
///
/// Delegates to [`super::coaching_session_goal::find_goals_grouped_by_session_ids`] for
/// the DB query and grouping, then caps at [`super::goal::max_in_progress_goals`] per session.
async fn batch_load_goals(
    db: &impl ConnectionTrait,
    session_ids: &[Id],
) -> Result<HashMap<Id, Vec<goals::Model>>, Error> {
    let all_goals =
        super::coaching_session_goal::find_goals_grouped_by_session_ids(db, session_ids).await?;

    let max_goals = super::goal::max_in_progress_goals();

    let map: HashMap<Id, Vec<goals::Model>> = all_goals
        .into_iter()
        .map(|(session_id, goals)| {
            let capped: Vec<_> = goals.into_iter().take(max_goals).collect();
            (session_id, capped)
        })
        .collect();

    debug!(
        "batch_load_goals: loaded goals for {} of {} sessions",
        map.len(),
        session_ids.len()
    );

    Ok(map)
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

/// Batch load topics by session IDs, pre-sorted by display_order then created_at.
async fn batch_load_topics(
    db: &impl ConnectionTrait,
    session_ids: &[Id],
) -> Result<HashMap<Id, Vec<coaching_session_topics::Model>>, Error> {
    if session_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut map: HashMap<Id, Vec<coaching_session_topics::Model>> = HashMap::new();
    for topic in coaching_session_topics::Entity::find()
        .filter(
            coaching_session_topics::Column::CoachingSessionId.is_in(session_ids.iter().copied()),
        )
        .order_by_asc(coaching_session_topics::Column::DisplayOrder)
        .order_by_asc(coaching_session_topics::Column::CreatedAt)
        .all(db)
        .await?
    {
        map.entry(topic.coaching_session_id)
            .or_default()
            .push(topic);
    }
    Ok(map)
}

/// Assemble an enriched session from base session and related data.
///
/// `includes` is needed to distinguish "not requested" (`None`) from
/// "requested but empty" (`Some(vec![])`) so the frontend can tell
/// the difference instead of getting stuck in a loading state.
fn assemble_enriched_session(
    session: Model,
    related: &RelatedData,
    includes: IncludeOptions,
) -> EnrichedSession {
    let relationship = related
        .relationships
        .get(&session.coaching_relationship_id)
        .cloned();

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

    let goals = if includes.goal {
        Some(related.goals.get(&session.id).cloned().unwrap_or(vec![]))
    } else {
        None
    };

    let agreement = related.agreements.get(&session.id).cloned();

    let topics = if includes.topics {
        Some(related.topics.get(&session.id).cloned().unwrap_or_default())
    } else {
        None
    };

    EnrichedSession {
        session,
        relationship,
        coach,
        coachee,
        organization,
        goals,
        agreement,
        topics,
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
            goals: None,
            agreement: None,
            topics: None,
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
    use entity::provider::Provider;
    use entity::Id;
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

    #[tokio::test]
    async fn bulk_create_recurring_inserts_all_rows_with_lazy_fields_null() -> Result<(), Error> {
        let relationship_id = Id::new_v4();
        let now = chrono::Utc::now();
        let dates = vec![
            chrono::NaiveDate::from_ymd_opt(2026, 6, 1)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap(),
            chrono::NaiveDate::from_ymd_opt(2026, 6, 8)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap(),
        ];
        let session1 = Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            date: dates[0],
            collab_document_name: None,
            duration_minutes: crate::duration::Duration::default_minutes(),
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
        };
        let session2 = Model {
            id: Id::new_v4(),
            date: dates[1],
            ..session1.clone()
        };

        // One round-trip: a single INSERT ... VALUES (...), (...) RETURNING * .
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![session1.clone(), session2.clone()]])
            .into_connection();

        let inserted = bulk_create_recurring(
            &db,
            relationship_id,
            Id::new_v4(),
            dates,
            Some(Duration::default()),
        )
        .await?;
        assert_eq!(inserted.len(), 2);
        assert!(inserted.iter().all(|s| s.provider.is_none()));
        assert!(inserted.iter().all(|s| s.collab_document_name.is_none()));
        assert!(inserted.iter().all(|s| s.meeting_url.is_none()));
        assert!(inserted.iter().all(|s| s.hydrated_at.is_none()));
        Ok(())
    }

    #[tokio::test]
    async fn bulk_create_recurring_returns_empty_for_no_dates() -> Result<(), Error> {
        // No mock query expected — the function short-circuits before touching the DB.
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let result = bulk_create_recurring(&db, Id::new_v4(), Id::new_v4(), vec![], None).await?;
        assert!(result.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn mark_hydrated_writes_lazy_fields_and_stamps_hydrated_at() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let target = Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            date: chrono::NaiveDate::from_ymd_opt(2026, 6, 1)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap(),
            collab_document_name: Some("doc-name".to_string()),
            duration_minutes: crate::duration::Duration::default_minutes(),
            meeting_url: Some("https://meet.example/x".to_string()),
            provider: Some(Provider::Zoom),
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: None,
        };
        // Whatever the DB returns from UPDATE ... RETURNING.
        let after = Model {
            hydrated_at: Some(now.into()),
            ..target.clone()
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![after.clone()]])
            .into_connection();

        let result = mark_hydrated(&db, &target).await?;
        assert_eq!(result.id, target.id);
        assert_eq!(result.collab_document_name, target.collab_document_name);
        assert_eq!(result.meeting_url, target.meeting_url);
        assert_eq!(result.provider, target.provider);
        assert!(result.hydrated_at.is_some());
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
                r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."collab_document_name", "coaching_sessions"."date", "coaching_sessions"."duration_minutes", "coaching_sessions"."meeting_url", CAST("coaching_sessions"."provider" AS "text"), "coaching_sessions"."created_at", "coaching_sessions"."updated_at", "coaching_sessions"."hydrated_at" FROM "refactor_platform"."coaching_sessions" WHERE "coaching_sessions"."id" = $1 LIMIT $2"#,
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
                r#"SELECT "coaching_sessions"."id" AS "A_id", "coaching_sessions"."coaching_relationship_id" AS "A_coaching_relationship_id", "coaching_sessions"."collab_document_name" AS "A_collab_document_name", "coaching_sessions"."date" AS "A_date", "coaching_sessions"."duration_minutes" AS "A_duration_minutes", "coaching_sessions"."meeting_url" AS "A_meeting_url", CAST("coaching_sessions"."provider" AS "text") AS "A_provider", "coaching_sessions"."created_at" AS "A_created_at", "coaching_sessions"."updated_at" AS "A_updated_at", "coaching_sessions"."hydrated_at" AS "A_hydrated_at", "coaching_relationships"."id" AS "B_id", "coaching_relationships"."organization_id" AS "B_organization_id", "coaching_relationships"."coach_id" AS "B_coach_id", "coaching_relationships"."coachee_id" AS "B_coachee_id", "coaching_relationships"."slug" AS "B_slug", "coaching_relationships"."created_at" AS "B_created_at", "coaching_relationships"."updated_at" AS "B_updated_at" FROM "refactor_platform"."coaching_sessions" LEFT JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE "coaching_sessions"."id" = $1 LIMIT $2"#,
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
                r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."collab_document_name", "coaching_sessions"."date", "coaching_sessions"."duration_minutes", "coaching_sessions"."meeting_url", CAST("coaching_sessions"."provider" AS "text"), "coaching_sessions"."created_at", "coaching_sessions"."updated_at", "coaching_sessions"."hydrated_at" FROM "refactor_platform"."coaching_sessions" INNER JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE "coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2"#,
                [user_id.into(), user_id.into()]
            )]
        );

        Ok(())
    }

    // Locks down the SeaORM-emitted SQL for the monthly count aggregation.
    // Catches refactoring drift on bind positions, the half-open range
    // conversion (`to_date.succ_opt()`), and the conditional Option branch.
    //
    // Note: this asserts the emitted SQL string, not Postgres-acceptance of
    // it. MockDatabase captures bytes but does not execute. A regression
    // that re-introduces multiple `cust_with_values` clones (each generating
    // a fresh `$N` placeholder) would still produce a SQL string that
    // textually differs from the expected one here -- which trips this
    // assertion -- but the underlying class of bug (Postgres rejecting
    // text-mismatched GROUP BY expressions, SQLSTATE 42803) is only
    // catchable end-to-end against a real database.
    #[tokio::test]
    async fn find_counts_by_month_for_user_emits_expected_sql_without_relationship_filter(
    ) -> Result<(), Error> {
        let user_id = Id::new_v4();
        let from_date = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let to_date = chrono::NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
        let to_exclusive = chrono::NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let _ = find_counts_by_month_for_user(
            &db,
            user_id,
            from_date,
            to_date,
            "America/Los_Angeles",
            None,
        )
        .await?;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT to_char(date_trunc('month', "coaching_sessions"."date" AT TIME ZONE $1::text), 'YYYY-MM') AS "month", COUNT(*)::bigint AS "count" FROM "refactor_platform"."coaching_sessions" INNER JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE "coaching_sessions"."date" >= $2 AND "coaching_sessions"."date" < $3 AND ("coaching_relationships"."coach_id" = $4 OR "coaching_relationships"."coachee_id" = $5) GROUP BY "month" ORDER BY "month" ASC"#,
                [
                    "America/Los_Angeles".into(),
                    from_date.into(),
                    to_exclusive.into(),
                    user_id.into(),
                    user_id.into(),
                ]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_counts_by_month_for_user_emits_expected_sql_with_relationship_filter(
    ) -> Result<(), Error> {
        let user_id = Id::new_v4();
        let rel_id = Id::new_v4();
        let from_date = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let to_date = chrono::NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
        let to_exclusive = chrono::NaiveDate::from_ymd_opt(2026, 4, 1).unwrap();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let _ = find_counts_by_month_for_user(
            &db,
            user_id,
            from_date,
            to_date,
            "Europe/Berlin",
            Some(rel_id),
        )
        .await?;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT to_char(date_trunc('month', "coaching_sessions"."date" AT TIME ZONE $1::text), 'YYYY-MM') AS "month", COUNT(*)::bigint AS "count" FROM "refactor_platform"."coaching_sessions" INNER JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE "coaching_sessions"."date" >= $2 AND "coaching_sessions"."date" < $3 AND ("coaching_relationships"."coach_id" = $4 OR "coaching_relationships"."coachee_id" = $5) AND "coaching_sessions"."coaching_relationship_id" = $6 GROUP BY "month" ORDER BY "month" ASC"#,
                [
                    "Europe/Berlin".into(),
                    from_date.into(),
                    to_exclusive.into(),
                    user_id.into(),
                    user_id.into(),
                    rel_id.into(),
                ]
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
            duration_minutes: crate::duration::Duration::default_minutes(),
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: Some(now.into()),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![session.clone()]])
            .into_connection();

        let results =
            find_by_user_with_includes(&db, user_id, SessionQueryOptions::default()).await?;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session.id, session_id);
        assert!(results[0].relationship.is_none());
        assert!(results[0].organization.is_none());
        assert!(results[0].goals.is_none());
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
            duration_minutes: crate::duration::Duration::default_minutes(),
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: Some(now.into()),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![session.clone()]])
            .into_connection();

        let results = find_by_user_with_includes(
            &db,
            user_id,
            SessionQueryOptions {
                from_date: Some(from_date),
                to_date: Some(to_date),
                ..Default::default()
            },
        )
        .await?;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session.id, session.id);
        assert_eq!(results[0].session.date, from_date.into());

        Ok(())
    }

    // Locks down the SQL emitted when `tz` is supplied: both date bounds are
    // shifted via `AT TIME ZONE`, matching the expression form used by
    // `find_counts_by_month_for_user`. Catches drift in bind positions, the
    // `to_date.succ_opt()` half-open conversion, and the tz-bind ordering.
    #[tokio::test]
    async fn find_by_user_with_includes_with_tz_emits_shifted_boundaries() -> Result<(), Error> {
        let user_id = Id::new_v4();
        let from_date = chrono::NaiveDate::from_ymd_opt(2026, 5, 24).unwrap();
        let to_date = chrono::NaiveDate::from_ymd_opt(2026, 5, 24).unwrap();
        let to_exclusive = chrono::NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let _ = find_by_user_with_includes(
            &db,
            user_id,
            SessionQueryOptions {
                from_date: Some(from_date),
                to_date: Some(to_date),
                tz: Some("America/Los_Angeles".to_string()),
                ..Default::default()
            },
        )
        .await?;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."collab_document_name", "coaching_sessions"."date", "coaching_sessions"."duration_minutes", "coaching_sessions"."meeting_url", CAST("coaching_sessions"."provider" AS "text"), "coaching_sessions"."created_at", "coaching_sessions"."updated_at", "coaching_sessions"."hydrated_at" FROM "refactor_platform"."coaching_sessions" INNER JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE ("coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2) AND ("coaching_sessions"."date" >= ($3::timestamp AT TIME ZONE $4::text) AT TIME ZONE 'UTC') AND ("coaching_sessions"."date" < ($5::timestamp AT TIME ZONE $6::text) AT TIME ZONE 'UTC')"#,
                [
                    user_id.into(),
                    user_id.into(),
                    from_date.into(),
                    "America/Los_Angeles".into(),
                    to_exclusive.into(),
                    "America/Los_Angeles".into(),
                ]
            )]
        );

        Ok(())
    }

    // Regression guard for the legacy UTC-naive path. When `tz` is omitted,
    // the boundaries must be plain `Column::Date.gte/.lt`, not the
    // tz-shifted expression. Asserting the absence of `AT TIME ZONE` here
    // prevents an accidental "always shift" rewrite from silently breaking
    // existing callers who rely on UTC interpretation.
    #[tokio::test]
    async fn find_by_user_with_includes_without_tz_emits_utc_naive_boundaries() -> Result<(), Error>
    {
        let user_id = Id::new_v4();
        let from_date = chrono::NaiveDate::from_ymd_opt(2026, 5, 24).unwrap();
        let to_date = chrono::NaiveDate::from_ymd_opt(2026, 5, 24).unwrap();
        let to_exclusive = chrono::NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let _ = find_by_user_with_includes(
            &db,
            user_id,
            SessionQueryOptions {
                from_date: Some(from_date),
                to_date: Some(to_date),
                tz: None,
                ..Default::default()
            },
        )
        .await?;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."collab_document_name", "coaching_sessions"."date", "coaching_sessions"."duration_minutes", "coaching_sessions"."meeting_url", CAST("coaching_sessions"."provider" AS "text"), "coaching_sessions"."created_at", "coaching_sessions"."updated_at", "coaching_sessions"."hydrated_at" FROM "refactor_platform"."coaching_sessions" INNER JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE ("coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2) AND "coaching_sessions"."date" >= $3 AND "coaching_sessions"."date" < $4"#,
                [
                    user_id.into(),
                    user_id.into(),
                    from_date.into(),
                    to_exclusive.into(),
                ]
            )]
        );

        Ok(())
    }

    // Mixed-presence: only `from_date` supplied, `tz` present. The shift
    // applies to the present bound only; no upper-bound filter is emitted.
    #[tokio::test]
    async fn find_by_user_with_includes_with_tz_and_only_from_date_shifts_lower_bound(
    ) -> Result<(), Error> {
        let user_id = Id::new_v4();
        let from_date = chrono::NaiveDate::from_ymd_opt(2026, 5, 24).unwrap();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let _ = find_by_user_with_includes(
            &db,
            user_id,
            SessionQueryOptions {
                from_date: Some(from_date),
                to_date: None,
                tz: Some("Europe/Berlin".to_string()),
                ..Default::default()
            },
        )
        .await?;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."collab_document_name", "coaching_sessions"."date", "coaching_sessions"."duration_minutes", "coaching_sessions"."meeting_url", CAST("coaching_sessions"."provider" AS "text"), "coaching_sessions"."created_at", "coaching_sessions"."updated_at", "coaching_sessions"."hydrated_at" FROM "refactor_platform"."coaching_sessions" INNER JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE ("coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2) AND ("coaching_sessions"."date" >= ($3::timestamp AT TIME ZONE $4::text) AT TIME ZONE 'UTC')"#,
                [
                    user_id.into(),
                    user_id.into(),
                    from_date.into(),
                    "Europe/Berlin".into(),
                ]
            )]
        );

        Ok(())
    }

    // Mirror of the lower-bound-only test: only `to_date` supplied, `tz`
    // present. Guards against future regressions that would accidentally
    // couple lower-bound emission to upper-bound presence (e.g. an
    // `if let (Some(from), Some(to)) = ...` rewrite that silently drops the
    // upper-only case while leaving both-bounds and lower-only green).
    #[tokio::test]
    async fn find_by_user_with_includes_with_tz_and_only_to_date_shifts_upper_bound(
    ) -> Result<(), Error> {
        let user_id = Id::new_v4();
        let to_date = chrono::NaiveDate::from_ymd_opt(2026, 5, 24).unwrap();
        let to_exclusive = chrono::NaiveDate::from_ymd_opt(2026, 5, 25).unwrap();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let _ = find_by_user_with_includes(
            &db,
            user_id,
            SessionQueryOptions {
                from_date: None,
                to_date: Some(to_date),
                tz: Some("Europe/Berlin".to_string()),
                ..Default::default()
            },
        )
        .await?;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."collab_document_name", "coaching_sessions"."date", "coaching_sessions"."duration_minutes", "coaching_sessions"."meeting_url", CAST("coaching_sessions"."provider" AS "text"), "coaching_sessions"."created_at", "coaching_sessions"."updated_at", "coaching_sessions"."hydrated_at" FROM "refactor_platform"."coaching_sessions" INNER JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE ("coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2) AND ("coaching_sessions"."date" < ($3::timestamp AT TIME ZONE $4::text) AT TIME ZONE 'UTC')"#,
                [
                    user_id.into(),
                    user_id.into(),
                    to_exclusive.into(),
                    "Europe/Berlin".into(),
                ]
            )]
        );

        Ok(())
    }

    #[test]
    fn include_options_needs_relationships_truth_table() {
        // organization || relationship → true; everything else → false.
        let none = IncludeOptions::none();
        let goal_only = IncludeOptions {
            goal: true,
            ..IncludeOptions::none()
        };
        let agreements_only = IncludeOptions {
            agreements: true,
            ..IncludeOptions::none()
        };
        let relationship = IncludeOptions {
            relationship: true,
            ..IncludeOptions::none()
        };
        // `organization` requires `relationship`; the validator enforces it.
        // For the predicate we still exercise the `organization=true` branch.
        let organization = IncludeOptions {
            relationship: true,
            organization: true,
            ..IncludeOptions::none()
        };

        assert!(!none.needs_relationships());
        assert!(!goal_only.needs_relationships());
        assert!(!agreements_only.needs_relationships());
        assert!(relationship.needs_relationships());
        assert!(organization.needs_relationships());
    }

    #[test]
    fn assemble_returns_empty_goals_vec_when_goal_included_but_none_exist() {
        let now = chrono::Utc::now();
        let session = Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            date: chrono::Local::now().naive_utc(),
            collab_document_name: None,
            duration_minutes: crate::duration::Duration::default_minutes(),
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: Some(now.into()),
        };

        let related = RelatedData::default();
        let includes = IncludeOptions {
            goal: true,
            ..IncludeOptions::none()
        };

        let enriched = assemble_enriched_session(session, &related, includes);

        // Must be Some(empty vec), not None — otherwise the frontend
        // can't distinguish "no goals" from "data not loaded yet".
        assert_eq!(enriched.goals, Some(vec![]));
    }

    #[test]
    fn assemble_returns_none_goals_when_goal_not_included() {
        let now = chrono::Utc::now();
        let session = Model {
            id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            date: chrono::Local::now().naive_utc(),
            collab_document_name: None,
            duration_minutes: crate::duration::Duration::default_minutes(),
            meeting_url: None,
            provider: None,
            created_at: now.into(),
            updated_at: now.into(),
            hydrated_at: Some(now.into()),
        };

        let related = RelatedData::default();
        let includes = IncludeOptions::none();

        let enriched = assemble_enriched_session(session, &related, includes);

        assert!(enriched.goals.is_none());
    }

    #[test]
    fn validate_allows_organization_with_relationship() {
        let includes = IncludeOptions {
            relationship: true,
            organization: true,
            goal: false,
            agreements: false,
            topics: false,
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
            topics: false,
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
            topics: false,
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
            topics: true,
        };
        assert!(includes.validate().is_ok());
    }

    #[test]
    fn validate_allows_none() {
        let includes = IncludeOptions::none();
        assert!(includes.validate().is_ok());
    }

    #[tokio::test]
    async fn find_meeting_url_returns_most_recent_url_skipping_sessions_without_one(
    ) -> Result<(), Error> {
        let relationship_id = Id::new_v4();

        // Session 1 (oldest): has a Google Meet URL
        let _session_1 = Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            date: chrono::NaiveDate::from_ymd_opt(2025, 1, 1).unwrap().into(),
            collab_document_name: None,
            duration_minutes: crate::duration::Duration::default_minutes(),
            meeting_url: Some("https://meet.google.com/old-meet-url".to_string()),
            provider: Some(Provider::Google),
            created_at: chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap(),
            updated_at: chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap(),
            hydrated_at: Some(
                chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z").unwrap(),
            ),
        };

        // Session 2 (middle): also has a Google Meet URL — this is the one we want
        let session_2 = Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            date: chrono::NaiveDate::from_ymd_opt(2025, 2, 1).unwrap().into(),
            collab_document_name: None,
            duration_minutes: crate::duration::Duration::default_minutes(),
            meeting_url: Some("https://meet.google.com/latest-meet-url".to_string()),
            provider: Some(Provider::Google),
            created_at: chrono::DateTime::parse_from_rfc3339("2025-02-01T00:00:00Z").unwrap(),
            updated_at: chrono::DateTime::parse_from_rfc3339("2025-02-01T00:00:00Z").unwrap(),
            hydrated_at: Some(
                chrono::DateTime::parse_from_rfc3339("2025-02-01T00:00:00Z").unwrap(),
            ),
        };

        // Session 3 (newest): no meeting URL — coach didn't request one this time
        let _session_3 = Model {
            id: Id::new_v4(),
            coaching_relationship_id: relationship_id,
            date: chrono::NaiveDate::from_ymd_opt(2025, 3, 1).unwrap().into(),
            collab_document_name: None,
            duration_minutes: crate::duration::Duration::default_minutes(),
            meeting_url: None,
            provider: None,
            created_at: chrono::DateTime::parse_from_rfc3339("2025-03-01T00:00:00Z").unwrap(),
            updated_at: chrono::DateTime::parse_from_rfc3339("2025-03-01T00:00:00Z").unwrap(),
            hydrated_at: Some(
                chrono::DateTime::parse_from_rfc3339("2025-03-01T00:00:00Z").unwrap(),
            ),
        };

        // The MockDatabase returns session_2 because our query filters for
        // meeting_url IS NOT NULL and orders by created_at DESC, so the DB
        // would return session_2 as the first (most recent) match.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![session_2.clone()]])
            .into_connection();

        let result =
            find_meeting_url_by_relationship_and_provider(&db, relationship_id, Provider::Google)
                .await?;

        // Should get session_2's URL, not session_1's older one.
        assert_eq!(
            result,
            Some("https://meet.google.com/latest-meet-url".to_string())
        );

        // Sanity-check the generated SQL: filters by relationship + provider,
        // requires meeting_url non-null, ordered by created_at desc.
        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."collab_document_name", "coaching_sessions"."date", "coaching_sessions"."duration_minutes", "coaching_sessions"."meeting_url", CAST("coaching_sessions"."provider" AS "text"), "coaching_sessions"."created_at", "coaching_sessions"."updated_at", "coaching_sessions"."hydrated_at" FROM "refactor_platform"."coaching_sessions" WHERE "coaching_sessions"."coaching_relationship_id" = $1 AND "coaching_sessions"."provider" = (CAST($2 AS "provider")) AND "coaching_sessions"."meeting_url" IS NOT NULL ORDER BY "coaching_sessions"."created_at" DESC LIMIT $3"#,
                [
                    relationship_id.into(),
                    "google".into(),
                    sea_orm::Value::BigUnsigned(Some(1))
                ]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_meeting_url_returns_none_when_no_matching_session_exists() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result =
            find_meeting_url_by_relationship_and_provider(&db, Id::new_v4(), Provider::Google)
                .await?;

        assert_eq!(result, None);
        Ok(())
    }
}
