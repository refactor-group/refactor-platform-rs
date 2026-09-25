//! The purge against a real SQL engine: in-memory SQLite, a real on-disk object store, and
//! the real job. The mock tests beside this file cannot see which rows a query selects or
//! what an update writes; these can. Row locking is the one thing SQLite cannot show, so
//! the `FOR UPDATE` on the claim stays pinned by the mock tests.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use entity::coaching_session_images::{ActiveModel, Entity, Model};
use entity_api::coaching_session_image::{find_by_id, restore, soft_delete, PURGE_SCAN_LIMIT};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};
use service::config::Config;
use tempfile::TempDir;

use super::*;
use crate::error::{DomainErrorKind, ExternalErrorKind};
use crate::gateway::object_storage::{LocalObjectStore, StoredObject};
use crate::test_utils::sqlite::{database, seed_coaching_session, within_time_limit};
use crate::Id;

const GRACE: chrono::Duration = chrono::Duration::days(7);

/// The real on-disk store, able to refuse deleting one chosen object.
struct TestStore {
    inner: LocalObjectStore,
    refuse_delete_of: Mutex<Option<String>>,
}

impl TestStore {
    fn refuse_delete_of(&self, key: Option<&str>) {
        if let Ok(mut refused) = self.refuse_delete_of.lock() {
            *refused = key.map(str::to_owned);
        }
    }

    fn refuses(&self, key: &str) -> bool {
        self.refuse_delete_of
            .lock()
            .is_ok_and(|refused| refused.as_deref() == Some(key))
    }
}

#[async_trait]
impl ObjectStore for TestStore {
    async fn put(&self, key: &str, source: &Path, content_type: &str) -> Result<(), Error> {
        self.inner.put(key, source, content_type).await
    }

    fn presigned_get(&self, key: &str, ttl: Duration) -> Result<Option<String>, Error> {
        self.inner.presigned_get(key, ttl)
    }

    async fn get(&self, key: &str) -> Result<StoredObject, Error> {
        self.inner.get(key).await
    }

    async fn delete(&self, key: &str) -> Result<(), Error> {
        if self.refuses(key) {
            return Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            });
        }
        self.inner.delete(key).await
    }
}

/// The job, the store it deletes from, and the directory that store writes to.
struct World {
    db: Arc<DatabaseConnection>,
    /// The session every image belongs to, and its uploader.
    session: (Id, Id),
    store: Arc<TestStore>,
    job: Purge,
    _root: TempDir,
}

impl World {
    async fn new() -> Self {
        let root = TempDir::new().expect("the store root is created");
        let store = Arc::new(TestStore {
            inner: LocalObjectStore::new(root.path()),
            refuse_delete_of: Mutex::new(None),
        });

        let db = database().await;
        let session = seed_coaching_session(&db).await;

        Self {
            db: Arc::new(db),
            session,
            job: Purge {
                store: Arc::clone(&store) as Arc<dyn ObjectStore>,
                grace: GRACE,
                interval: Duration::from_secs(3600),
            },
            store,
            _root: root,
        }
    }

    async fn tick(&self) -> Outcome {
        self.job
            .run(&Context {
                db: Arc::clone(&self.db),
                config: Config::from_args(["test"]),
            })
            .await
            .expect("the tick succeeds")
    }

    /// Stores an object and records a row for it, removed `removed_ago` in the past, or
    /// live when that is `None`.
    async fn image(&self, removed_ago: Option<chrono::Duration>) -> Model {
        let id = Id::new_v4();
        let key = format!("coaching-sessions/abc/images/{id}.png");

        let source = tempfile::NamedTempFile::new().expect("a source file is created");
        self.store
            .put(&key, source.path(), "image/png")
            .await
            .expect("the object is stored");

        let now = Utc::now();
        ActiveModel {
            id: Set(id),
            coaching_session_id: Set(self.session.0),
            uploaded_by_id: Set(self.session.1),
            storage_key: Set(key),
            mime_type: Set("image/png".to_string()),
            byte_size: Set(0),
            width: Set(Some(1)),
            height: Set(Some(1)),
            deleted_at: Set(removed_ago.map(|ago| (now - ago).into())),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        }
        .insert(&*self.db)
        .await
        .expect("the row is recorded")
    }

    /// Ids of the rows still in the table.
    async fn surviving_rows(&self) -> BTreeSet<Id> {
        Entity::find()
            .all(&*self.db)
            .await
            .expect("the table is readable")
            .into_iter()
            .map(|image| image.id)
            .collect()
    }

    async fn object_exists(&self, image: &Model) -> bool {
        self.store.get(&image.storage_key).await.is_ok()
    }

    /// Row and object both still present.
    async fn is_whole(&self, image: &Model) -> bool {
        self.surviving_rows().await.contains(&image.id) && self.object_exists(image).await
    }

    /// Row and object both gone.
    async fn is_destroyed(&self, image: &Model) -> bool {
        !self.surviving_rows().await.contains(&image.id) && !self.object_exists(image).await
    }
}

/// The central safety property: exactly the images past their grace period are destroyed,
/// row and object both, and every other image is left whole. The boundary pair sits one
/// minute either side of the cutoff.
#[tokio::test]
async fn destroys_exactly_the_images_past_their_grace_period() {
    within_time_limit(async {
        let world = World::new().await;
        let minute = chrono::Duration::minutes(1);

        let live = world.image(None).await;
        let just_removed = world.image(Some(minute)).await;
        let inside_grace = world.image(Some(GRACE - minute)).await;
        let past_grace = world.image(Some(GRACE + minute)).await;
        let long_gone = world.image(Some(chrono::Duration::days(30))).await;

        let outcome = world.tick().await;

        assert_eq!(
            world.surviving_rows().await,
            BTreeSet::from([live.id, just_removed.id, inside_grace.id]),
            "only rows past the grace period may be deleted"
        );
        for kept in [&live, &just_removed, &inside_grace] {
            assert!(
                world.is_whole(kept).await,
                "an image inside its grace period was damaged: {}",
                kept.storage_key
            );
        }
        for destroyed in [&past_grace, &long_gone] {
            assert!(
                world.is_destroyed(destroyed).await,
                "a purged image left its row or object behind: {}",
                destroyed.storage_key
            );
        }
        assert_eq!((outcome.processed, outcome.attempted), (2, 2));

        // With nothing left due, a second tick must touch nothing.
        let second = world.tick().await;
        assert_eq!((second.processed, second.attempted), (0, 0));
        assert_eq!(
            world.surviving_rows().await,
            BTreeSet::from([live.id, just_removed.id, inside_grace.id])
        );
    })
    .await;
}

/// A removal through the real write path starts the grace period now, not in the past.
#[tokio::test]
async fn a_fresh_removal_survives_the_next_tick() {
    within_time_limit(async {
        let world = World::new().await;
        let image = world.image(None).await;

        soft_delete(&*world.db, image.id)
            .await
            .expect("the removal succeeds");
        world.tick().await;

        assert!(world.is_whole(&image).await);
    })
    .await;
}

/// An undo must survive the purge, and the proof has to come from what is in the table,
/// not from the model `restore` hands back.
#[tokio::test]
async fn a_restored_image_is_never_purged() {
    within_time_limit(async {
        let world = World::new().await;
        let image = world.image(Some(chrono::Duration::days(30))).await;

        restore(&*world.db, image.id)
            .await
            .expect("the restore succeeds");

        let stored = find_by_id(&*world.db, image.id)
            .await
            .expect("the row is still there");
        assert_eq!(stored.deleted_at, None, "a restore must clear the mark");

        world.tick().await;

        assert!(world.is_whole(&image).await);
    })
    .await;
}

/// A restore landing after the scan chose a row, before the purge claimed it. The claim's
/// own check of the row is all that stands between that undo and a destroyed image.
#[tokio::test]
async fn an_image_restored_after_the_scan_is_left_whole() {
    within_time_limit(async {
        let world = World::new().await;
        let image = world.image(Some(chrono::Duration::days(30))).await;
        let cutoff = (Utc::now() - GRACE).into();

        let due = find_purgeable(&*world.db, cutoff)
            .await
            .expect("the scan succeeds");
        assert_eq!(
            due.iter().map(|due| due.id).collect::<Vec<_>>(),
            vec![image.id],
            "the scan must have chosen the image, or the race is not being exercised"
        );

        restore(&*world.db, image.id)
            .await
            .expect("the restore succeeds");

        let purged = world
            .job
            .purge_one(&world.db, image.id, cutoff)
            .await
            .expect("the purge succeeds");

        assert!(matches!(purged, Purged::Restored));
        assert!(world.is_whole(&image).await);
    })
    .await;
}

/// Storage refusing a delete must leave the image whole, row included, so a later tick can
/// finish it. Losing the row here would strand the object with nothing pointing at it.
#[tokio::test]
async fn a_failed_object_delete_keeps_the_image_until_a_later_tick_succeeds() {
    within_time_limit(async {
        let world = World::new().await;
        let stubborn = world.image(Some(chrono::Duration::days(30))).await;
        let removable = world.image(Some(chrono::Duration::days(30))).await;
        world.store.refuse_delete_of(Some(&stubborn.storage_key));

        let outcome = world.tick().await;

        assert!(
            world.is_whole(&stubborn).await,
            "a refused delete must change nothing"
        );
        assert!(world.is_destroyed(&removable).await);
        assert_eq!((outcome.processed, outcome.attempted), (1, 2));

        world.store.refuse_delete_of(None);
        world.tick().await;

        assert!(
            world.is_destroyed(&stubborn).await,
            "the retry must finish the job"
        );
    })
    .await;
}

/// One tick takes at most the cap, oldest first, and the next tick takes the rest.
#[tokio::test]
async fn the_scan_cap_purges_oldest_first_and_leaves_the_rest_for_the_next_tick() {
    within_time_limit(async {
        let world = World::new().await;
        let cap = i64::try_from(PURGE_SCAN_LIMIT).expect("the cap fits");

        // One more than the cap, each removed a minute apart. `newest` is the least overdue.
        let mut images = Vec::new();
        for minutes in 0..=cap {
            let removed_ago = chrono::Duration::days(30) + chrono::Duration::minutes(minutes);
            images.push(world.image(Some(removed_ago)).await);
        }
        let newest = images[0].clone();

        let first = world.tick().await;

        assert_eq!(first.processed, PURGE_SCAN_LIMIT);
        assert_eq!(
            world.surviving_rows().await,
            BTreeSet::from([newest.id]),
            "the oldest removals go first, and only the cap's worth"
        );

        let second = world.tick().await;

        assert_eq!(second.processed, 1);
        assert!(world.is_destroyed(&newest).await);
    })
    .await;
}
