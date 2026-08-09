use super::{
    error::{EntityApiErrorKind, Error},
    organization,
};
use crate::user;
use chrono::Utc;
use entity::{
    coachees, coaches,
    coaching_relationships::{self, ActiveModel, Entity, Model},
    users, Id,
};
use log::*;
use sea_orm::{
    entity::prelude::*, sea_query::Alias, sea_query::OnConflict, Condition, DatabaseConnection,
    DbErr, FromQueryResult, JoinType, QuerySelect, QueryTrait, Set,
};
use serde::ser::{SerializeStruct, Serializer};
use serde::Serialize;
use slugify::slugify;
use utoipa::ToSchema;

/// Creates a coaching relationship between a coach and a coachee in an organization.
///
/// Returns the existing relationship when one already exists for that coach, coachee
/// and organization.
pub async fn create(
    db: &impl ConnectionTrait,
    organization_id: Id,
    coaching_relationship_model: Model,
) -> Result<CoachingRelationshipWithUserNames, Error> {
    debug!("New Coaching Relationship Model to be inserted: {coaching_relationship_model:?}");

    if coaching_relationship_model.coach_id == coaching_relationship_model.coachee_id {
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::ValidationError {
                message: "A user cannot be their own coach.".into(),
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

    let coach = user::find_by_id(db, coaching_relationship_model.coach_id).await?;
    let coachee = user::find_by_id(db, coaching_relationship_model.coachee_id).await?;

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
        error!("Coach and coachee do not belong to the correct organization, not creating requested new coaching relationship between coach: {:?} and coachee: {:?} for organization: {:?}.", coaching_relationship_model.coach_id, coaching_relationship_model.coachee_id, organization_id);
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::ValidationError {
                message: "Coach and coachee must belong to the specified organization.".into(),
                details: None,
            },
        });
    }

    // Coaching Relationship must be unique within the context of an organization
    // Note: this is enforced at the database level as well
    let existing_coaching_relationship = find_for_pair(
        db,
        organization_id,
        coaching_relationship_model.coach_id,
        coaching_relationship_model.coachee_id,
    )
    .await?;

    if let Some(existing) = existing_coaching_relationship {
        debug!("Reusing existing coaching relationship: {existing:?}");
        return Ok(with_user_names(existing, &coach, &coachee));
    }

    let now = Utc::now();
    let slug = slugify!(format!("{} {}", coach.first_name, coachee.first_name).as_str());

    let coaching_relationship_active_model: ActiveModel = ActiveModel {
        organization_id: Set(organization_id),
        coach_id: Set(coaching_relationship_model.coach_id),
        coachee_id: Set(coaching_relationship_model.coachee_id),
        slug: Set(slug),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };
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

    match Entity::insert(coaching_relationship_active_model)
        .on_conflict(conflict)
        .exec_with_returning(db)
        .await
    {
        Ok(inserted) => Ok(with_user_names(inserted, &coach, &coachee)),
        // DO NOTHING wrote no row, so RETURNING yielded none and SeaORM reports the
        // miss as RecordNotFound. Only a conflict can produce that here, since this
        // statement inserts exactly one row.
        Err(DbErr::RecordNotFound(_)) => {
            let winner = find_for_pair(
                db,
                organization_id,
                coaching_relationship_model.coach_id,
                coaching_relationship_model.coachee_id,
            )
            .await?
            .ok_or_else(|| Error {
                source: None,
                error_kind: EntityApiErrorKind::RecordNotFound,
            })?;
            debug!("Lost the create race, reusing the winning relationship: {winner:?}");
            Ok(with_user_names(winner, &coach, &coachee))
        }
        Err(err) => Err(err.into()),
    }
}

/// The relationship for exactly this coach, coachee and organization, if one exists.
///
/// Shared by the pre-check and the lost-race recovery so both agree on what "the same
/// relationship" means. Directional: swapping coach and coachee is a different pair.
async fn find_for_pair(
    db: &impl ConnectionTrait,
    organization_id: Id,
    coach_id: Id,
    coachee_id: Id,
) -> Result<Option<Model>, Error> {
    Ok(find_by_organization(db, organization_id)
        .await?
        .into_iter()
        .find(|cr| cr.coach_id == coach_id && cr.coachee_id == coachee_id))
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

/// Pairs a coaching relationship with the names of its coach and coachee.
fn with_user_names(
    coaching_relationship: Model,
    coach: &users::Model,
    coachee: &users::Model,
) -> CoachingRelationshipWithUserNames {
    CoachingRelationshipWithUserNames {
        id: coaching_relationship.id,
        coach_id: coaching_relationship.coach_id,
        coachee_id: coaching_relationship.coachee_id,
        coach_first_name: coach.first_name.clone(),
        coach_last_name: coach.last_name.clone(),
        coachee_first_name: coachee.first_name.clone(),
        coachee_last_name: coachee.last_name.clone(),
        created_at: coaching_relationship.created_at,
        updated_at: coaching_relationship.updated_at,
    }
}

pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
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

/// Checks if a user is a coach of another user.
///
/// Returns `true` if there exists a coaching relationship where
/// `potential_coach_id` is the coach and `potential_coachee_id` is the coachee.
pub async fn is_coach_of(
    db: &DatabaseConnection,
    potential_coach_id: Id,
    potential_coachee_id: Id,
) -> Result<bool, Error> {
    let relationship = coaching_relationships::Entity::find()
        .filter(
            Condition::all()
                .add(coaching_relationships::Column::CoachId.eq(potential_coach_id))
                .add(coaching_relationships::Column::CoacheeId.eq(potential_coachee_id)),
        )
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
    let coaches = Alias::new("coaches");
    let coachees = Alias::new("coachees");

    let query = by_organization(coaching_relationships::Entity::find(), organization_id)
        .await
        .join_as(
            JoinType::Join,
            coaches::Relation::CoachingRelationships.def().rev(),
            coaches.clone(),
        )
        .join_as(
            JoinType::Join,
            coachees::Relation::CoachingRelationships.def().rev(),
            coachees.clone(),
        )
        .select_only()
        .column(coaching_relationships::Column::Id)
        .column(coaching_relationships::Column::OrganizationId)
        .column(coaching_relationships::Column::CoachId)
        .column(coaching_relationships::Column::CoacheeId)
        .column(coaching_relationships::Column::CreatedAt)
        .column(coaching_relationships::Column::UpdatedAt)
        .column_as(Expr::cust("coaches.first_name"), "coach_first_name")
        .column_as(Expr::cust("coaches.last_name"), "coach_last_name")
        .column_as(Expr::cust("coachees.first_name"), "coachee_first_name")
        .column_as(Expr::cust("coachees.last_name"), "coachee_last_name")
        .into_model::<CoachingRelationshipWithUserNames>();

    Ok(query.all(db).await?)
}

pub async fn find_by_user_and_organization_with_user_names(
    db: &impl ConnectionTrait,
    user_id: Id,
    organization_id: Id,
) -> Result<Vec<CoachingRelationshipWithUserNames>, Error> {
    let coaches = Alias::new("coaches");
    let coachees = Alias::new("coachees");

    let query = by_organization(coaching_relationships::Entity::find(), organization_id)
        .await
        .filter(
            Condition::any()
                .add(coaching_relationships::Column::CoachId.eq(user_id))
                .add(coaching_relationships::Column::CoacheeId.eq(user_id)),
        )
        .join_as(
            JoinType::Join,
            coaches::Relation::CoachingRelationships.def().rev(),
            coaches.clone(),
        )
        .join_as(
            JoinType::Join,
            coachees::Relation::CoachingRelationships.def().rev(),
            coachees.clone(),
        )
        .select_only()
        .column(coaching_relationships::Column::Id)
        .column(coaching_relationships::Column::OrganizationId)
        .column(coaching_relationships::Column::CoachId)
        .column(coaching_relationships::Column::CoacheeId)
        .column(coaching_relationships::Column::CreatedAt)
        .column(coaching_relationships::Column::UpdatedAt)
        .column_as(Expr::cust("coaches.first_name"), "coach_first_name")
        .column_as(Expr::cust("coaches.last_name"), "coach_last_name")
        .column_as(Expr::cust("coachees.first_name"), "coachee_first_name")
        .column_as(Expr::cust("coachees.last_name"), "coachee_last_name")
        .into_model::<CoachingRelationshipWithUserNames>();

    Ok(query.all(db).await?)
}

pub async fn get_relationship_with_user_names(
    db: &DatabaseConnection,
    relationship_id: Id,
) -> Result<Option<CoachingRelationshipWithUserNames>, Error> {
    let coaches = Alias::new("coaches");
    let coachees = Alias::new("coachees");

    let query = by_coaching_relationship(coaching_relationships::Entity::find(), relationship_id)
        .await
        .join_as(
            JoinType::Join,
            coaches::Relation::CoachingRelationships.def().rev(),
            coaches.clone(),
        )
        .join_as(
            JoinType::Join,
            coachees::Relation::CoachingRelationships.def().rev(),
            coachees.clone(),
        )
        .select_only()
        .column(coaching_relationships::Column::Id)
        .column(coaching_relationships::Column::OrganizationId)
        .column(coaching_relationships::Column::CoachId)
        .column(coaching_relationships::Column::CoacheeId)
        .column(coaching_relationships::Column::CreatedAt)
        .column(coaching_relationships::Column::UpdatedAt)
        .column_as(Expr::cust("coaches.first_name"), "coach_first_name")
        .column_as(Expr::cust("coaches.last_name"), "coach_last_name")
        .column_as(Expr::cust("coachees.first_name"), "coachee_first_name")
        .column_as(Expr::cust("coachees.last_name"), "coachee_last_name")
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

    query.filter(coaching_relationships::Column::Id.in_subquery(relationship_subsquery.to_owned()))
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

    query.filter(
        coaching_relationships::Column::OrganizationId
            .in_subquery(organization_subquery.to_owned()),
    )
}

pub async fn delete_by_user_id(db: &impl ConnectionTrait, user_id: Id) -> Result<(), Error> {
    Entity::delete_many()
        .filter(
            Condition::any()
                .add(coaching_relationships::Column::CoachId.eq(user_id))
                .add(coaching_relationships::Column::CoacheeId.eq(user_id)),
        )
        .exec(db)
        .await?;
    Ok(())
}

/// Trait for filtering coaching relationships by user's role.
///
/// Implement this trait in the web layer to define role-based filtering
/// while keeping the entity_api layer decoupled from web-specific types.
pub trait RoleFilterable {
    /// Returns true if only relationships where the user is a coach should be returned.
    fn filter_coach_only(&self) -> bool;
    /// Returns true if only relationships where the user is a coachee should be returned.
    fn filter_coachee_only(&self) -> bool;
}

/// Finds coaching relationships for a user with optional role filtering.
///
/// Returns relationships with user names for display purposes.
pub async fn find_by_user_id_with_user_names(
    db: &DatabaseConnection,
    user_id: Id,
    role_filter: impl RoleFilterable,
) -> Result<Vec<CoachingRelationshipWithUserNames>, Error> {
    let coaches = Alias::new("coaches");
    let coachees = Alias::new("coachees");

    let filter = if role_filter.filter_coach_only() {
        Condition::all().add(coaching_relationships::Column::CoachId.eq(user_id))
    } else if role_filter.filter_coachee_only() {
        Condition::all().add(coaching_relationships::Column::CoacheeId.eq(user_id))
    } else {
        // Default: return all relationships where user is coach or coachee
        Condition::any()
            .add(coaching_relationships::Column::CoachId.eq(user_id))
            .add(coaching_relationships::Column::CoacheeId.eq(user_id))
    };

    let query = coaching_relationships::Entity::find()
        .filter(filter)
        .join_as(
            JoinType::Join,
            coaches::Relation::CoachingRelationships.def().rev(),
            coaches.clone(),
        )
        .join_as(
            JoinType::Join,
            coachees::Relation::CoachingRelationships.def().rev(),
            coachees.clone(),
        )
        .select_only()
        .column(coaching_relationships::Column::Id)
        .column(coaching_relationships::Column::OrganizationId)
        .column(coaching_relationships::Column::CoachId)
        .column(coaching_relationships::Column::CoacheeId)
        .column(coaching_relationships::Column::CreatedAt)
        .column(coaching_relationships::Column::UpdatedAt)
        .column_as(Expr::cust("coaches.first_name"), "coach_first_name")
        .column_as(Expr::cust("coaches.last_name"), "coach_last_name")
        .column_as(Expr::cust("coachees.first_name"), "coachee_first_name")
        .column_as(Expr::cust("coachees.last_name"), "coachee_last_name")
        .into_model::<CoachingRelationshipWithUserNames>();

    Ok(query.all(db).await?)
}

// A convenient combined struct that holds the results of looking up the Users associated
// with the coach/coachee ids.
#[derive(FromQueryResult, Debug, PartialEq, Clone, ToSchema)]
#[schema(as = domain::coaching_relationship::CoachingRelationshipWithUserNames)]
pub struct CoachingRelationshipWithUserNames {
    pub id: Id,
    pub coach_id: Id,
    pub coachee_id: Id,
    pub coach_first_name: String,
    pub coach_last_name: String,
    pub coachee_first_name: String,
    pub coachee_last_name: String,
    pub created_at: DateTimeWithTimeZone,
    pub updated_at: DateTimeWithTimeZone,
}

// serialize the CoachingRelationshipUserWithNames struct so that it can be used in the API
// and appears to be a coaching_relationship JSON object.
impl Serialize for CoachingRelationshipWithUserNames {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("CoachingRelationship", 9)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("coach_id", &self.coach_id)?;
        state.serialize_field("coachee_id", &self.coachee_id)?;
        state.serialize_field("coach_first_name", &self.coach_first_name)?;
        state.serialize_field("coach_last_name", &self.coach_last_name)?;
        state.serialize_field("coachee_first_name", &self.coachee_first_name)?;
        state.serialize_field("coachee_last_name", &self.coachee_last_name)?;
        state.serialize_field("created_at", &self.created_at)?;
        state.serialize_field("updated_at", &self.updated_at)?;
        state.end()
    }
}

#[cfg(test)]
// We need to gate seaORM's mock feature behind conditional compilation because
// the feature removes the Clone trait implementation from seaORM's DatabaseConnection.
// see https://github.com/SeaQL/sea-orm/issues/830
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
                r#"SELECT "coaching_relationships"."id", "coaching_relationships"."organization_id", "coaching_relationships"."coach_id", "coaching_relationships"."coachee_id", "coaching_relationships"."slug", "coaching_relationships"."created_at", "coaching_relationships"."updated_at" FROM "refactor_platform"."coaching_relationships" WHERE "coaching_relationships"."id" = $1 LIMIT $2"#,
                [
                    coaching_relationship_id.into(),
                    sea_orm::Value::BigUnsigned(Some(1))
                ]
            )]
        );

        Ok(())
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
                r#"SELECT "coaching_relationships"."id", "coaching_relationships"."organization_id", "coaching_relationships"."coach_id", "coaching_relationships"."coachee_id", "coaching_relationships"."slug", "coaching_relationships"."created_at", "coaching_relationships"."updated_at" FROM "refactor_platform"."coaching_relationships" WHERE "coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2"#,
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
                r#"SELECT "coaching_relationships"."id", "coaching_relationships"."organization_id", "coaching_relationships"."coach_id", "coaching_relationships"."coachee_id", "coaching_relationships"."slug", "coaching_relationships"."created_at", "coaching_relationships"."updated_at" FROM "refactor_platform"."coaching_relationships" WHERE "coaching_relationships"."organization_id" IN (SELECT "organizations"."id" FROM "refactor_platform"."organizations" WHERE "organizations"."id" = $1)"#,
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
                r#"SELECT "coaching_relationships"."id", "coaching_relationships"."organization_id", "coaching_relationships"."coach_id", "coaching_relationships"."coachee_id", "coaching_relationships"."created_at", "coaching_relationships"."updated_at", coaches.first_name AS "coach_first_name", coaches.last_name AS "coach_last_name", coachees.first_name AS "coachee_first_name", coachees.last_name AS "coachee_last_name" FROM "refactor_platform"."coaching_relationships" JOIN "refactor_platform"."users" AS "coaches" ON "coaching_relationships"."coach_id" = "coaches"."id" JOIN "refactor_platform"."users" AS "coachees" ON "coaching_relationships"."coachee_id" = "coachees"."id" WHERE "coaching_relationships"."organization_id" IN (SELECT "organizations"."id" FROM "refactor_platform"."organizations" WHERE "organizations"."id" = $1)"#,
                [organization_id.into()]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn delete_by_user_id_deletes_all_records_associated_with_user() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let user_id = Id::new_v4();
        let _ = delete_by_user_id(&db, user_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"DELETE FROM "refactor_platform"."coaching_relationships" WHERE "coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2"#,
                [user_id.into(), user_id.into()]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn is_coach_of_returns_true_when_relationship_exists() -> Result<(), Error> {
        let now = chrono::Utc::now();
        let coach_id = Id::new_v4();
        let coachee_id = Id::new_v4();

        let relationship = Model {
            id: Id::new_v4(),
            organization_id: Id::new_v4(),
            coach_id,
            coachee_id,
            slug: "test-relationship".to_string(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![relationship]])
            .into_connection();

        let result = is_coach_of(&db, coach_id, coachee_id).await?;

        assert!(result);
        Ok(())
    }

    #[tokio::test]
    async fn is_coach_of_returns_false_when_no_relationship() -> Result<(), Error> {
        let coach_id = Id::new_v4();
        let coachee_id = Id::new_v4();

        // Return empty result - no relationship exists
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<Model>::new()])
            .into_connection();

        let result = is_coach_of(&db, coach_id, coachee_id).await?;

        assert!(!result);
        Ok(())
    }

    #[tokio::test]
    async fn is_coach_of_returns_false_when_roles_reversed() -> Result<(), Error> {
        let coach_id = Id::new_v4();
        let coachee_id = Id::new_v4();

        // Query with reversed roles returns no result
        // (coachee trying to access coach's data)
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<Model>::new()])
            .into_connection();

        // coachee_id is passed as potential_coach, coach_id as potential_coachee
        let result = is_coach_of(&db, coachee_id, coach_id).await?;

        assert!(!result);
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

        let model = Model {
            id: Id::new_v4(),
            organization_id,
            coach_id: Id::new_v4(),
            coachee_id: Id::new_v4(),
            slug: String::new(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let result = create(&db, organization_id, model).await;

        let err = result.expect_err("expected archived-org rejection");
        assert!(matches!(
            err.error_kind,
            EntityApiErrorKind::OrganizationArchived
        ));
    }

    #[tokio::test]
    async fn create_rejects_a_user_as_their_own_coach() {
        let now = Utc::now();
        let organization_id = Id::new_v4();
        let user_id = Id::new_v4();

        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let model = Model {
            id: Id::new_v4(),
            organization_id,
            coach_id: user_id,
            coachee_id: user_id,
            slug: String::new(),
            created_at: now.into(),
            updated_at: now.into(),
        };

        let err = create(&db, organization_id, model)
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
#[cfg(feature = "mock")]
#[path = "coaching_relationship_reuse_tests.rs"]
mod reuse_tests;

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "coaching_relationship_race_tests.rs"]
mod race_tests;
