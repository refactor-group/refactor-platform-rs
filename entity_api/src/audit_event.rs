//! The audit log: an append-only record of who changed what in an organization.
//!
//! Each organization's events form a hash chain. An event's `hash` is the lowercase hex
//! SHA-256 of its canonical encoding, which includes the previous event's hash, so editing,
//! removing or reordering a stored event breaks every link after it. See
//! `docs/architecture/audit_log.md`.

use chrono::{DateTime, SecondsFormat, SubsecRound, TimeDelta, Utc};
use entity::audit_events::{Action, Column, Entity, Model};
use entity::Id;
use futures::TryStreamExt;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, DbBackend, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, Statement, StreamTrait,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::actor::Actor;
use super::error::Error;

/// One change to record. Carries ids only, never names or emails.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEvent {
    pub organization_id: Id,
    /// Who made the change; `None` when the system did.
    pub actor: Option<Actor>,
    pub action: Action,
    /// The changed record, whose type the action names.
    pub subject_id: Id,
    /// Small facts about the change, such as the kind before and after. Strings, integers,
    /// booleans and ids only: floats may not read back from `jsonb` exactly as written.
    pub details: Value,
}

/// What a walk of an organization's chain found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainCheck {
    /// Every event links to the one before it and matches its own hash.
    Intact { count: u64 },
    /// The first event whose link to the previous event, or whose own hash, does not match.
    BrokenAt { event_id: Id },
}

/// Appends `event` to its organization's chain inside `txn`, so the event exists exactly when
/// the change it records commits.
///
/// Holds a per-organization lock until `txn` ends, so concurrent appends to one organization
/// take turns and each links to the one committed before it. Each event's time is strictly
/// later than the previous one's, so time order is chain order even across skewed clocks.
///
/// # Errors
///
/// Returns `EntityApiErrorKind::SystemError` when the lock, the read or the insert fails.
pub async fn append(txn: &DatabaseTransaction, event: NewEvent) -> Result<Model, Error> {
    lock_organization(txn, event.organization_id).await?;
    let previous = latest(txn, event.organization_id).await?;

    let unhashed = Model {
        id: Id::new_v4(),
        organization_id: event.organization_id,
        actor_user_id: event.actor.map(|actor| actor.id()),
        action: event.action.as_str().to_owned(),
        subject_type: event.action.subject_type().to_owned(),
        subject_id: event.subject_id,
        details: event.details,
        occurred_at: next_occurred_at(previous.as_ref()).fixed_offset(),
        prev_hash: previous.map(|previous| previous.hash),
        hash: String::new(),
    };

    Ok(Model {
        hash: hash_of(&unhashed),
        ..unhashed
    }
    .into_active_model()
    .reset_all()
    .insert(txn)
    .await?)
}

/// Walks an organization's events in chain order and reports whether each links to the one
/// before it and still matches its own hash.
///
/// # Errors
///
/// Returns `EntityApiErrorKind::SystemError` when the read fails.
pub async fn verify_chain(
    db: &(impl ConnectionTrait + StreamTrait),
    organization_id: Id,
) -> Result<ChainCheck, Error> {
    let mut events = Entity::find()
        .filter(Column::OrganizationId.eq(organization_id))
        .order_by_asc(Column::OccurredAt)
        .order_by_asc(Column::Id)
        .stream(db)
        .await?;

    let mut previous_hash = None;
    let mut count = 0;
    while let Some(event) = events.try_next().await? {
        if event.prev_hash != previous_hash || event.hash != hash_of(&event) {
            return Ok(ChainCheck::BrokenAt { event_id: event.id });
        }
        previous_hash = Some(event.hash);
        count += 1;
    }
    Ok(ChainCheck::Intact { count })
}

/// The hash of `event`: the lowercase hex SHA-256 of its canonical encoding. Ignores the
/// event's stored `hash`.
fn hash_of(event: &Model) -> String {
    hex::encode(Sha256::digest(canonical_encoding(event)))
}

/// The bytes an event's hash covers: a compact JSON array of, in this order, `prev_hash`,
/// `id`, `organization_id`, `actor_user_id`, `action`, `subject_type`, `subject_id`,
/// `details` and `occurred_at`.
///
/// A missing `prev_hash` or `actor_user_id` is `null`; ids are lowercase hyphenated UUIDs;
/// `details` has every object's keys sorted, at every depth; `occurred_at` is RFC 3339 in UTC
/// with exactly six fractional digits and a `Z` suffix.
fn canonical_encoding(event: &Model) -> String {
    json!([
        event.prev_hash,
        event.id,
        event.organization_id,
        event.actor_user_id,
        event.action,
        event.subject_type,
        event.subject_id,
        with_sorted_keys(&event.details),
        event
            .occurred_at
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Micros, true),
    ])
    .to_string()
}

/// `value` with every object's keys in sorted order, whatever order they were inserted in.
fn with_sorted_keys(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries: Vec<_> = object.iter().collect();
            entries.sort_by_key(|(key, _)| *key);
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), with_sorted_keys(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(with_sorted_keys).collect()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
}

/// The time for an event after `previous`: now, at the microsecond precision Postgres stores,
/// or one microsecond after `previous` when the clock has not moved past it.
fn next_occurred_at(previous: Option<&Model>) -> DateTime<Utc> {
    let now = Utc::now().trunc_subsecs(6);
    previous
        .map(|previous| previous.occurred_at.with_timezone(&Utc) + TimeDelta::microseconds(1))
        .map_or(now, |after_previous| now.max(after_previous))
}

/// The organization's most recent event, if it has any.
async fn latest(db: &impl ConnectionTrait, organization_id: Id) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::OrganizationId.eq(organization_id))
        .order_by_desc(Column::OccurredAt)
        .order_by_desc(Column::Id)
        .one(db)
        .await?)
}

/// Takes the organization's audit lock, held until the transaction ends.
async fn lock_organization(db: &impl ConnectionTrait, organization_id: Id) -> Result<(), Error> {
    // SQLite, the test tier, has no advisory locks and runs on one connection.
    if db.get_database_backend() != DbBackend::Postgres {
        return Ok(());
    }
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT pg_advisory_xact_lock($1, $2)",
        [
            AUDIT_LOCK_NAMESPACE.into(),
            organization_lock_key(organization_id).into(),
        ],
    ))
    .await?;
    Ok(())
}

/// The first key of every audit lock, apart from the pair locks' namespace (1) in
/// `coaching_relationship`.
const AUDIT_LOCK_NAMESPACE: i32 = 2;

/// The second key of an organization's audit lock.
fn organization_lock_key(organization_id: Id) -> i32 {
    let (upper, lower) = organization_id.as_u64_pair();
    let folded = upper ^ lower;
    (folded ^ (folded >> 32)) as i32
}
