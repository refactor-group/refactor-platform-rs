use super::error::{EntityApiErrorKind, Error};
use entity::coaching_session_note_images::{ActiveModel, Entity, Model};
use entity::Id;
use sea_orm::{entity::prelude::*, ActiveValue::Set, TryIntoModel};

/// Everything needed to record one stored image. Bundled so `create` stays at two
/// arguments and reads as a single statement at the call site.
pub struct NewNoteImage {
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
pub async fn create(db: &impl ConnectionTrait, params: NewNoteImage) -> Result<Model, Error> {
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
            storage_key: "coaching-sessions/abc/notes/def.png".to_owned(),
            mime_type: "image/png".to_owned(),
            byte_size: 1234,
            width: Some(640),
            height: Some(480),
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    #[tokio::test]
    async fn create_returns_a_new_note_image_model() -> Result<(), Error> {
        let expected = image_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![expected.clone()]])
            .into_connection();

        let image = create(
            &db,
            NewNoteImage {
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
