use super::error::{EntityApiErrorKind, Error};
use entity::coaching_session_images::{ActiveModel, Column, Entity, Model};
use entity::Id;
use sea_orm::sea_query::{Expr, Func};
use sea_orm::{entity::prelude::*, ActiveValue::Set, QueryOrder, QuerySelect, TryIntoModel};

/// Ceiling on one purge scan. Every row costs a network round trip, so an unbounded
/// result set after an outage of the job would make a single tick run far past its own
/// poll interval. Oldest first, and the next tick takes the remainder.
pub const PURGE_SCAN_LIMIT: u64 = 500;

/// Everything needed to record one stored image. Bundled so `create` stays at two
/// arguments and reads as a single statement at the call site.
pub struct NewCoachingSessionImage {
    pub coaching_session_id: Id,
    pub uploaded_by_id: Id,
    pub storage_key: String,
    pub mime_type: String,
    pub byte_size: i64,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

/// Records the metadata row for an image whose bytes are already in object storage.
///
/// # Errors
///
/// Returns `EntityApiErrorKind::SystemError` when the insert fails, including the
/// unique-violation on `storage_key`.
pub async fn create(
    db: &impl ConnectionTrait,
    params: NewCoachingSessionImage,
) -> Result<Model, Error> {
    let now = chrono::Utc::now();

    let active = ActiveModel {
        coaching_session_id: Set(params.coaching_session_id),
        uploaded_by_id: Set(params.uploaded_by_id),
        storage_key: Set(params.storage_key),
        mime_type: Set(params.mime_type),
        byte_size: Set(params.byte_size),
        width: Set(params.width),
        height: Set(params.height),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active.save(db).await?.try_into_model()?)
}

/// Looks up one image's metadata by id.
///
/// # Errors
///
/// Returns `EntityApiErrorKind::RecordNotFound` when no image has that id.
pub async fn find_by_id(db: &impl ConnectionTrait, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or(Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Reads back the single row an update returned, or reports it missing.
fn only_updated(rows: Vec<Model>) -> Result<Model, Error> {
    rows.into_iter().next().ok_or(Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Marks the row deleted, leaving an existing timestamp untouched.
///
/// Both participants observe the same removal and both may call this, so the timestamp
/// is chosen by a `COALESCE` in the update itself rather than a read-then-branch: a
/// second call cannot extend the grace window, and two concurrent calls cannot race.
///
/// # Errors
///
/// Returns `EntityApiErrorKind::RecordNotFound` when no image has that id, and
/// `EntityApiErrorKind::SystemError` when the update fails.
pub async fn soft_delete(db: &impl ConnectionTrait, id: Id) -> Result<Model, Error> {
    let now: DateTimeWithTimeZone = chrono::Utc::now().into();

    Entity::update_many()
        .col_expr(
            Column::DeletedAt,
            Func::coalesce([Expr::col(Column::DeletedAt).into(), Expr::value(now)]).into(),
        )
        .col_expr(Column::UpdatedAt, Expr::value(now))
        .filter(Column::Id.eq(id))
        .exec_with_returning(db)
        .await
        .map_err(Error::from)
        .and_then(only_updated)
}

/// Clears the deletion mark, which is how an undo in the editor resurrects an image.
///
/// # Errors
///
/// Returns `EntityApiErrorKind::RecordNotFound` when no image has that id, and
/// `EntityApiErrorKind::SystemError` when the update fails.
pub async fn restore(db: &impl ConnectionTrait, id: Id) -> Result<Model, Error> {
    let now: DateTimeWithTimeZone = chrono::Utc::now().into();

    Entity::update_many()
        .col_expr(
            Column::DeletedAt,
            Expr::value(Option::<DateTimeWithTimeZone>::None),
        )
        .col_expr(Column::UpdatedAt, Expr::value(now))
        .filter(Column::Id.eq(id))
        .exec_with_returning(db)
        .await
        .map_err(Error::from)
        .and_then(only_updated)
}

/// Rows marked deleted strictly before `cutoff`, oldest first, capped at
/// `PURGE_SCAN_LIMIT`. Never returns a row whose `deleted_at` is null.
///
/// The `IS NOT NULL` predicate is redundant against SQL's three-valued logic — a null
/// never compares less than anything — and is stated anyway because it is the safety
/// guard a reader needs to see, and the one a test can pin.
///
/// # Errors
///
/// Returns `EntityApiErrorKind::SystemError` when the query fails.
pub async fn find_purgeable(
    db: &impl ConnectionTrait,
    cutoff: DateTimeWithTimeZone,
) -> Result<Vec<Model>, Error> {
    Entity::find()
        .filter(Column::DeletedAt.is_not_null())
        .filter(Column::DeletedAt.lt(cutoff))
        .order_by_asc(Column::DeletedAt)
        .limit(PURGE_SCAN_LIMIT)
        .all(db)
        .await
        .map_err(Into::into)
}

/// Locks one image row for the purge, but only while it is still due.
///
/// A restore clears `deleted_at` on this same row, so taking the lock with the predicate
/// re-checked serializes the two: a restore that committed first leaves nothing to claim,
/// and one that arrives later waits for the purge to commit and then finds no row. Hold
/// the returned lock (the caller's transaction) until the row is deleted.
///
/// # Errors
///
/// Returns `EntityApiErrorKind::SystemError` when the query fails.
pub async fn claim_purgeable(
    db: &impl ConnectionTrait,
    id: Id,
    cutoff: DateTimeWithTimeZone,
) -> Result<Option<Model>, Error> {
    Entity::find_by_id(id)
        .filter(Column::DeletedAt.is_not_null())
        .filter(Column::DeletedAt.lt(cutoff))
        .lock_exclusive()
        .one(db)
        .await
        .map_err(Into::into)
}

/// Every image belonging to any of `coaching_session_ids`, deleted or not.
///
/// Read before a session is deleted: the session FK cascades, so this is the last moment
/// the storage keys exist anywhere. An empty slice never reaches the database.
///
/// # Errors
///
/// Returns `EntityApiErrorKind::SystemError` when the query fails.
pub async fn find_by_coaching_session_ids(
    db: &impl ConnectionTrait,
    coaching_session_ids: &[Id],
) -> Result<Vec<Model>, Error> {
    if coaching_session_ids.is_empty() {
        return Ok(Vec::new());
    }

    Entity::find()
        .filter(Column::CoachingSessionId.is_in(coaching_session_ids.iter().copied()))
        .all(db)
        .await
        .map_err(Into::into)
}

/// Destroys the metadata row. The purge job calls this only after the object is gone.
///
/// # Errors
///
/// Returns `EntityApiErrorKind::SystemError` when the delete fails.
pub async fn delete_by_id(db: &impl ConnectionTrait, id: Id) -> Result<(), Error> {
    Entity::delete_by_id(id).exec(db).await?;
    Ok(())
}

#[cfg(test)]
// We need to gate seaORM's mock feature behind conditional compilation because
// the feature removes the Clone trait implementation from seaORM's DatabaseConnection.
// see https://github.com/SeaQL/sea-orm/issues/830
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, MockDatabase};

    fn image_model() -> Model {
        let now = chrono::Utc::now();

        Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            uploaded_by_id: Id::new_v4(),
            storage_key: "coaching-sessions/abc/images/def.png".to_owned(),
            mime_type: "image/png".to_owned(),
            byte_size: 1234,
            width: Some(640),
            height: Some(480),
            deleted_at: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    #[tokio::test]
    async fn create_returns_a_new_coaching_session_image_model() -> Result<(), Error> {
        let expected = image_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![expected.clone()]])
            .into_connection();

        let image = create(
            &db,
            NewCoachingSessionImage {
                coaching_session_id: expected.coaching_session_id,
                uploaded_by_id: expected.uploaded_by_id,
                storage_key: expected.storage_key.clone(),
                mime_type: expected.mime_type.clone(),
                byte_size: expected.byte_size,
                width: expected.width,
                height: expected.height,
            },
        )
        .await?;

        assert_eq!(image.storage_key, expected.storage_key);
        assert_eq!(image.mime_type, "image/png");
        assert_eq!(image.byte_size, 1234);

        Ok(())
    }

    #[tokio::test]
    async fn find_by_id_returns_the_matching_model() -> Result<(), Error> {
        let expected = image_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![expected.clone()]])
            .into_connection();

        assert_eq!(find_by_id(&db, expected.id).await?.id, expected.id);

        Ok(())
    }

    /// Returns the model as it looked before the call, so an assertion on the value that
    /// comes back is an assertion about what the statement did to it.
    fn deleted_image_model(deleted_at: DateTimeWithTimeZone) -> Model {
        Model {
            deleted_at: Some(deleted_at),
            ..image_model()
        }
    }

    /// The statements a mock connection recorded, debug-formatted. Debug rather than
    /// Display because that is the only rendering `Transaction` offers, and it carries
    /// the bound values alongside the SQL. Quotes come out escaped.
    fn statements_of(db: sea_orm::DatabaseConnection) -> String {
        format!("{:?}", db.into_transaction_log())
    }

    #[tokio::test]
    async fn soft_delete_marks_a_live_row() -> Result<(), Error> {
        let marked = deleted_image_model(chrono::Utc::now().into());

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![marked.clone()]])
            .into_connection();

        assert!(soft_delete(&db, marked.id).await?.deleted_at.is_some());

        Ok(())
    }

    /// Both participants see the same removal and both may call this. A second call must
    /// not push the purge further out, so the timestamp is chosen by `COALESCE` inside the
    /// update rather than by reading the row and branching.
    #[tokio::test]
    async fn soft_delete_preserves_an_existing_timestamp() -> Result<(), Error> {
        let first_marked: DateTimeWithTimeZone =
            (chrono::Utc::now() - chrono::Duration::days(3)).into();
        let already_deleted = deleted_image_model(first_marked);

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![already_deleted.clone()]])
            .into_connection();

        let image = soft_delete(&db, already_deleted.id).await?;

        assert_eq!(
            image.deleted_at,
            Some(first_marked),
            "a repeat removal must not extend the grace window"
        );

        let sql = statements_of(db).to_uppercase();
        assert!(
            sql.contains("COALESCE"),
            "the null check belongs in the update, not in a read-then-branch: {sql}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn soft_delete_returns_record_not_found_when_absent() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<Model>::new()])
            .into_connection();

        let Err(err) = soft_delete(&db, Id::new_v4()).await else {
            panic!("a missing image must not be markable");
        };

        assert_eq!(err.error_kind, EntityApiErrorKind::RecordNotFound);
    }

    /// Undo in the editor resurrects the node with the same id; this is what makes it
    /// resolve again.
    #[tokio::test]
    async fn restore_clears_deleted_at() -> Result<(), Error> {
        let restored = image_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![restored.clone()]])
            .into_connection();

        assert_eq!(restore(&db, restored.id).await?.deleted_at, None);

        let sql = statements_of(db).to_uppercase();
        assert!(
            sql.contains("UPDATE") && sql.contains("DELETED_AT"),
            "restore must write deleted_at: {sql}"
        );

        Ok(())
    }

    /// The headline safety test. A null never compares less than a cutoff, so dropping the
    /// predicate would still behave — until a later rewrite of the comparison. Asserting on
    /// the generated statement is what keeps the guard visible and load-bearing.
    #[tokio::test]
    async fn find_purgeable_states_deleted_at_is_not_null() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<Model>::new()])
            .into_connection();

        find_purgeable(&db, chrono::Utc::now().into()).await?;

        let sql = statements_of(db);
        assert!(
            sql.contains(r#"\"deleted_at\" IS NOT NULL"#),
            "the purge must refuse rows that were never removed: {sql}"
        );

        Ok(())
    }

    /// The cutoff is the grace period. Without it the job would destroy an image the
    /// moment it was removed, which is the one thing deferring destruction exists to stop.
    #[tokio::test]
    async fn find_purgeable_filters_on_the_cutoff() -> Result<(), Error> {
        let cutoff: DateTimeWithTimeZone = (chrono::Utc::now() - chrono::Duration::days(7)).into();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<Model>::new()])
            .into_connection();

        find_purgeable(&db, cutoff).await?;

        let sql = statements_of(db);
        assert!(
            sql.contains(r#"\"deleted_at\" < "#),
            "the scan must be bounded by the cutoff: {sql}"
        );
        assert!(
            sql.contains(&format!("{cutoff:?}")),
            "the cutoff the caller passed must be the one bound into the statement: {sql}"
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_purgeable_returns_the_rows_it_found() -> Result<(), Error> {
        let expected =
            deleted_image_model((chrono::Utc::now() - chrono::Duration::days(30)).into());

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![expected.clone()]])
            .into_connection();

        let purgeable = find_purgeable(&db, chrono::Utc::now().into()).await?;

        assert_eq!(purgeable.len(), 1);
        assert_eq!(purgeable[0].id, expected.id);

        Ok(())
    }

    /// An outage of the purge job leaves a backlog, and every row costs a network round
    /// trip. Without the cap one tick would run far past its own poll interval.
    #[tokio::test]
    async fn find_purgeable_caps_and_orders_the_scan() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<Model>::new()])
            .into_connection();

        find_purgeable(&db, chrono::Utc::now().into()).await?;

        let sql = statements_of(db);
        // The cap binds as a parameter, so the clause and the value are asserted apart.
        assert!(sql.contains("LIMIT $"), "the scan must be bounded: {sql}");
        assert!(
            sql.contains(&format!("BigUnsigned(Some({PURGE_SCAN_LIMIT}))")),
            "the bound cap must be the one the module declares: {sql}"
        );
        assert!(
            sql.contains(r#"\"deleted_at\" ASC"#),
            "oldest removals must be purged first: {sql}"
        );

        Ok(())
    }

    /// The claim is what stops a purge destroying an image restored after the scan. Without
    /// the lock a restore could commit mid-purge; without the predicate the claim would lock
    /// a row that is no longer due.
    #[tokio::test]
    async fn claim_purgeable_locks_the_row_and_rechecks_that_it_is_due() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<Model>::new()])
            .into_connection();

        let claimed = claim_purgeable(&db, Id::new_v4(), chrono::Utc::now().into()).await?;
        assert!(claimed.is_none(), "a restored row is not claimed");

        let sql = statements_of(db);
        assert!(sql.contains("FOR UPDATE"), "the row must be locked: {sql}");
        assert!(
            sql.contains(r#"\"deleted_at\" IS NOT NULL"#) && sql.contains(r#"\"deleted_at\" < "#),
            "the claim must re-check that the row is still due: {sql}"
        );

        Ok(())
    }

    /// Called from the session-delete path, which may have no sessions to clean up. An
    /// unguarded `IN ()` would be a statement the database has to answer for nothing.
    #[tokio::test]
    async fn find_by_coaching_session_ids_does_not_query_for_an_empty_slice() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<Model>::new()])
            .into_connection();

        assert!(find_by_coaching_session_ids(&db, &[]).await?.is_empty());
        assert_eq!(
            statements_of(db),
            "[]",
            "an empty slice must not reach the database"
        );

        Ok(())
    }

    #[tokio::test]
    async fn find_by_coaching_session_ids_returns_the_session_images() -> Result<(), Error> {
        let expected = image_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![expected.clone()]])
            .into_connection();

        let images = find_by_coaching_session_ids(&db, &[expected.coaching_session_id]).await?;

        assert_eq!(images.len(), 1);
        assert_eq!(images[0].storage_key, expected.storage_key);

        Ok(())
    }

    #[tokio::test]
    async fn find_by_id_returns_record_not_found_when_absent() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<Model>::new()])
            .into_connection();

        let Err(err) = find_by_id(&db, Id::new_v4()).await else {
            panic!("a missing image must not resolve");
        };

        assert_eq!(err.error_kind, EntityApiErrorKind::RecordNotFound);
    }
}
