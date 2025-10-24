# Remove organizations_users Table

## Overview

Remove the redundant `organizations_users` table and migrate to using `user_roles` exclusively. Every user must have one or more user_role records, and users only have access to organizations they have a user_role for. This makes organizations_users redundant.

## Background

**Current State:**
- **organizations_users**: id, organization_id, user_id, created_at, updated_at
- **user_roles**: id, role (SuperAdmin/Admin/User), organization_id (nullable), user_id, created_at, updated_at
  - Constraint: SuperAdmin roles must have NULL organization_id
  - Constraint: Admin/User roles must have non-NULL organization_id

**Problem:**
- Both tables track user-organization associations
- `create_by_organization()` creates BOTH organizations_users AND user_roles (redundant)
- Queries join on organizations_users even though user_roles contains the same data
- Deleting users requires managing both tables

## Implementation Plan

### Phase 1: Create user_roles API module

**Create** `entity_api/src/user_roles.rs`:
```rust
use super::error::Error;
use entity::user_roles::{Column, Entity};
use entity::Id;
use sea_orm::{ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter};

pub async fn delete_by_user_id(db: &impl ConnectionTrait, user_id: Id) -> Result<(), Error> {
    Entity::delete_many()
        .filter(Condition::all().add(Column::UserId.eq(user_id)))
        .exec(db)
        .await?;
    Ok(())
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod test {
    use super::*;
    use entity::Id;
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

    #[tokio::test]
    async fn test_delete_by_user_id() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();

        let user_id = Id::new_v4();
        let _ = delete_by_user_id(&db, user_id).await;

        assert_eq!(
            db.into_transaction_log(),
            [Transaction::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"DELETE FROM "refactor_platform"."user_roles" WHERE "user_roles"."user_id" = $1"#,
                [user_id.into()]
            )]
        );

        Ok(())
    }
}
```

**Update** `entity_api/src/lib.rs`:
- Add after line 21: `pub mod user_roles;`

### Phase 2: Update domain layer

**Update** `domain/src/user.rs`:
- Line 8: Change import from `organizations_user` to `user_roles`:
  ```rust
  use entity_api::{
      coaching_relationship, mutate, user_roles, // Changed from organizations_user
      query::{IntoQueryFilterMap, QuerySort},
      user,
  };
  ```
- Line 145: Change function call:
  ```rust
  coaching_relationship::delete_by_user_id(&txn, user_id).await?;
  user_roles::delete_by_user_id(&txn, user_id).await?; // Changed from organizations_user
  user::delete(&txn, user_id).await?;
  ```

### Phase 3: Update entity layer

**Update** `entity/src/organizations.rs`:
- Line 31-35: Add UserRoles relation, keep CoachingRelationships:
  ```rust
  #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
  pub enum Relation {
      #[sea_orm(has_many = "super::coaching_relationships::Entity")]
      CoachingRelationships,

      #[sea_orm(has_many = "super::user_roles::Entity")]
      UserRoles,
  }
  ```
- Lines 74-85: Update `Related<users::Entity>` to use user_roles:
  ```rust
  // Through relationship for users by way of user_roles
  // organizations -> user_roles -> users
  impl Related<super::users::Entity> for Entity {
      fn to() -> RelationDef {
          super::user_roles::Relation::Users.def()
      }

      fn via() -> Option<RelationDef> {
          Some(
              super::user_roles::Relation::Organizations
                  .def()
                  .rev(),
          )
      }
  }
  ```

**Update** `entity/src/users.rs`:
- Lines 47-52: Remove OrganizationsUsers, keep UserRoles:
  ```rust
  #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
  pub enum Relation {
      #[sea_orm(has_many = "super::user_roles::Entity")]
      UserRoles,
  }
  ```
- Lines 54-62: Update `Related<organizations::Entity>` to use user_roles:
  ```rust
  impl Related<super::organizations::Entity> for Entity {
      fn to() -> RelationDef {
          super::user_roles::Relation::Organizations.def()
      }

      fn via() -> Option<RelationDef> {
          Some(super::user_roles::Relation::Users.def().rev())
      }
  }
  ```

**Delete** `entity/src/organizations_users.rs`

**Update** `entity/src/lib.rs`:
- Line 14: Remove `pub mod organizations_users;`

### Phase 4: Update entity_api layer

**Update** `entity_api/src/user.rs`:
- Line 7: Remove `organizations_users` from imports:
  ```rust
  use entity::{organizations, user_roles, roles, Id};
  ```
- Lines 54-62: **Remove entire organizations_users insert block** in `create_by_organization()`:
  ```rust
  // DELETE THESE LINES:
  let organization_user = organizations_users::ActiveModel {
      organization_id: Set(organization_id),
      user_id: Set(user.id),
      created_at: Set(now.into()),
      updated_at: Set(now.into()),
      ..Default::default()
  };
  organization_user.insert(&txn).await?;
  ```
- Lines 119-132: Update `find_by_organization()` to use user_roles (optimized with single join):
  ```rust
  pub async fn find_by_organization(
      db: &DatabaseConnection,
      organization_id: Id,
  ) -> Result<Vec<Model>, Error> {
      let results = Entity::find()
          .inner_join(user_roles::Entity)
          .filter(user_roles::Column::OrganizationId.eq(organization_id))
          .find_with_related(user_roles::Entity)
          .all(db)
          .await?;

      Ok(results
          .into_iter()
          .map(|(mut user, roles)| {
              user.roles = roles;
              user
          })
          .collect())
  }
  ```

**Update** `entity_api/src/organization.rs`:
- Line 4: Remove `organizations_users` import:
  ```rust
  use entity::{organizations::*, user_roles, prelude::Organizations, Id};
  ```
- Lines 93-98: Update `by_user()` to use user_roles:
  ```rust
  async fn by_user(query: Select<Organizations>, user_id: Id) -> Select<Organizations> {
      query
          .join(JoinType::InnerJoin, Relation::UserRoles.def())
          .filter(user_roles::Column::UserId.eq(user_id))
          .distinct()
  }
  ```

**Delete** `entity_api/src/organizations_user.rs`

**Update** `entity_api/src/lib.rs`:
- Line 7: Remove `organizations_users` from exports:
  ```rust
  pub use entity::{
      actions, agreements, coachees, coaches, coaching_relationships, coaching_sessions, jwts, notes,
      organizations, overarching_goals, user_roles, users, users::Role, Id,
  };
  ```
- Line 18: Remove module: `pub mod organizations_user;`
- Lines 149-215: Update `seed_database()` to remove organizations_users creation:
  ```rust
  // DELETE THESE BLOCKS (lines 149-158, 160-169, 184-193, 195-204, 206-215):
  let _jim_refactor_coaching = organizations_users::ActiveModel { ... };
  let _caleb_refactor_coaching = organizations_users::ActiveModel { ... };
  let _caleb_acme_corp = organizations_users::ActiveModel { ... };
  let _jim_acme_corp = organizations_users::ActiveModel { ... };
  let _other_user_acme_corp = organizations_users::ActiveModel { ... };

  // Note: user_roles should already be created elsewhere in seed logic
  // If not, add user_roles creation here
  ```

### Phase 5: Migration

**Create** `migration/src/m20251023_000000_remove_organizations_users_table.rs`:

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Validate all organizations_users have matching user_roles
        let conn = manager.get_connection();
        let backend = conn.get_database_backend();

        let validation_sql = r#"
            SELECT COUNT(*) as orphan_count
            FROM refactor_platform.organizations_users ou
            LEFT JOIN refactor_platform.user_roles ur
              ON ou.user_id = ur.user_id
              AND ou.organization_id = ur.organization_id
            WHERE ur.id IS NULL
        "#;

        let result = conn
            .query_one(Statement::from_string(backend, validation_sql))
            .await?
            .ok_or_else(|| DbErr::Custom("Validation query failed".to_string()))?;

        let count: i64 = result
            .try_get("", "orphan_count")
            .map_err(|e| DbErr::Custom(format!("Failed to parse count: {}", e)))?;

        if count > 0 {
            return Err(DbErr::Custom(format!(
                "Found {} organizations_users records without matching user_roles. \
                Each organizations_users record must have a corresponding user_roles record \
                with the same user_id and organization_id before this table can be removed.",
                count
            )));
        }

        // Create index on user_roles.organization_id for optimized queries
        // This is needed because queries previously using organizations_users will now use user_roles
        manager
            .create_index(
                Index::create()
                    .name("idx_user_roles_organization_id")
                    .table(Alias::new("refactor_platform.user_roles"))
                    .col(Alias::new("organization_id"))
                    .to_owned(),
            )
            .await?;

        // Drop the organizations_users table
        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new("refactor_platform.organizations_users"))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Recreate organizations_users table
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("refactor_platform.organizations_users"))
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .uuid()
                            .not_null()
                            .primary_key()
                            .extra("DEFAULT gen_random_uuid()"),
                    )
                    .col(ColumnDef::new(Alias::new("organization_id")).uuid().not_null())
                    .col(ColumnDef::new(Alias::new("user_id")).uuid().not_null())
                    .col(
                        ColumnDef::new(Alias::new("created_at"))
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT now()"),
                    )
                    .col(
                        ColumnDef::new(Alias::new("updated_at"))
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT now()"),
                    )
                    .to_owned(),
            )
            .await?;

        // Add foreign key constraints
        manager
            .create_foreign_key(
                ForeignKey::create()
                    .name("fk_organizations_users_organization")
                    .from(
                        Alias::new("refactor_platform.organizations_users"),
                        Alias::new("organization_id"),
                    )
                    .to(
                        Alias::new("refactor_platform.organizations"),
                        Alias::new("id"),
                    )
                    .on_delete(ForeignKeyAction::NoAction)
                    .on_update(ForeignKeyAction::NoAction)
                    .to_owned(),
            )
            .await?;

        manager
            .create_foreign_key(
                ForeignKey::create()
                    .name("fk_organizations_users_user")
                    .from(
                        Alias::new("refactor_platform.organizations_users"),
                        Alias::new("user_id"),
                    )
                    .to(
                        Alias::new("refactor_platform.users"),
                        Alias::new("id"),
                    )
                    .on_delete(ForeignKeyAction::NoAction)
                    .on_update(ForeignKeyAction::NoAction)
                    .to_owned(),
            )
            .await?;

        // Add unique constraint to prevent duplicate entries on rollback
        manager
            .create_index(
                Index::create()
                    .name("idx_organizations_users_unique")
                    .table(Alias::new("refactor_platform.organizations_users"))
                    .col(Alias::new("user_id"))
                    .col(Alias::new("organization_id"))
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Repopulate from user_roles where organization_id IS NOT NULL
        let repopulate_sql = r#"
            INSERT INTO refactor_platform.organizations_users (user_id, organization_id, created_at, updated_at)
            SELECT user_id, organization_id, created_at, updated_at
            FROM refactor_platform.user_roles
            WHERE organization_id IS NOT NULL
            ON CONFLICT (user_id, organization_id) DO NOTHING
        "#;

        manager
            .get_connection()
            .execute_unprepared(repopulate_sql)
            .await?;

        // Drop the index that was added in up()
        manager
            .drop_index(
                Index::drop()
                    .name("idx_user_roles_organization_id")
                    .table(Alias::new("refactor_platform.user_roles"))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
```

**Update** `migration/src/lib.rs`:
- Add the migration to the list

### Phase 6: Update tests

**Update** `entity_api/src/user.rs`:
- Lines 321-340: Remove organization_user_model from `create_by_organization_returns_a_new_user_model` test:
  ```rust
  // DELETE THESE LINES:
  let organization_user_model = entity::organizations_users::Model {
      id: Id::new_v4(),
      organization_id,
      user_id,
      created_at: now.into(),
      updated_at: now.into(),
  };

  // And remove from MockDatabase:
  .append_query_results([[organization_user_model.clone()]])  // DELETE
  ```

**Update** `entity_api/src/organization.rs`:
- Lines 147-154: Update test SQL expectation in `find_by_user_returns_all_records_associated_with_user`:
  ```rust
  assert_eq!(
      db.into_transaction_log(),
      [Transaction::from_sql_and_values(
          DatabaseBackend::Postgres,
          r#"SELECT DISTINCT "organizations"."id", "organizations"."name", "organizations"."logo", "organizations"."slug", "organizations"."created_at", "organizations"."updated_at" FROM "refactor_platform"."organizations" INNER JOIN "refactor_platform"."user_roles" ON "organizations"."id" = "user_roles"."organization_id" WHERE "user_roles"."user_id" = $1"#,
          [user_id.into()]
      )]
  );
  ```

**Update** `entity_api/src/user.rs`:
- Line 290: Update test SQL expectation in `find_by_organization_returns_users_who_are_coaches_or_coachees`:
  ```rust
  // Updated to reflect optimized query with single join
  r#"SELECT "users"."id" AS "A_id", "users"."email" AS "A_email", "users"."first_name" AS "A_first_name", "users"."last_name" AS "A_last_name", "users"."display_name" AS "A_display_name", "users"."password" AS "A_password", "users"."github_username" AS "A_github_username", "users"."github_profile_url" AS "A_github_profile_url", "users"."timezone" AS "A_timezone", CAST("users"."role" AS "text") AS "A_role", "users"."created_at" AS "A_created_at", "users"."updated_at" AS "A_updated_at", "user_roles"."id" AS "B_id", CAST("user_roles"."role" AS "text") AS "B_role", "user_roles"."organization_id" AS "B_organization_id", "user_roles"."user_id" AS "B_user_id", "user_roles"."created_at" AS "B_created_at", "user_roles"."updated_at" AS "B_updated_at" FROM "refactor_platform"."users" INNER JOIN "refactor_platform"."user_roles" ON "users"."id" = "user_roles"."user_id" WHERE "user_roles"."organization_id" = $1 ORDER BY "users"."id" ASC"#,
  ```

  **Note**: The exact SQL string format should be verified against actual SeaORM output during implementation.

### Phase 7: Update documentation

**Update** `docs/db/refactor_platform_rs.dbml`:
- Remove the entire `Table organizations_users` block
- Remove any references to `Ref: organizations_users.organization_id > organizations.id`
- Remove any references to `Ref: organizations_users.user_id > users.id`

## Validation Checklist

Before running the migration:
- [ ] All code changes implemented
- [ ] All tests updated and passing
- [ ] Verify in staging that all organizations_users have matching user_roles
- [ ] No new code creates organizations_users records

After running the migration:
- [ ] Verify user deletion still works
- [ ] Verify find_by_organization returns correct users
- [ ] Verify organization.find_by_user returns correct organizations
- [ ] Verify user creation for organization works
- [ ] Verify UserInOrganization authorization check works

## Rollback Plan

If issues are discovered after deployment:
1. Run migration down: `sea-orm-cli migrate down`
2. This will recreate organizations_users table and repopulate from user_roles
3. Revert code changes via git
4. Redeploy previous version

## Notes

- **Foreign Key Cascade**: organizations_users has NO CASCADE (defaults to RESTRICT), while user_roles HAS CASCADE on delete. After migration, user deletions will automatically cascade to user_roles, but we keep explicit deletion in domain layer for transaction control.
- **Seed Data**: The seed_database() function creates test organizations_users records. These should be removed as part of this migration.
- **Old Migrations**: Migration `m20250509_164646_add_initial_user.rs` creates and deletes organizations_users records. This can be left as-is since it's an old migration that won't be re-run.
