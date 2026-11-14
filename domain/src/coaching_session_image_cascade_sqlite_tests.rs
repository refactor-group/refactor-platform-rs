//! The foreign key cascades that remove image rows when their session or uploader goes.
//! A cascade that reaches too far deletes another session's images.

use std::collections::BTreeSet;

use chrono::Utc;
use entity::coaching_session_images::{ActiveModel, Entity};
use entity::{coaching_sessions, users};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};

use crate::test_utils::sqlite::{database, seed_coaching_session, seed_user, within_time_limit};
use crate::Id;

/// Records a live image in `coaching_session_id` uploaded by `uploaded_by_id`; returns its id.
async fn image(db: &DatabaseConnection, coaching_session_id: Id, uploaded_by_id: Id) -> Id {
    let id = Id::new_v4();
    let now = Utc::now();
    ActiveModel {
        id: Set(id),
        coaching_session_id: Set(coaching_session_id),
        uploaded_by_id: Set(uploaded_by_id),
        storage_key: Set(format!(
            "coaching-sessions/{coaching_session_id}/images/{id}.png"
        )),
        mime_type: Set("image/png".to_string()),
        byte_size: Set(0),
        width: Set(Some(1)),
        height: Set(Some(1)),
        deleted_at: Set(None),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
    }
    .insert(db)
    .await
    .expect("the image is recorded")
    .id
}

/// Every image id left in the table.
async fn remaining(db: &DatabaseConnection) -> BTreeSet<Id> {
    Entity::find()
        .all(db)
        .await
        .expect("the images are read")
        .into_iter()
        .map(|row| row.id)
        .collect()
}

#[tokio::test]
async fn deleting_a_session_removes_only_its_own_images() {
    within_time_limit(async {
        let db = database().await;
        let (a, a_user) = seed_coaching_session(&db).await;
        let (b, b_user) = seed_coaching_session(&db).await;
        let a_images = [image(&db, a, a_user).await, image(&db, a, a_user).await];
        let b_images = BTreeSet::from([image(&db, b, b_user).await, image(&db, b, b_user).await]);

        coaching_sessions::Entity::delete_by_id(a)
            .exec(&db)
            .await
            .expect("the session is deleted");

        let left = remaining(&db).await;
        assert_eq!(left, b_images, "exactly the other session's images remain");
        assert!(a_images.iter().all(|id| !left.contains(id)));
    })
    .await
}

#[tokio::test]
async fn deleting_the_uploader_removes_their_images() {
    within_time_limit(async {
        let db = database().await;
        let (session, owner) = seed_coaching_session(&db).await;
        let uploader = seed_user(&db).await;
        let kept = image(&db, session, owner).await;
        image(&db, session, uploader).await;

        users::Entity::delete_by_id(uploader)
            .exec(&db)
            .await
            .expect("the uploader is deleted");

        assert_eq!(remaining(&db).await, BTreeSet::from([kept]));
    })
    .await
}
