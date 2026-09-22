//! Destroys note images whose removal has outlived its grace period.
//!
//! Removing an image from a coaching note only marks `deleted_at`; the bytes and the row
//! stay put so an undo re-inserts the node with the same id and it still resolves. This
//! sweep is what eventually makes the removal real, once undo is no longer plausible.
//!
//! Each tick re-derives what is due from `deleted_at` alone, so a restore in between
//! simply removes the row from the next tick's result set. Nothing has to be cancelled.
//!
//! See [`crate::jobs`] for why this is a sweep rather than an enqueued job.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use log::*;
use service::config::Config;

use crate::error::Error;
use crate::gateway::object_storage::{self, ObjectStore};
use crate::jobs::{Context, FirstRun, Job, Outcome};

pub struct Purge {
    store: Arc<dyn ObjectStore>,
    /// How long a removal survives before it is made real.
    grace: chrono::Duration,
    interval: Duration,
}

impl Purge {
    /// Build the purge from config, or `None` when object storage is unconfigured.
    ///
    /// With nowhere to delete from there is nothing this job could accomplish, so it is
    /// never scheduled rather than waking to log the same failure every hour.
    pub fn from_config(config: &Config) -> Option<Self> {
        let store = object_storage::from_config(config)?;

        let grace = match chrono::Duration::from_std(config.coaching_session_image_grace_period()) {
            Ok(grace) => grace,
            Err(e) => {
                warn!("[coaching-session-image-purge] COACHING_SESSION_IMAGE_GRACE_PERIOD_HOURS is out of range, purging disabled: {e:?}");
                return None;
            }
        };

        Some(Self {
            store,
            grace,
            interval: config.coaching_session_image_purge_poll_interval(),
        })
    }
}

#[async_trait]
impl Job for Purge {
    fn name(&self) -> &'static str {
        "coaching-session-image-purge"
    }

    fn interval(&self) -> Duration {
        self.interval
    }

    /// Nothing here is urgent — a removal waits days either way — so the first tick stays
    /// out of the way while the process is still starting up.
    fn first_run(&self) -> FirstRun {
        FirstRun::AfterInterval
    }

    async fn run(&self, ctx: &Context) -> Result<Outcome, Error> {
        let cutoff = Utc::now() - self.grace;
        let due =
            entity_api::coaching_session_image::find_purgeable(&*ctx.db, cutoff.into()).await?;

        if due.is_empty() {
            return Ok(Outcome::IDLE);
        }

        let mut purged = 0;
        for image in &due {
            // Object first, row second. Deleting the row first would leave the object
            // with nothing pointing at it, and no later tick could ever find it again.
            if let Err(e) = self.store.delete(&image.storage_key).await {
                warn!(
                    "[coaching-session-image-purge] could not delete object {} for image {}; \
                     the row stays and the next tick retries: {e:?}",
                    image.storage_key, image.id
                );
                continue;
            }

            if let Err(e) =
                entity_api::coaching_session_image::delete_by_id(&*ctx.db, image.id).await
            {
                // The object is already gone, so the retry is a no-op against storage and
                // the row is removed then.
                warn!(
                    "[coaching-session-image-purge] deleted object {} but could not delete \
                     image row {}: {e:?}",
                    image.storage_key, image.id
                );
                continue;
            }

            purged += 1;
        }

        let found = due.len() as u64;
        Ok(if purged == found {
            Outcome::processed(purged)
        } else {
            Outcome::partial(purged, found)
        })
    }
}

#[cfg(all(test, feature = "mock"))]
mod tests {
    use std::sync::Mutex;

    use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};

    use super::*;
    use crate::coaching_session_images::Model;
    use crate::error::{DomainErrorKind, ExternalErrorKind};
    use crate::gateway::object_storage::StoredObject;
    use crate::Id;

    /// Records every call in order, so a test can assert what the job did and when.
    /// Nothing here touches the network or the filesystem.
    #[derive(Default)]
    struct RecordingStore {
        calls: Mutex<Vec<String>>,
        /// Keys whose delete is made to fail, to drive the storage-failure paths.
        fail_keys: Vec<String>,
    }

    impl RecordingStore {
        fn failing_for(keys: &[&str]) -> Self {
            Self {
                fail_keys: keys.iter().map(|key| (*key).to_owned()).collect(),
                ..Default::default()
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls
                .lock()
                .map(|calls| calls.clone())
                .unwrap_or_default()
        }
    }

    #[async_trait]
    impl ObjectStore for RecordingStore {
        async fn put(&self, _key: &str, _bytes: Vec<u8>, _content_type: &str) -> Result<(), Error> {
            unimplemented!("the purge job never writes")
        }

        fn presigned_get(&self, _key: &str, _ttl: Duration) -> Result<Option<String>, Error> {
            unimplemented!("the purge job never reads")
        }

        async fn get(&self, _key: &str) -> Result<StoredObject, Error> {
            unimplemented!("the purge job never reads")
        }

        async fn delete(&self, key: &str) -> Result<(), Error> {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push(format!("delete:{key}"));
            }

            if self.fail_keys.iter().any(|failing| failing == key) {
                return Err(Error {
                    source: None,
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                });
            }

            Ok(())
        }
    }

    fn deleted_image(key: &str) -> Model {
        let now = Utc::now();

        Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            uploaded_by_id: Id::new_v4(),
            storage_key: key.to_owned(),
            mime_type: "image/png".to_owned(),
            byte_size: 1234,
            width: Some(640),
            height: Some(480),
            deleted_at: Some((now - chrono::Duration::days(30)).into()),
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    /// Builds a context whose mock database returns `due` from the purge scan and, when
    /// `row_delete_succeeds`, accepts one row delete after it.
    fn context(due: Vec<Model>, row_delete_succeeds: bool) -> Context {
        let mut db =
            MockDatabase::new(DatabaseBackend::Postgres).append_query_results(vec![due.clone()]);

        if row_delete_succeeds {
            db = db.append_exec_results(vec![
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                };
                due.len()
            ]);
        }

        Context {
            db: Arc::new(db.into_connection()),
            config: Config::from_args(["test"]),
        }
    }

    fn config(args: &[&str]) -> Config {
        let mut argv = vec!["test"];
        argv.extend_from_slice(args);
        Config::from_args(argv)
    }

    /// Nowhere to delete from means nothing to do, so the job must not be scheduled at
    /// all rather than waking every hour to log the same configuration failure.
    #[test]
    fn from_config_is_none_when_object_storage_is_unconfigured() {
        assert!(
            Purge::from_config(&config(&["--object-store-backend=spaces"])).is_none(),
            "spaces without credentials yields no store, so no purge job"
        );
    }

    #[test]
    fn from_config_reads_the_grace_period_and_poll_interval() {
        let purge = Purge::from_config(&config(&[
            "--object-store-backend=local",
            "--coaching-session-image-grace-period-hours=48",
            "--coaching-session-image-purge-poll-minutes=5",
        ]))
        .expect("the local backend is always configured");

        assert_eq!(purge.grace, chrono::Duration::hours(48));
        assert_eq!(purge.interval, Duration::from_secs(5 * 60));
    }

    #[tokio::test]
    async fn a_tick_with_nothing_due_is_idle() {
        let store = Arc::new(RecordingStore::default());
        let job = Purge {
            store: Arc::clone(&store) as Arc<dyn ObjectStore>,
            grace: chrono::Duration::hours(168),
            interval: Duration::from_secs(3600),
        };

        let outcome = job
            .run(&context(Vec::new(), false))
            .await
            .expect("an empty scan is not a failure");

        assert_eq!(outcome.attempted, 0);
        assert!(
            store.calls().is_empty(),
            "nothing due means nothing deleted"
        );
    }

    /// The object must go before the row, proved by a tick where one of two objects
    /// refuses to be deleted. In the right order the failing row survives and only the
    /// other one is deleted from the database; in the wrong order both rows would already
    /// be gone by the time storage was asked, and the surviving row's id would be absent
    /// from neither statement. Deleting the row first would strand the bytes with nothing
    /// pointing at them.
    #[tokio::test]
    async fn deletes_the_object_before_the_row() {
        let stubborn_key = "coaching-sessions/abc/images/stubborn.png";
        let store = Arc::new(RecordingStore::failing_for(&[stubborn_key]));
        let stubborn = deleted_image(stubborn_key);
        let removable = deleted_image("coaching-sessions/abc/images/removable.png");
        let ctx = context(vec![stubborn.clone(), removable.clone()], true);
        let job = Purge {
            store: Arc::clone(&store) as Arc<dyn ObjectStore>,
            grace: chrono::Duration::hours(168),
            interval: Duration::from_secs(3600),
        };

        let outcome = job.run(&ctx).await.expect("a partial tick still succeeds");

        assert_eq!(
            store.calls(),
            vec![
                format!("delete:{stubborn_key}"),
                "delete:coaching-sessions/abc/images/removable.png".to_string(),
            ],
            "storage is asked about both images, in the order the scan returned them"
        );
        assert_eq!(outcome.processed, 1);
        assert_eq!(outcome.attempted, 2);

        let Ok(db) = Arc::try_unwrap(ctx.db) else {
            panic!("the job must not retain a handle to the database");
        };
        let deletes = format!("{:?}", db.into_transaction_log());
        assert!(
            deletes.contains(&removable.id.to_string()),
            "the row whose object went must be deleted: {deletes}"
        );
        assert!(
            !deletes.contains(&stubborn.id.to_string()),
            "the row whose object stayed must not be deleted: {deletes}"
        );
    }

    /// A failed object delete must leave the row alone, so the next tick retries. Deleting
    /// the row anyway would strand the bytes with nothing pointing at them.
    #[tokio::test]
    async fn a_failed_object_delete_leaves_the_row_and_reports_partial() {
        let store = Arc::new(RecordingStore::failing_for(&[
            "coaching-sessions/abc/images/def.png",
        ]));
        let image = deleted_image("coaching-sessions/abc/images/def.png");
        let ctx = context(vec![image.clone()], false);
        let job = Purge {
            store: Arc::clone(&store) as Arc<dyn ObjectStore>,
            grace: chrono::Duration::hours(168),
            interval: Duration::from_secs(3600),
        };

        let outcome = job
            .run(&ctx)
            .await
            .expect("a failed object delete is survivable");

        assert_eq!(outcome.processed, 0);
        assert_eq!(outcome.attempted, 1, "a tick that found work is not idle");
        assert_eq!(
            store.calls(),
            vec!["delete:coaching-sessions/abc/images/def.png".to_string()],
            "the object delete was attempted"
        );

        let Ok(db) = Arc::try_unwrap(ctx.db) else {
            panic!("the job must not retain a handle to the database");
        };
        assert!(
            !format!("{:?}", db.into_transaction_log())
                .to_uppercase()
                .contains("DELETE FROM"),
            "the row must survive an object delete that failed"
        );
    }
}
