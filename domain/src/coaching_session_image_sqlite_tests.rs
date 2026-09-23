//! Session-scoped image lookups against a real SQL engine. Deleting a session destroys
//! the objects behind whatever keys this returns, so a wrong row here is a lost image.

use std::collections::BTreeSet;

use chrono::Utc;
use entity::coaching_session_images::ActiveModel;
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};

use super::*;
use crate::sqlite_test_support::{database, within_time_limit};

/// Records an image in `coaching_session_id`, removed when `removed` is set.
async fn image(db: &DatabaseConnection, coaching_session_id: Id, removed: bool) -> Model {
    let id = Id::new_v4();
    let now = Utc::now();

    ActiveModel {
        id: Set(id),
        coaching_session_id: Set(coaching_session_id),
        uploaded_by_id: Set(Id::new_v4()),
        storage_key: Set(format!(
            "coaching-sessions/{coaching_session_id}/images/{id}.png"
        )),
        mime_type: Set("image/png".to_string()),
        byte_size: Set(0),
        width: Set(Some(1)),
        height: Set(Some(1)),
        deleted_at: Set(removed.then(|| now.into())),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the row is recorded")
}

/// Every image of the named sessions, removed or not, and none from any other session.
#[tokio::test]
async fn storage_keys_come_only_from_the_named_sessions() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let (deleted_a, deleted_b, kept) = (Id::new_v4(), Id::new_v4(), Id::new_v4());

        let mut expected = BTreeSet::new();
        for session in [deleted_a, deleted_b] {
            for removed in [false, true] {
                expected.insert(image(&db, session, removed).await.storage_key);
            }
        }
        let untouched = image(&db, kept, false).await;

        let keys: BTreeSet<String> = storage_keys_for_sessions(&db, &[deleted_a, deleted_b])
            .await?
            .into_iter()
            .collect();

        assert_eq!(
            keys, expected,
            "exactly the named sessions' images, live and removed"
        );
        assert!(
            !keys.contains(&untouched.storage_key),
            "another session's image would be destroyed"
        );
        assert!(storage_keys_for_sessions(&db, &[]).await?.is_empty());

        Ok::<(), Error>(())
    })
    .await
}
