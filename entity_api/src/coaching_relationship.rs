use super::{
    error::{EntityApiErrorKind, Error},
    organization,
};
use crate::user;
use chrono::Utc;
use entity::{
    coachees, coaches,
    coaching_relationships::{self, ActiveModel, Entity, Model},
    Id,
};
use log::*;
use sea_orm::{
    entity::prelude::*, sea_query::Alias, Condition, DatabaseConnection, FromQueryResult, JoinType,
    QuerySelect, QueryTrait, Set,
};
use serde::ser::{Serialize, SerializeStruct, Serializer};
use slugify::slugify;

pub async fn create(
    db: &impl ConnectionTrait,
    organization_id: Id,
    coaching_relationship_model: Model,
) -> Result<CoachingRelationshipWithUserNames, Error> {
    debug!("New Coaching Relationship Model to be inserted: {coaching_relationship_model:?}");

    let coach = user::find_by_id(db, coaching_relationship_model.coach_id).await?;
    let coachee = user::find_by_id(db, coaching_relationship_model.coachee_id).await?;

    let coach_organization_ids = organization::find_by_user(db, coach.id)
        .await?
        .iter()
        .map(|org| org.id)
        .collect::<Vec<Id>>();
    let coachee_organization_ids = organization::find_by_user(db, coachee.id)
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
            error_kind: EntityApiErrorKind::ValidationError,
        });
    }

    // Coaching Relationship must be unique within the context of an organization
    // Note: this is enforced at the database level as well
    let existing_coaching_relationships = find_by_organization(db, organization_id).await?;
    let existing_coaching_relationship = existing_coaching_relationships.iter().find(|cr| {
        cr.coach_id == coaching_relationship_model.coach_id
            && cr.coachee_id == coaching_relationship_model.coachee_id
    });

    if existing_coaching_relationship.is_some() {
        error!("Coaching relationship already exists for coach: {} and coachee: {} in organization: {}", coaching_relationship_model.coach_id, coaching_relationship_model.coachee_id, coaching_relationship_model.organization_id);
        return Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::ValidationError,
        });
    }

    let now = Utc::now();
    let coach = user::find_by_id(db, coaching_relationship_model.coach_id).await?;
    let coachee = user::find_by_id(db, coaching_relationship_model.coachee_id).await?;
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
    let inserted: Model = coaching_relationship_active_model.insert(db).await?;

    Ok(CoachingRelationshipWithUserNames {
        id: inserted.id,
        coach_id: inserted.coach_id,
        coachee_id: inserted.coachee_id,
        coach_first_name: coach.first_name,
        coach_last_name: coach.last_name,
        coachee_first_name: coachee.first_name,
        coachee_last_name: coachee.last_name,
        created_at: inserted.created_at,
        updated_at: inserted.updated_at,
    })
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

pub async fn find_by_organization(
    db: &impl ConnectionTrait,
    organization_id: Id,
) -> Result<Vec<Model>, Error> {
    let query = by_organization(coaching_relationships::Entity::find(), organization_id).await;

    Ok(query.all(db).await?)
}

pub async fn find_by_organization_with_user_names(
    db: &DatabaseConnection,
    organization_id: Id,
    user_id: Id,
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
        .filter(
            Condition::any()
                .add(coaching_relationships::Column::CoachId.eq(user_id))
                .add(coaching_relationships::Column::CoacheeId.eq(user_id)),
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

// A convenient combined struct that holds the results of looking up the Users associated
// with the coach/coachee ids. This should be used as an implementation detail only.
#[derive(FromQueryResult, Debug, PartialEq)]
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
        let mut state = serializer.serialize_struct("CoachingRelationship", 7)?;
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
                [organization_id.clone().into()]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_organization_with_user_names_returns_all_records_by_organization_with_user_names(
    ) -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let organization_id = Id::new_v4();
        let user_id = Id::new_v4();
        let _ = find_by_organization_with_user_names(&db, organization_id, user_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"SELECT "coaching_relationships"."id", "coaching_relationships"."organization_id", "coaching_relationships"."coach_id", "coaching_relationships"."coachee_id", "coaching_relationships"."created_at", "coaching_relationships"."updated_at", coaches.first_name AS "coach_first_name", coaches.last_name AS "coach_last_name", coachees.first_name AS "coachee_first_name", coachees.last_name AS "coachee_last_name" FROM "refactor_platform"."coaching_relationships" JOIN "refactor_platform"."users" AS "coaches" ON "coaching_relationships"."coach_id" = "coaches"."id" JOIN "refactor_platform"."users" AS "coachees" ON "coaching_relationships"."coachee_id" = "coachees"."id" WHERE "coaching_relationships"."organization_id" IN (SELECT "organizations"."id" FROM "refactor_platform"."organizations" WHERE "organizations"."id" = $1) AND ("coaching_relationships"."coach_id" = $2 OR "coaching_relationships"."coachee_id" = $3)"#,
                [
                    organization_id.clone().into(),
                    user_id.clone().into(),
                    user_id.clone().into()
                ]
            )]
        );

        Ok(())
    }

    #[tokio::test]
    async fn create_returns_validation_error_for_duplicate_relationship() -> Result<(), Error> {
        use entity::coaching_relationships::Model;
        use sea_orm::{DatabaseBackend, MockDatabase};

        let organization_id = Id::new_v4();
        let coach_id = Id::new_v4();
        let coachee_id = Id::new_v4();
        let coach_organization_id = Id::new_v4();
        let coachee_organization_id = Id::new_v4();

        let coach_user = entity::users::Model {
            id: coach_id.clone(),
            first_name: "Coach".to_string(),
            last_name: "User".to_string(),
            email: "coach@example.com".to_string(),
            password: "hash".to_string(),
            display_name: Some("Coach User".to_string()),
            github_username: Some("coach_user".to_string()),
            role: entity::users::Role::User,
            github_profile_url: Some("https://github.com/coach_user".to_string()),
            timezone: "UTC".to_string(),
            created_at: chrono::Utc::now().into(),
            updated_at: chrono::Utc::now().into(),
        };

        let coachee_user = entity::users::Model {
            id: coachee_id.clone(),
            first_name: "Coachee".to_string(),
            last_name: "User".to_string(),
            email: "coachee@example.com".to_string(),
            password: "hash".to_string(),
            display_name: Some("Coachee User".to_string()),
            github_username: Some("coachee_user".to_string()),
            role: entity::users::Role::User,
            github_profile_url: Some("https://github.com/coachee_user".to_string()),
            timezone: "UTC".to_string(),
            created_at: chrono::Utc::now().into(),
            updated_at: chrono::Utc::now().into(),
        };

        let coaching_relationships = vec![Model {
            id: Id::new_v4(),
            organization_id: organization_id.clone(),
            coach_id: coach_id.clone(),
            coachee_id: coachee_id.clone(),
            slug: "coach-coachee".to_string(),
            created_at: chrono::Utc::now().into(),
            updated_at: chrono::Utc::now().into(),
        }];

        let coach_organization = entity::organizations::Model {
            id: coach_organization_id,
            name: "Organization".to_string(),
            slug: "organization".to_string(),
            logo: None,
            created_at: chrono::Utc::now().into(),
            updated_at: chrono::Utc::now().into(),
        };

        let coachee_organization = entity::organizations::Model {
            id: coachee_organization_id,
            name: "Organization".to_string(),
            slug: "organization".to_string(),
            logo: None,
            created_at: chrono::Utc::now().into(),
            updated_at: chrono::Utc::now().into(),
        };

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![coach_user]])
            .append_query_results(vec![vec![coachee_user]])
            .append_query_results(vec![vec![coach_organization]])
            .append_query_results(vec![vec![coachee_organization]])
            .append_query_results(vec![coaching_relationships])
            .into_connection();

        let model = Model {
            id: Id::new_v4(),
            organization_id: organization_id.clone(),
            coach_id: coach_id.clone(),
            coachee_id: coachee_id.clone(),
            slug: "coach-coachee".to_string(),
            created_at: chrono::Utc::now().into(),
            updated_at: chrono::Utc::now().into(),
        };

        let result = create(&db, organization_id, model).await;
        println!("Result: {:?}", result);
        assert!(
            result
                == Err(Error {
                    source: None,
                    error_kind: EntityApiErrorKind::ValidationError,
                })
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
                [user_id.clone().into(), user_id.clone().into()]
            )]
        );

        Ok(())
    }
}
