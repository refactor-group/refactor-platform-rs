use std::borrow::Cow;

use super::{
    advisory_lock::{advisory_xact_lock, fold},
    error::{EntityApiErrorKind, Error},
    organization,
};
use crate::user;
use crate::user_role::is_member;
use chrono::Utc;
use entity::{
    coachees, coaches,
    coaching_relationship_kind::{Kind, Role, Seat},
    coaching_relationships::{self, viewer_of, ActiveModel, Entity, Model, Viewer, Visibility},
    coaching_sessions, users, Id,
};
use log::*;
use sea_orm::{
    entity::prelude::*, sea_query::Alias, sea_query::Expr, sea_query::OnConflict,
    ActiveValue::Unchanged, Condition, DatabaseConnection, DbErr, ExprTrait, FromQueryResult,
    Iterable, JoinType, QueryResult, QuerySelect, QueryTrait, Set, TransactionSession,
    TransactionTrait,
};
use serde::ser::{SerializeStruct, Serializer};
use serde::Serialize;
use slugify::slugify;
use utoipa::openapi::{ArrayBuilder, ObjectBuilder, Ref, RefOr, Schema};
use utoipa::{PartialSchema, ToSchema};

/// A relationship to create: who holds each seat and what kind it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draft {
    pub coach_id: Id,
    pub coachee_id: Id,
    pub kind: Kind,
}

/// A relationship [`create`] settled on, and whether this call inserted it.
#[derive(Debug, Clone)]
pub struct Settled {
    pub relationship: CoachingRelationshipWithUserNames,
    /// False when the pair's existing relationship was returned instead.
    pub is_new: bool,
}

/// Creates the drafted relationship in an organization.
///
/// Returns the existing relationship when the pair already holds one of the same kind
/// (in either direction for colleagues, in the drafted direction for coaching), and
/// refuses with `PairAlreadyRelated` when the pair holds one of the other kind.
///
/// The check and the insert run in one transaction holding the pair's advisory lock, so
/// two concurrent creates for the same pair are decided one after the other. Called on
/// a caller's transaction, it runs in a savepoint and the lock lasts until that
/// transaction ends.
pub async fn create(
    db: &(impl ConnectionTrait + TransactionTrait),
    organization_id: Id,
    draft: Draft,
) -> Result<Settled, Error> {
    debug!("New Coaching Relationship to be inserted: {draft:?}");

    if draft.coach_id == draft.coachee_id {
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::ValidationError {
                message: "A user cannot be their own partner in a relationship.".into(),
                details: None,
            },
        });
    }

    let organization = organization::find_by_id(db, organization_id).await?;
    if organization.archived_at.is_some() {
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::OrganizationArchived,
        });
    }

    let coach = user::find_by_id(db, draft.coach_id).await?;
    let coachee = user::find_by_id(db, draft.coachee_id).await?;

    let coach_organization_ids =
        // membership is independent of archive state
        organization::find_by_user(db, coach.id, organization::StatusFilter::All)
            .await?
            .iter()
            .map(|org| org.id)
            .collect::<Vec<Id>>();
    let coachee_organization_ids =
        organization::find_by_user(db, coachee.id, organization::StatusFilter::All)
            .await?
            .iter()
            .map(|org| org.id)
            .collect::<Vec<Id>>();

    // Check that the coach and coachee belong to the correct organization
    if !coach_organization_ids.contains(&organization_id)
        || !coachee_organization_ids.contains(&organization_id)
    {
        error!("Coach and coachee do not belong to the correct organization, not creating requested new coaching relationship between coach: {:?} and coachee: {:?} for organization: {:?}.", draft.coach_id, draft.coachee_id, organization_id);
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::ValidationError {
                message: "Coach and coachee must belong to the specified organization.".into(),
                details: None,
            },
        });
    }

    let txn = db.begin().await?;
    lock_pair(&txn, organization_id, draft.coach_id, draft.coachee_id).await?;

    let (relationship, is_new) = match settle(
        &draft,
        find_between(&txn, organization_id, draft.coach_id, draft.coachee_id).await?,
    )? {
        Some(existing) => {
            debug!(
                "Coaching relationship {} already exists for coach {}, coachee {} in organization {organization_id}; returning it",
                existing.id, existing.coach_id, existing.coachee_id
            );
            (existing, false)
        }
        None => insert(&txn, organization_id, draft, &coach, &coachee).await?,
    };

    txn.commit().await?;
    let (seated_coach, seated_coachee) = in_stored_seats(&relationship, &coach, &coachee);
    Ok(Settled {
        relationship: with_user_names(&relationship, seated_coach, seated_coachee),
        is_new,
    })
}

/// The two users in the seats `relationship` stores them in, which for a returned colleague
/// relationship may be the reverse of the request's.
fn in_stored_seats<'a>(
    relationship: &Model,
    one: &'a users::Model,
    other: &'a users::Model,
) -> (&'a users::Model, &'a users::Model) {
    if relationship.coach_id == one.id {
        (one, other)
    } else {
        (other, one)
    }
}

/// Inserts the drafted relationship, or returns the one a concurrent request inserted first;
/// the flag says whether this call inserted it.
async fn insert(
    db: &impl ConnectionTrait,
    organization_id: Id,
    draft: Draft,
    coach: &users::Model,
    coachee: &users::Model,
) -> Result<(Model, bool), Error> {
    let now = Utc::now();
    let slug = slugify!(format!("{} {}", coach.first_name, coachee.first_name).as_str());

    let coaching_relationship_active_model: ActiveModel = ActiveModel {
        organization_id: Set(organization_id),
        coach_id: Set(draft.coach_id),
        coachee_id: Set(draft.coachee_id),
        kind: Set(draft.kind),
        slug: Set(slug),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    match insert_unless_exists(db, coaching_relationship_active_model).await? {
        Some(inserted) => Ok((inserted, true)),
        None => {
            let winner = settle(
                &draft,
                find_between(db, organization_id, draft.coach_id, draft.coachee_id).await?,
            )?
            .ok_or_else(not_found)?;
            debug!(
                "Coaching relationship for coach {}, coachee {} in organization {organization_id} was created by a concurrent request; returning that one ({})",
                winner.coach_id, winner.coachee_id, winner.id
            );
            Ok((winner, false))
        }
    }
}

/// Holds the pair's advisory lock until the transaction ends, so creates and switches for
/// the same pair in an organization are decided one after the other.
async fn lock_pair(
    db: &impl ConnectionTrait,
    organization_id: Id,
    a: Id,
    b: Id,
) -> Result<(), Error> {
    advisory_xact_lock(
        db,
        PAIR_LOCK_NAMESPACE,
        pair_lock_key(organization_id, a, b),
    )
    .await
}

/// The first key of every pair lock. Pair locks take the two-key form, which Postgres keeps
/// apart from the single-key session locks, and this namespace keeps them apart from any
/// other two-key lock.
const PAIR_LOCK_NAMESPACE: i32 = 1;

/// The second key of the advisory lock for a pair in an organization, the same whichever
/// person is first.
fn pair_lock_key(organization_id: Id, a: Id, b: Id) -> i32 {
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let hash = [organization_id, Ord::min(a, b), Ord::max(a, b)]
        .iter()
        .map(|id| {
            let (upper, lower) = id.as_u64_pair();
            upper ^ lower
        })
        .fold(0, |key: u64, part| key.wrapping_mul(FNV_PRIME) ^ part);
    fold(hash)
}

/// Decides a create from the pair's existing relationships in the organization: the one to
/// return, `None` to insert, or `PairAlreadyRelated` when the pair holds the other kind.
///
/// A kind whose two seats hold the same role (colleagues) makes the pair unordered, so a
/// relationship in either direction is returned; otherwise only the drafted direction is.
fn settle(draft: &Draft, existing: Vec<Model>) -> Result<Option<Model>, Error> {
    if let Some(other) = existing.iter().find(|related| related.kind != draft.kind) {
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::PairAlreadyRelated {
                relationship_id: other.id,
            },
        });
    }
    let unordered = !draft.kind.seats_differ();
    Ok(existing
        .into_iter()
        .find(|related| unordered || related.coach_id == draft.coach_id))
}

/// Refuses with `PairAlreadyRelated` when the pair's other relationships in the organization
/// leave no room for `relationship` to become `target`: the same rule a create of `target`
/// follows, with `relationship` itself left out.
pub async fn ensure_pair_allows(
    db: &impl ConnectionTrait,
    relationship: &Model,
    target: Draft,
) -> Result<(), Error> {
    let others = find_between(
        db,
        relationship.organization_id,
        relationship.coach_id,
        relationship.coachee_id,
    )
    .await?
    .into_iter()
    .filter(|other| other.id != relationship.id)
    .collect();
    settle(&target, others)?.map_or(Ok(()), |other| {
        Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::PairAlreadyRelated {
                relationship_id: other.id,
            },
        })
    })
}

/// `relationship` as it stands once the pair's advisory lock is held, so any concurrent
/// create or switch for the pair has finished. Run it on a transaction: the lock lasts until
/// that transaction ends.
///
/// # Errors
///
/// `RecordNotFound` when the relationship no longer exists in its organization.
pub async fn find_locked(db: &impl ConnectionTrait, relationship: &Model) -> Result<Model, Error> {
    lock_pair(
        db,
        relationship.organization_id,
        relationship.coach_id,
        relationship.coachee_id,
    )
    .await?;
    find_by_id_in_organization(db, relationship.organization_id, relationship.id).await
}

/// Changes `relationship` in place to `target`'s kind and seats, keeping its id and history.
///
/// Holds the pair's advisory lock and re-checks [`ensure_pair_allows`] under it, so a
/// concurrent create or switch for the pair is decided one after the other. Run it on a
/// transaction: the lock lasts until that transaction ends.
pub async fn change_kind(
    db: &impl ConnectionTrait,
    relationship: &Model,
    target: Draft,
) -> Result<Model, Error> {
    lock_pair(
        db,
        relationship.organization_id,
        relationship.coach_id,
        relationship.coachee_id,
    )
    .await?;
    ensure_pair_allows(db, relationship, target).await?;
    Ok(ActiveModel {
        id: Unchanged(relationship.id),
        coach_id: Set(target.coach_id),
        coachee_id: Set(target.coachee_id),
        kind: Set(target.kind),
        updated_at: Set(Utc::now().into()),
        ..Default::default()
    }
    .update(db)
    .await?)
}

/// Inserts `relationship`, returning `None` when its coach, coachee and organization already exist.
pub async fn insert_unless_exists(
    db: &impl ConnectionTrait,
    relationship: ActiveModel,
) -> Result<Option<Model>, DbErr> {
    // DO NOTHING rather than letting the unique index raise: a constraint violation
    // aborts the caller's transaction, which would poison the membership insert that
    // `attach_to_organization` wraps around this and leave the recovery read unable
    // to run at all.
    let conflict = OnConflict::columns([
        coaching_relationships::Column::CoachId,
        coaching_relationships::Column::CoacheeId,
        coaching_relationships::Column::OrganizationId,
    ])
    .do_nothing()
    .to_owned();

    match Entity::insert(relationship)
        .on_conflict(conflict)
        .exec_with_returning(db)
        .await
    {
        Ok(inserted) => Ok(Some(inserted)),
        // DO NOTHING wrote no row, so RETURNING yielded none and SeaORM reports the
        // miss as RecordNotFound. Only a conflict can produce that here, since this
        // statement inserts exactly one row.
        Err(DbErr::RecordNotFound(_)) => Ok(None),
        Err(err) => Err(err),
    }
}

/// Every relationship between two people in an organization, in either direction.
async fn find_between(
    db: &impl ConnectionTrait,
    organization_id: Id,
    a: Id,
    b: Id,
) -> Result<Vec<Model>, Error> {
    let directed = |coach_id: Id, coachee_id: Id| {
        Condition::all()
            .add(coaching_relationships::Column::CoachId.eq(coach_id))
            .add(coaching_relationships::Column::CoacheeId.eq(coachee_id))
    };
    Ok(Entity::find()
        .filter(coaching_relationships::Column::OrganizationId.eq(organization_id))
        .filter(Condition::any().add(directed(a, b)).add(directed(b, a)))
        .all(db)
        .await?)
}

/// The relationship's participants who are still members of its organization.
///
/// The notify set for anything that happens inside a coaching relationship. Someone
/// removed from the organization keeps the relationship but stops being notified.
pub async fn notify_member_ids(
    db: &impl ConnectionTrait,
    relationship: &Model,
) -> Result<Vec<Id>, Error> {
    crate::user_role::retain_organization_members(
        db,
        &[relationship.coach_id, relationship.coachee_id],
        relationship.organization_id,
    )
    .await
}

/// Whether each participant still holds a role in the relationship's organization.
///
/// The same test as [`notify_member_ids`], so `true` means a write naming that
/// participant is not refused for membership.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, FromQueryResult)]
pub struct Membership {
    /// Whether the coach still holds a role in the relationship's organization.
    pub coach_is_member: bool,
    /// Whether the coachee still holds a role in the relationship's organization.
    pub coachee_is_member: bool,
}

impl Membership {
    /// The participants of `relationship` still in its organization.
    pub fn member_ids(self, relationship: &Model) -> Vec<Id> {
        [
            (relationship.coach_id, self.coach_is_member),
            (relationship.coachee_id, self.coachee_is_member),
        ]
        .into_iter()
        .filter_map(|(id, is_member)| is_member.then_some(id))
        .collect()
    }
}

/// One of a relationship's two people as the wire describes them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = domain::coaching_relationship::Participant)]
pub struct Participant {
    pub id: Id,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
    /// What this person is to the relationship, from its kind and their seat.
    pub role: Role,
    /// Whether this person still holds a role in the relationship's organization.
    pub is_member: bool,
}

impl Participant {
    /// The person in `relationship`'s `seat`, their role taken from its kind.
    pub fn new(
        relationship: &Model,
        seat: Seat,
        first_name: String,
        last_name: String,
        email: String,
        is_member: bool,
    ) -> Self {
        Self {
            id: relationship.participant(seat),
            first_name,
            last_name,
            email,
            role: relationship.kind.rules().role(seat),
            is_member,
        }
    }
}

/// What a relationship lets its caller do, and whose leaving its organization blocks
/// scheduling its sessions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = domain::coaching_relationship::Capabilities)]
pub struct Capabilities {
    /// What the caller may do in the relationship; null when the caller isn't a participant.
    #[schema(required = true)]
    pub viewer: Option<Viewer>,
    /// Whether transcripts of its sessions are scanned for suggested actions.
    pub suggests_actions: bool,
    /// The participant whose leaving the organization blocks scheduling its sessions, the one
    /// a scheduling 409 names; null when nobody does.
    #[schema(required = true)]
    pub scheduling_blocked_by: Option<Id>,
}

impl Capabilities {
    /// `relationship` as `caller` sees it, given which of its participants are still members.
    pub fn for_caller(relationship: &Model, membership: Membership, caller: &users::Model) -> Self {
        Self {
            viewer: viewer_of(relationship, caller),
            suggests_actions: relationship.kind.rules().suggests_actions(),
            scheduling_blocked_by: relationship
                .seat_blocking_scheduling(&membership.member_ids(relationship))
                .map(|seat| relationship.participant(seat)),
        }
    }
}

/// A coaching relationship with its participants' [`Membership`].
#[derive(Debug, Clone, PartialEq, Serialize, FromQueryResult)]
pub struct WithMembership {
    #[serde(flatten)]
    #[sea_orm(nested)]
    pub relationship: Model,
    /// Off the wire: a session list carries it on its `participants`.
    #[serde(skip)]
    #[sea_orm(nested)]
    pub membership: Membership,
    /// Empty until [`WithMembership::for_caller`] fills it.
    #[serde(flatten)]
    #[sea_orm(skip)]
    pub capabilities: Capabilities,
}

/// By hand: the relationship's derived schema leaves out the fields it never deserializes,
/// which the wire still carries.
impl PartialSchema for WithMembership {
    fn schema() -> RefOr<Schema> {
        let fields: [(&str, RefOr<Schema>); 8] = [
            ("id", utoipa::schema!(Uuid).into()),
            ("organization_id", utoipa::schema!(Uuid).into()),
            ("coach_id", utoipa::schema!(Uuid).into()),
            ("coachee_id", utoipa::schema!(Uuid).into()),
            (
                "kind",
                Ref::from_schema_name(<Kind as ToSchema>::name()).into(),
            ),
            ("slug", utoipa::schema!(String).into()),
            ("created_at", utoipa::schema!(DateTime<FixedOffset>).into()),
            ("updated_at", utoipa::schema!(DateTime<FixedOffset>).into()),
        ];
        object_with_capabilities(fields)
    }
}

impl ToSchema for WithMembership {
    fn name() -> Cow<'static, str> {
        Cow::Borrowed("domain.coaching_relationship.WithMembership")
    }

    fn schemas(schemas: &mut Vec<(String, RefOr<Schema>)>) {
        schemas.push((<Kind as ToSchema>::name().into_owned(), Kind::schema()));
        schemas.push((<Viewer as ToSchema>::name().into_owned(), Viewer::schema()));
    }
}

impl WithMembership {
    /// This relationship as `caller` sees it.
    pub fn for_caller(self, caller: &users::Model) -> Self {
        Self {
            capabilities: Capabilities::for_caller(&self.relationship, self.membership, caller),
            ..self
        }
    }
}

/// Adds each participant's [`Membership`] to a relationship select, computed in its SQL.
pub(crate) fn select_membership<Q: QuerySelect>(query: Q) -> Q {
    let organization = || Expr::col(coaching_relationships::Column::OrganizationId.as_column_ref());
    query
        .column_as(
            is_member(
                Expr::col(coaching_relationships::Column::CoachId.as_column_ref()),
                organization(),
            ),
            "coach_is_member",
        )
        .column_as(
            is_member(
                Expr::col(coaching_relationships::Column::CoacheeId.as_column_ref()),
                organization(),
            ),
            "coachee_is_member",
        )
}

/// Joins a relationship select to its coach and coachee and selects their names and emails.
fn select_with_user_names(
    query: Select<coaching_relationships::Entity>,
) -> Select<coaching_relationships::Entity> {
    query
        .join_as(
            JoinType::Join,
            coaches::Relation::CoachingRelationships.def().rev(),
            Alias::new("coaches"),
        )
        .join_as(
            JoinType::Join,
            coachees::Relation::CoachingRelationships.def().rev(),
            Alias::new("coachees"),
        )
        .select_only()
        .column(coaching_relationships::Column::Id)
        .column(coaching_relationships::Column::OrganizationId)
        .column(coaching_relationships::Column::CoachId)
        .column(coaching_relationships::Column::CoacheeId)
        .column(coaching_relationships::Column::Kind)
        .column(coaching_relationships::Column::Slug)
        .column(coaching_relationships::Column::CreatedAt)
        .column(coaching_relationships::Column::UpdatedAt)
        .column_as(Expr::cust("coaches.first_name"), "coach_first_name")
        .column_as(Expr::cust("coaches.last_name"), "coach_last_name")
        .column_as(Expr::cust("coachees.first_name"), "coachee_first_name")
        .column_as(Expr::cust("coachees.last_name"), "coachee_last_name")
        .column_as(Expr::cust("coaches.email"), "coach_email")
        .column_as(Expr::cust("coachees.email"), "coachee_email")
}

/// Pairs a coaching relationship with the names and emails of its coach and coachee.
///
/// Only `create` builds one, after refusing unless both hold a role in the organization.
fn with_user_names(
    coaching_relationship: &Model,
    coach: &users::Model,
    coachee: &users::Model,
) -> CoachingRelationshipWithUserNames {
    CoachingRelationshipWithUserNames {
        id: coaching_relationship.id,
        organization_id: coaching_relationship.organization_id,
        coach_id: coaching_relationship.coach_id,
        coachee_id: coaching_relationship.coachee_id,
        kind: coaching_relationship.kind,
        slug: coaching_relationship.slug.clone(),
        coach_first_name: coach.first_name.clone(),
        coach_last_name: coach.last_name.clone(),
        coachee_first_name: coachee.first_name.clone(),
        coachee_last_name: coachee.last_name.clone(),
        coach_email: coach.email.clone(),
        coachee_email: coachee.email.clone(),
        created_at: coaching_relationship.created_at,
        updated_at: coaching_relationship.updated_at,
        membership: Membership {
            coach_is_member: true,
            coachee_is_member: true,
        },
        capabilities: Capabilities::default(),
    }
}

pub async fn find_by_id(db: &impl ConnectionTrait, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// The relationship `id` in `organization_id`.
///
/// # Errors
///
/// `RecordNotFound` when there is none, or it belongs to another organization.
pub async fn find_by_id_in_organization(
    db: &impl ConnectionTrait,
    organization_id: Id,
    id: Id,
) -> Result<Model, Error> {
    Entity::find_by_id(id)
        .filter(coaching_relationships::Column::OrganizationId.eq(organization_id))
        .one(db)
        .await?
        .ok_or_else(not_found)
}

/// The relationship's coach and coachee, in that order, read in one query.
///
/// # Errors
///
/// `RecordNotFound` when the relationship does not exist.
pub async fn find_coach_and_coachee(
    db: &impl ConnectionTrait,
    relationship_id: Id,
) -> Result<(users::Model, users::Model), Error> {
    let row = joined_people_row(db, Entity::find_by_id(relationship_id).select_only()).await?;
    people(&row)
}

/// The relationship with its coach and coachee, in that order, read in one query.
///
/// # Errors
///
/// `RecordNotFound` when the relationship does not exist.
pub async fn find_with_coach_and_coachee(
    db: &impl ConnectionTrait,
    relationship_id: Id,
) -> Result<(Model, users::Model, users::Model), Error> {
    let row = joined_people_row(db, Entity::find_by_id(relationship_id)).await?;
    let (coach, coachee) = people(&row)?;
    Ok((Model::from_query_result(&row, "")?, coach, coachee))
}

/// The one row of `select` joined to its coach and coachee, their columns prefixed.
async fn joined_people_row(
    db: &impl ConnectionTrait,
    mut select: Select<Entity>,
) -> Result<QueryResult, Error> {
    for (alias, prefix, relationship_column) in [
        (
            COACH_ALIAS,
            COACH_PREFIX,
            coaching_relationships::Column::CoachId,
        ),
        (
            COACHEE_ALIAS,
            COACHEE_PREFIX,
            coaching_relationships::Column::CoacheeId,
        ),
    ] {
        select = users::Column::iter().fold(select, |select, column| {
            select.column_as(
                Expr::col((Alias::new(alias), column)),
                format!("{prefix}{}", column.to_string()),
            )
        });
        QuerySelect::query(&mut select).join_as(
            JoinType::InnerJoin,
            users::Entity.table_ref(),
            Alias::new(alias),
            Expr::col((Alias::new(alias), users::Column::Id))
                .equals((coaching_relationships::Entity, relationship_column)),
        );
    }

    db.query_one(QueryTrait::query(&mut select))
        .await?
        .ok_or_else(not_found)
}

/// The coach and coachee read from a row of [`joined_people_row`].
fn people(row: &QueryResult) -> Result<(users::Model, users::Model), Error> {
    Ok((
        users::Model::from_query_result(row, COACH_PREFIX)?,
        users::Model::from_query_result(row, COACHEE_PREFIX)?,
    ))
}

const COACH_ALIAS: &str = "coach";
const COACH_PREFIX: &str = "coach_";
const COACHEE_ALIAS: &str = "coachee";
const COACHEE_PREFIX: &str = "coachee_";

fn not_found() -> Error {
    Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    }
}

pub async fn find_by_user(db: &DatabaseConnection, user_id: Id) -> Result<Vec<Model>, Error> {
    let coaching_relationships: Vec<coaching_relationships::Model> =
        coaching_relationships::Entity::find()
            .filter(
                Condition::any()
                    .add(coaching_relationships::Column::CoachId.eq(user_id))
                    .add(coaching_relationships::Column::CoacheeId.eq(user_id)),
            )
            .all(db)
            .await?;

    Ok(coaching_relationships)
}

/// Finds coaching relationships where the given user is the coach, within a specific organization.
pub async fn find_by_coach_and_organization(
    db: &DatabaseConnection,
    coach_id: Id,
    organization_id: Id,
) -> Result<Vec<Model>, Error> {
    let relationships = coaching_relationships::Entity::find()
        .filter(
            Condition::all()
                .add(coaching_relationships::Column::CoachId.eq(coach_id))
                .add(coaching_relationships::Column::OrganizationId.eq(organization_id)),
        )
        .all(db)
        .await?;

    Ok(relationships)
}

/// Finds coaching relationships where the given user is a participant
/// (coach OR coachee), within a specific organization.
pub async fn find_by_user_and_organization(
    db: &DatabaseConnection,
    user_id: Id,
    organization_id: Id,
) -> Result<Vec<Model>, Error> {
    let relationships = coaching_relationships::Entity::find()
        .filter(
            Condition::all()
                .add(
                    Condition::any()
                        .add(coaching_relationships::Column::CoachId.eq(user_id))
                        .add(coaching_relationships::Column::CoacheeId.eq(user_id)),
                )
                .add(coaching_relationships::Column::OrganizationId.eq(organization_id)),
        )
        .all(db)
        .await?;

    Ok(relationships)
}

/// The query form of [`Model::is_participant_and_member`]: relationships `viewer` is coach or
/// coachee of, in an organization they are still a member of unless a SuperAdmin.
pub fn accessible_to(viewer: &Visibility) -> Condition {
    Condition::all()
        .add(
            Condition::any()
                .add(coaching_relationships::Column::CoachId.eq(viewer.user_id))
                .add(coaching_relationships::Column::CoacheeId.eq(viewer.user_id)),
        )
        .add_option(in_member_organization(viewer))
}

/// Relationships where `coach` coaches `coachee_id` and is still a member of the
/// relationship's organization, unless `coach` is a SuperAdmin.
pub fn coached_by(coach: &Visibility, coachee_id: Id) -> Condition {
    Condition::all()
        .add(coaching_relationships::Column::CoachId.eq(coach.user_id))
        .add(coaching_relationships::Column::CoacheeId.eq(coachee_id))
        .add_option(in_member_organization(coach))
}

/// The org half of [`accessible_to`]; `None` for a SuperAdmin, who skips it.
fn in_member_organization(viewer: &Visibility) -> Option<Expr> {
    (!viewer.is_super_admin).then(|| {
        coaching_relationships::Column::OrganizationId.is_in(viewer.member_org_ids.iter().copied())
    })
}

/// Checks if a user is a coach of another user.
///
/// Returns `true` if `coach` coaches `potential_coachee_id` in a relationship they may
/// still access (see [`coached_by`]).
pub async fn is_coach_of(
    db: &DatabaseConnection,
    coach: &Visibility,
    potential_coachee_id: Id,
) -> Result<bool, Error> {
    let relationship = coaching_relationships::Entity::find()
        .filter(coached_by(coach, potential_coachee_id))
        .one(db)
        .await?;

    Ok(relationship.is_some())
}

pub async fn find_by_organization(
    db: &impl ConnectionTrait,
    organization_id: Id,
) -> Result<Vec<Model>, Error> {
    let query = by_organization(coaching_relationships::Entity::find(), organization_id).await;

    Ok(query.all(db).await?)
}

pub async fn find_by_organization_with_user_names(
    db: &impl ConnectionTrait,
    organization_id: Id,
) -> Result<Vec<CoachingRelationshipWithUserNames>, Error> {
    let query = by_organization(coaching_relationships::Entity::find(), organization_id).await;
    let query = select_membership(select_with_user_names(query))
        .into_model::<CoachingRelationshipWithUserNames>();

    Ok(query.all(db).await?)
}

pub async fn find_by_user_and_organization_with_user_names(
    db: &impl ConnectionTrait,
    user_id: Id,
    organization_id: Id,
) -> Result<Vec<CoachingRelationshipWithUserNames>, Error> {
    let query = by_organization(coaching_relationships::Entity::find(), organization_id)
        .await
        .filter(
            Condition::any()
                .add(coaching_relationships::Column::CoachId.eq(user_id))
                .add(coaching_relationships::Column::CoacheeId.eq(user_id)),
        );
    let query = select_membership(select_with_user_names(query))
        .into_model::<CoachingRelationshipWithUserNames>();

    Ok(query.all(db).await?)
}

pub async fn get_relationship_with_user_names(
    db: &DatabaseConnection,
    relationship_id: Id,
) -> Result<Option<CoachingRelationshipWithUserNames>, Error> {
    let query =
        by_coaching_relationship(coaching_relationships::Entity::find(), relationship_id).await;
    let query = select_membership(select_with_user_names(query))
        .into_model::<CoachingRelationshipWithUserNames>();

    Ok(query.one(db).await?)
}

pub async fn by_coaching_relationship(
    query: Select<coaching_relationships::Entity>,
    id: Id,
) -> Select<coaching_relationships::Entity> {
    let relationship_subsquery = Entity::find_by_id(id)
        .select_only()
        .column(entity::coaching_relationships::Column::Id)
        .filter(entity::coaching_relationships::Column::Id.eq(id))
        .into_query();

    query.filter(coaching_relationships::Column::Id.in_subquery(relationship_subsquery))
}

async fn by_organization(
    query: Select<coaching_relationships::Entity>,
    organization_id: Id,
) -> Select<coaching_relationships::Entity> {
    let organization_subquery = entity::organizations::Entity::find()
        .select_only()
        .column(entity::organizations::Column::Id)
        .filter(entity::organizations::Column::Id.eq(organization_id))
        .into_query();

    query.filter(coaching_relationships::Column::OrganizationId.in_subquery(organization_subquery))
}

/// Deletes every relationship `user_id` coaches or is coached in, and returns them.
///
/// # Errors
///
/// A relationship that still has a coaching session fails the foreign key; see
/// [`Error::is_foreign_key_violation`].
pub async fn delete_by_user_id(
    db: &impl ConnectionTrait,
    user_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::delete_many()
        .filter(involving(user_id))
        .exec_with_returning(db)
        .await?)
}

/// Counts the relationships `user_id` coaches or is coached in that have at least one session.
pub async fn count_with_sessions_by_user_id(
    db: &impl ConnectionTrait,
    user_id: Id,
) -> Result<u64, Error> {
    let has_session = coaching_sessions::Entity::find()
        .select_only()
        .expr(Expr::cust("1"))
        .filter(
            Expr::col(coaching_sessions::Column::CoachingRelationshipId.as_column_ref())
                .equals(coaching_relationships::Column::Id.as_column_ref()),
        )
        .into_query();

    Ok(Entity::find()
        .filter(involving(user_id))
        .filter(Expr::exists(has_session))
        .count(db)
        .await?)
}

/// Relationships in which `user_id` is the coach or the coachee.
fn involving(user_id: Id) -> Condition {
    Condition::any()
        .add(coaching_relationships::Column::CoachId.eq(user_id))
        .add(coaching_relationships::Column::CoacheeId.eq(user_id))
}

/// The column a participant in `seat` occupies.
fn seat_column(seat: Seat) -> coaching_relationships::Column {
    match seat {
        Seat::CoachId => coaching_relationships::Column::CoachId,
        Seat::CoacheeId => coaching_relationships::Column::CoacheeId,
    }
}

/// The relationships in which `user_id` holds `role`: for each kind, the seats whose rules give
/// that role.
fn holding_role(user_id: Id, role: Role) -> Condition {
    Kind::iter()
        .flat_map(|kind| [Seat::CoachId, Seat::CoacheeId].map(|seat| (kind, seat)))
        .filter(|&(kind, seat)| kind.rules().role(seat) == role)
        .fold(Condition::any(), |held, (kind, seat)| {
            held.add(
                Condition::all()
                    .add(coaching_relationships::Column::Kind.eq(kind))
                    .add(seat_column(seat).eq(user_id)),
            )
        })
}

/// Finds coaching relationships for a user with optional role filtering.
///
/// Returns relationships with user names for display purposes: those `viewer` may access
/// (see [`accessible_to`]), narrowed to those in which they hold `role` when one is given.
pub async fn find_by_user_id_with_user_names(
    db: &DatabaseConnection,
    viewer: &Visibility,
    role: Option<Role>,
) -> Result<Vec<CoachingRelationshipWithUserNames>, Error> {
    let query = coaching_relationships::Entity::find()
        .filter(accessible_to(viewer))
        .apply_if(role, |q, role| q.filter(holding_role(viewer.user_id, role)));
    let query = select_membership(select_with_user_names(query))
        .into_model::<CoachingRelationshipWithUserNames>();

    Ok(query.all(db).await?)
}

// A convenient combined struct that holds the results of looking up the Users associated
// with the coach/coachee ids.
#[derive(FromQueryResult, Debug, PartialEq, Clone)]
pub struct CoachingRelationshipWithUserNames {
    pub id: Id,
    pub organization_id: Id,
    pub coach_id: Id,
    pub coachee_id: Id,
    pub kind: Kind,
    pub slug: String,
    pub coach_first_name: String,
    pub coach_last_name: String,
    pub coachee_first_name: String,
    pub coachee_last_name: String,
    pub coach_email: String,
    pub coachee_email: String,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
    #[sea_orm(nested)]
    pub membership: Membership,
    /// Empty until [`CoachingRelationshipWithUserNames::for_caller`] fills it.
    #[sea_orm(skip)]
    pub capabilities: Capabilities,
}

impl CoachingRelationshipWithUserNames {
    /// The relationship row this describes.
    pub fn relationship(&self) -> Model {
        Model {
            id: self.id,
            organization_id: self.organization_id,
            coach_id: self.coach_id,
            coachee_id: self.coachee_id,
            kind: self.kind,
            slug: self.slug.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    /// This relationship as `caller` sees it.
    pub fn for_caller(self, caller: &users::Model) -> Self {
        Self {
            capabilities: Capabilities::for_caller(&self.relationship(), self.membership, caller),
            ..self
        }
    }

    /// Its two people, the coach seat first and the coachee seat second.
    pub fn participants(&self) -> [Participant; 2] {
        let relationship = self.relationship();
        [
            Participant::new(
                &relationship,
                Seat::CoachId,
                self.coach_first_name.clone(),
                self.coach_last_name.clone(),
                self.coach_email.clone(),
                self.membership.coach_is_member,
            ),
            Participant::new(
                &relationship,
                Seat::CoacheeId,
                self.coachee_first_name.clone(),
                self.coachee_last_name.clone(),
                self.coachee_email.clone(),
                self.membership.coachee_is_member,
            ),
        ]
    }
}

/// By hand: the wire flattens `capabilities` and lists `participants` in place of the
/// name, email and membership fields.
impl PartialSchema for CoachingRelationshipWithUserNames {
    fn schema() -> RefOr<Schema> {
        let participants = ArrayBuilder::new()
            .items(Ref::from_schema_name(<Participant as ToSchema>::name()))
            .min_items(Some(2))
            .max_items(Some(2))
            .description(Some("The coach seat, then the coachee seat."));
        let fields: [(&str, RefOr<Schema>); 7] = [
            ("id", utoipa::schema!(Uuid).into()),
            ("coach_id", utoipa::schema!(Uuid).into()),
            ("coachee_id", utoipa::schema!(Uuid).into()),
            (
                "kind",
                Ref::from_schema_name(<Kind as ToSchema>::name()).into(),
            ),
            ("participants", participants.into()),
            ("created_at", utoipa::schema!(DateTime<FixedOffset>).into()),
            ("updated_at", utoipa::schema!(DateTime<FixedOffset>).into()),
        ];
        object_with_capabilities(fields)
    }
}

/// An object requiring each of `fields` and [`Capabilities`]' fields, which the wire flattens
/// beside them.
fn object_with_capabilities<'a>(
    fields: impl IntoIterator<Item = (&'a str, RefOr<Schema>)>,
) -> RefOr<Schema> {
    let object = fields
        .into_iter()
        .fold(ObjectBuilder::new(), |object, (name, schema)| {
            object.property(name, schema).required(name)
        });
    [Capabilities::schema()]
        .into_iter()
        .filter_map(|schema| match schema {
            RefOr::T(Schema::Object(object)) => Some(object),
            _ => None,
        })
        .flat_map(|flattened| flattened.properties.into_iter())
        .fold(object, |object, (name, schema)| {
            object.property(&name, schema).required(&name)
        })
        .into()
}

impl ToSchema for CoachingRelationshipWithUserNames {
    fn name() -> Cow<'static, str> {
        Cow::Borrowed("domain.coaching_relationship.CoachingRelationshipWithUserNames")
    }

    fn schemas(schemas: &mut Vec<(String, RefOr<Schema>)>) {
        schemas.push((<Kind as ToSchema>::name().into_owned(), Kind::schema()));
        schemas.push((<Viewer as ToSchema>::name().into_owned(), Viewer::schema()));
        schemas.push((
            <Participant as ToSchema>::name().into_owned(),
            Participant::schema(),
        ));
        schemas.push((<Role as ToSchema>::name().into_owned(), Role::schema()));
    }
}

// serialize the CoachingRelationshipUserWithNames struct so that it can be used in the API
// and appears to be a coaching_relationship JSON object.
impl Serialize for CoachingRelationshipWithUserNames {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("CoachingRelationship", 10)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("coach_id", &self.coach_id)?;
        state.serialize_field("coachee_id", &self.coachee_id)?;
        state.serialize_field("kind", &self.kind)?;
        state.serialize_field("participants", &self.participants())?;
        state.serialize_field("created_at", &self.created_at)?;
        state.serialize_field("updated_at", &self.updated_at)?;
        state.serialize_field("viewer", &self.capabilities.viewer)?;
        state.serialize_field("suggests_actions", &self.capabilities.suggests_actions)?;
        state.serialize_field(
            "scheduling_blocked_by",
            &self.capabilities.scheduling_blocked_by,
        )?;
        state.end()
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn schema_lists_exactly_the_serialized_keys() {
        let now = Utc::now().fixed_offset();
        let relationship = CoachingRelationshipWithUserNames {
            id: Id::new_v4(),
            organization_id: Id::new_v4(),
            coach_id: Id::new_v4(),
            coachee_id: Id::new_v4(),
            kind: Kind::Coaching,
            slug: String::new(),
            coach_first_name: String::new(),
            coach_last_name: String::new(),
            coachee_first_name: String::new(),
            coachee_last_name: String::new(),
            coach_email: String::new(),
            coachee_email: String::new(),
            created_at: now,
            updated_at: now,
            membership: Membership::default(),
            capabilities: Capabilities::default(),
        };
        let serde_json::Value::Object(wire) =
            serde_json::to_value(&relationship).expect("serializes")
        else {
            panic!("expected an object");
        };
        let RefOr::T(Schema::Object(schema)) = CoachingRelationshipWithUserNames::schema() else {
            panic!("expected an object schema");
        };
        let keys: BTreeSet<&str> = wire.keys().map(String::as_str).collect();
        let properties: BTreeSet<&str> = schema.properties.keys().map(String::as_str).collect();
        let required: BTreeSet<&str> = schema.required.iter().map(String::as_str).collect();
        assert_eq!(keys, properties);
        assert_eq!(keys, required);
        assert_eq!(wire["kind"], "coaching");
        assert!(required.contains("kind"));

        let participants = wire["participants"].as_array().expect("a participant list");
        assert_eq!(participants.len(), 2);
        let RefOr::T(Schema::Object(participant)) = Participant::schema() else {
            panic!("expected an object schema");
        };
        let participant_properties: BTreeSet<&str> =
            participant.properties.keys().map(String::as_str).collect();
        let participant_required: BTreeSet<&str> =
            participant.required.iter().map(String::as_str).collect();
        for item in participants {
            let item_keys: BTreeSet<&str> = item
                .as_object()
                .expect("a participant object")
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(item_keys, participant_properties);
            assert_eq!(item_keys, participant_required);
        }
    }

    #[test]
    fn a_relationship_with_membership_serializes_exactly_its_schemas_keys() {
        let now = Utc::now().fixed_offset();
        let with_membership = WithMembership {
            relationship: Model {
                id: Id::new_v4(),
                organization_id: Id::new_v4(),
                coach_id: Id::new_v4(),
                coachee_id: Id::new_v4(),
                kind: Kind::Coaching,
                slug: String::new(),
                created_at: now,
                updated_at: now,
            },
            membership: Membership::default(),
            capabilities: Capabilities::default(),
        };
        let serde_json::Value::Object(wire) =
            serde_json::to_value(&with_membership).expect("serializes")
        else {
            panic!("expected an object");
        };
        let RefOr::T(Schema::Object(schema)) = WithMembership::schema() else {
            panic!("expected an object schema");
        };
        let keys: BTreeSet<&str> = wire.keys().map(String::as_str).collect();
        let properties: BTreeSet<&str> = schema.properties.keys().map(String::as_str).collect();
        let required: BTreeSet<&str> = schema.required.iter().map(String::as_str).collect();
        assert_eq!(keys, properties);
        assert_eq!(keys, required);
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

    #[tokio::test]
    async fn find_by_id_returns_record_when_present() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let coaching_relationship_id = Id::new_v4();
        let _ = find_by_id(&db, coaching_relationship_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_relationships"."id", "coaching_relationships"."organization_id", "coaching_relationships"."coach_id", "coaching_relationships"."coachee_id", CAST("coaching_relationships"."kind" AS "text"), "coaching_relationships"."slug", "coaching_relationships"."created_at", "coaching_relationships"."updated_at" FROM "refactor_platform"."coaching_relationships" WHERE "coaching_relationships"."id" = $1 LIMIT $2"#,
                [
                    coaching_relationship_id.into(),
                    sea_orm::Value::BigUnsigned(Some(1))
                ]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_coach_and_coachee_reads_both_users_in_one_joined_statement() {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let _ = find_coach_and_coachee(&db, Id::new_v4()).await;

        let statements: Vec<String> = db
            .into_transaction_log()
            .iter()
            .flat_map(|transaction| transaction.statements().iter().map(|s| s.sql.clone()))
            .collect();
        assert_eq!(statements.len(), 1);
        assert!(statements[0].contains("JOIN"), "{}", statements[0]);
        assert!(!statements[0].contains(" OR "), "{}", statements[0]);
    }

    #[tokio::test]
    async fn find_by_user_returns_all_records_associated_with_user() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let user_id = Id::new_v4();
        let _ = find_by_user(&db, user_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_relationships"."id", "coaching_relationships"."organization_id", "coaching_relationships"."coach_id", "coaching_relationships"."coachee_id", CAST("coaching_relationships"."kind" AS "text"), "coaching_relationships"."slug", "coaching_relationships"."created_at", "coaching_relationships"."updated_at" FROM "refactor_platform"."coaching_relationships" WHERE "coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2"#,
                [user_id.into(), user_id.into()]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_organization_queries_for_all_records_by_organization() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let organization_id = Id::new_v4();
        let _ = find_by_organization(&db, organization_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_relationships"."id", "coaching_relationships"."organization_id", "coaching_relationships"."coach_id", "coaching_relationships"."coachee_id", CAST("coaching_relationships"."kind" AS "text"), "coaching_relationships"."slug", "coaching_relationships"."created_at", "coaching_relationships"."updated_at" FROM "refactor_platform"."coaching_relationships" WHERE "coaching_relationships"."organization_id" IN (SELECT "organizations"."id" FROM "refactor_platform"."organizations" WHERE "organizations"."id" = $1)"#,
                [organization_id.into()]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_organization_with_user_names_returns_all_records_by_organization_with_user_names(
    ) -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let organization_id = Id::new_v4();
        let _ = find_by_organization_with_user_names(&db, organization_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_relationships"."id", "coaching_relationships"."organization_id", "coaching_relationships"."coach_id", "coaching_relationships"."coachee_id", CAST("coaching_relationships"."kind" AS "text"), "coaching_relationships"."slug", "coaching_relationships"."created_at", "coaching_relationships"."updated_at", coaches.first_name AS "coach_first_name", coaches.last_name AS "coach_last_name", coachees.first_name AS "coachee_first_name", coachees.last_name AS "coachee_last_name", coaches.email AS "coach_email", coachees.email AS "coachee_email", EXISTS(SELECT 1 FROM "refactor_platform"."user_roles" WHERE "user_roles"."user_id" = "coaching_relationships"."coach_id" AND ("user_roles"."organization_id" = "coaching_relationships"."organization_id" OR ("user_roles"."role" = (CAST($1 AS "role")) AND "user_roles"."organization_id" IS NULL))) AS "coach_is_member", EXISTS(SELECT 1 FROM "refactor_platform"."user_roles" WHERE "user_roles"."user_id" = "coaching_relationships"."coachee_id" AND ("user_roles"."organization_id" = "coaching_relationships"."organization_id" OR ("user_roles"."role" = (CAST($2 AS "role")) AND "user_roles"."organization_id" IS NULL))) AS "coachee_is_member" FROM "refactor_platform"."coaching_relationships" JOIN "refactor_platform"."users" AS "coaches" ON "coaching_relationships"."coach_id" = "coaches"."id" JOIN "refactor_platform"."users" AS "coachees" ON "coaching_relationships"."coachee_id" = "coachees"."id" WHERE "coaching_relationships"."organization_id" IN (SELECT "organizations"."id" FROM "refactor_platform"."organizations" WHERE "organizations"."id" = $3)"#,
                [
                    "super_admin".into(),
                    "super_admin".into(),
                    organization_id.into()
                ]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn delete_by_user_id_deletes_and_returns_every_relationship_of_the_user(
    ) -> Result<(), Error> {
        let user_id = Id::new_v4();
        let now = Utc::now();
        let relationship = Model {
            id: Id::new_v4(),
            organization_id: Id::new_v4(),
            coach_id: user_id,
            coachee_id: Id::new_v4(),
            kind: Kind::Coaching,
            slug: "coach-coachee".to_owned(),
            created_at: now.into(),
            updated_at: now.into(),
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([[relationship.clone()]])
            .into_connection();

        let deleted = delete_by_user_id(&db, user_id).await?;

        assert_eq!(deleted, vec![relationship]);
        let log = db.into_transaction_log();
        assert_eq!(log.len(), 1, "one statement deletes and returns: {log:?}");
        let sql = log[0].statements()[0].sql.clone();
        assert!(
            sql.starts_with(
                r#"DELETE FROM "refactor_platform"."coaching_relationships" WHERE "coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2 RETURNING"#
            ),
            "{sql}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn count_with_sessions_by_user_id_counts_only_relationships_with_a_session(
    ) -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results([[std::collections::BTreeMap::from([(
                "num_items".to_string(),
                sea_orm::Value::BigInt(Some(2)),
            )])]])
            .into_connection();

        let user_id = Id::new_v4();
        assert_eq!(count_with_sessions_by_user_id(&db, user_id).await?, 2);

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT COUNT(*) AS num_items FROM (SELECT "coaching_relationships"."id", "coaching_relationships"."organization_id", "coaching_relationships"."coach_id", "coaching_relationships"."coachee_id", CAST("coaching_relationships"."kind" AS "text"), "coaching_relationships"."slug", "coaching_relationships"."created_at", "coaching_relationships"."updated_at" FROM "refactor_platform"."coaching_relationships" WHERE ("coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2) AND EXISTS(SELECT 1 FROM "refactor_platform"."coaching_sessions" WHERE "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id")) AS "sub_query""#,
                [user_id.into(), user_id.into()]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn create_rejects_archived_organization() {
        let now = Utc::now();
        let organization_id = Id::new_v4();
        let archived_org = entity::organizations::Model {
            id: organization_id,
            name: "Archived Org".to_string(),
            logo: None,
            slug: "archived-org".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
            archived_at: Some(now.into()),
            archived_by: Some(Id::new_v4()),
        };

        // create issues find_by_id(org) first; archived org short-circuits before
        // the coach/coachee lookups.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![archived_org]])
            .into_connection();

        let draft = Draft {
            coach_id: Id::new_v4(),
            coachee_id: Id::new_v4(),
            kind: Kind::Coaching,
        };

        let result = create(&db, organization_id, draft).await;

        let err = result.expect_err("expected archived-org rejection");
        assert!(matches!(
            err.error_kind,
            EntityApiErrorKind::OrganizationArchived
        ));
    }

    #[tokio::test]
    async fn create_rejects_a_user_as_their_own_coach() {
        let organization_id = Id::new_v4();
        let user_id = Id::new_v4();

        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let draft = Draft {
            coach_id: user_id,
            coachee_id: user_id,
            kind: Kind::Coaching,
        };

        let err = create(&db, organization_id, draft)
            .await
            .expect_err("expected self-coaching rejection");

        assert!(matches!(
            err.error_kind,
            EntityApiErrorKind::ValidationError { .. }
        ));
        assert!(
            db.into_transaction_log().is_empty(),
            "self-coaching must be rejected before any statement runs"
        );
    }
}

#[cfg(test)]
mod lock_key_tests {
    use super::*;

    #[test]
    fn a_pairs_lock_key_ignores_which_person_is_first_and_depends_on_the_organization() {
        let (organization_id, a, b) = (Id::new_v4(), Id::new_v4(), Id::new_v4());

        assert_eq!(
            pair_lock_key(organization_id, a, b),
            pair_lock_key(organization_id, b, a)
        );
        assert_ne!(
            pair_lock_key(organization_id, a, b),
            pair_lock_key(Id::new_v4(), a, b)
        );
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "coaching_relationship_reuse_tests.rs"]
mod reuse_tests;

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "coaching_relationship_race_tests.rs"]
mod race_tests;

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "coaching_relationship_membership_tests.rs"]
mod membership_tests;
