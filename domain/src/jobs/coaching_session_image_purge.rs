//! Destroys note images whose removal has outlived its grace period.
//!
//! Removing an image from a coaching note only marks `deleted_at`; the bytes and the row
//! stay put so an undo re-inserts the node with the same id and it still resolves. This
//! sweep is what eventually makes the removal real, once undo is no longer plausible.
//!
//! Each tick re-derives what is due from `deleted_at` alone, so a restore in between
//! simply removes the row from the next tick's result set. Nothing has to be cancelled.
//! A restore that lands during a tick is handled by claiming each row under a lock before
//! destroying anything; see `Purge::purge_one`.
//!
//! See [`crate::jobs`] for why this is a sweep rather than an enqueued job.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use entity_api::coaching_session_image::{claim_purgeable, delete_by_id, find_purgeable};
use log::*;
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::{DatabaseConnection, TransactionTrait};
use service::config::Config;

use crate::error::Error;
use crate::gateway::object_storage::{self, ObjectStore};
use crate::jobs::{Context, FirstRun, Job, Outcome};
use crate::Id;

/// What happened to one image the scan found due.
enum Purged {
    Destroyed,
    /// Restored after the scan, so there was nothing left to claim.
    Restored,
}

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

    /// Destroys one image, holding its row locked from the claim to the commit.
    ///
    /// The object delete is the irreversible step, so the lock has to cover it. Without it
    /// a restore could clear `deleted_at` between the scan and the delete: the undo would
    /// report success and the image would already be gone. With it, a restore either lands
    /// first and there is nothing to claim, or waits and then finds no row.
    async fn purge_one(
        &self,
        db: &DatabaseConnection,
        id: Id,
        cutoff: DateTimeWithTimeZone,
    ) -> Result<Purged, Error> {
        let txn = db.begin().await.map_err(entity_api::error::Error::from)?;

        let Some(image) = claim_purgeable(&txn, id, cutoff).await? else {
            return Ok(Purged::Restored);
        };

        // Object first, row second. Deleting the row first would leave the object with
        // nothing pointing at it, and no later tick could ever find it again.
        self.store.delete(&image.storage_key).await?;
        delete_by_id(&txn, image.id).await?;

        txn.commit().await.map_err(entity_api::error::Error::from)?;
        Ok(Purged::Destroyed)
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
        let cutoff: DateTimeWithTimeZone = (Utc::now() - self.grace).into();
        let due = find_purgeable(&*ctx.db, cutoff).await?;

        let mut purged = 0;
        let mut attempted = 0;
        for image in &due {
            match self.purge_one(&ctx.db, image.id, cutoff).await {
                Ok(Purged::Destroyed) => {
                    purged += 1;
                    attempted += 1;
                }
                Ok(Purged::Restored) => {
                    debug!(
                        "[coaching-session-image-purge] image {} was restored; skipped",
                        image.id
                    );
                }
                Err(e) => {
                    // The transaction rolled back, so the row stays and the next tick retries.
                    // If the object went before the failure, the retry's delete is a no-op.
                    warn!(
                        "[coaching-session-image-purge] could not purge image {}; the next tick \
                         retries: {e:?}",
                        image.id
                    );
                    attempted += 1;
                }
            }
        }

        Ok(if purged == attempted {
            Outcome::processed(purged)
        } else {
            Outcome::partial(purged, attempted)
        })
    }
}

#[cfg(test)]
#[path = "coaching_session_image_purge_sqlite_tests.rs"]
mod sqlite_tests;

#[cfg(all(test, feature = "mock"))]
mod tests {
    use std::path::Path;
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
        async fn put(&self, _key: &str, _source: &Path, _content_type: &str) -> Result<(), Error> {
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

    /// Builds a context whose mock database returns `due` from the scan, then answers each
    /// row's claim from `claims` in order (`None` for a row restored since the scan), and
    /// accepts `row_deletes` deletes.
    fn context(due: Vec<Model>, claims: Vec<Option<Model>>, row_deletes: usize) -> Context {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![due])
            .append_query_results(
                claims
                    .into_iter()
                    .map(|claim| claim.into_iter().collect::<Vec<_>>()),
            )
            .append_exec_results(vec![
                MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 1,
                };
                row_deletes
            ]);

        Context {
            db: Arc::new(db.into_connection()),
            config: Config::from_args(["test"]),
        }
    }

    /// The `DELETE` statements a tick issued, debug-formatted with their bound values. The
    /// claims also carry each row's id, so assertions about deletion must look only here.
    fn row_deletes(ctx: Context) -> String {
        let Ok(db) = Arc::try_unwrap(ctx.db) else {
            panic!("the job must not retain a handle to the database");
        };

        db.into_transaction_log()
            .iter()
            .flat_map(|txn| txn.statements())
            .filter(|stmt| stmt.sql.starts_with("DELETE"))
            .map(|stmt| format!("{stmt:?}"))
            .collect()
    }

    fn job(store: &Arc<RecordingStore>) -> Purge {
        Purge {
            store: Arc::clone(store) as Arc<dyn ObjectStore>,
            grace: chrono::Duration::hours(168),
            interval: Duration::from_secs(3600),
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

        let outcome = job(&store)
            .run(&context(Vec::new(), Vec::new(), 0))
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
    /// other one is deleted from the database. Deleting the row first would strand the
    /// bytes with nothing pointing at them.
    #[tokio::test]
    async fn deletes_the_object_before_the_row() {
        let stubborn_key = "coaching-sessions/abc/images/stubborn.png";
        let store = Arc::new(RecordingStore::failing_for(&[stubborn_key]));
        let stubborn = deleted_image(stubborn_key);
        let removable = deleted_image("coaching-sessions/abc/images/removable.png");
        let ctx = context(
            vec![stubborn.clone(), removable.clone()],
            vec![Some(stubborn.clone()), Some(removable.clone())],
            1,
        );

        let outcome = job(&store)
            .run(&ctx)
            .await
            .expect("a partial tick still succeeds");

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

        let deletes = row_deletes(ctx);
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
        let ctx = context(vec![image.clone()], vec![Some(image.clone())], 0);

        let outcome = job(&store)
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
        assert!(
            row_deletes(ctx).is_empty(),
            "the row must survive an object delete that failed"
        );
    }

    /// An undo that lands after the scan. The claim finds the row no longer due, so the
    /// image is left whole: the object stays in storage and the row stays in the table.
    /// Destroying either here would lose an image the user was just told they got back.
    #[tokio::test]
    async fn an_image_restored_after_the_scan_is_left_whole() {
        let store = Arc::new(RecordingStore::default());
        let image = deleted_image("coaching-sessions/abc/images/restored.png");
        let ctx = context(vec![image], vec![None], 0);

        let outcome = job(&store)
            .run(&ctx)
            .await
            .expect("a restored row is not a failure");

        assert!(
            store.calls().is_empty(),
            "a restored image's object must not be deleted"
        );
        assert_eq!(
            (outcome.processed, outcome.attempted),
            (0, 0),
            "a restore is not a failed purge"
        );
        assert!(
            row_deletes(ctx).is_empty(),
            "a restored image's row must not be deleted"
        );
    }

    /// The lock is what makes the claim mean anything. The claim must come before the
    /// object delete and inside the same transaction as the row delete, or a restore can
    /// still slip between them.
    #[tokio::test]
    async fn the_claim_and_the_row_delete_share_one_transaction() {
        let store = Arc::new(RecordingStore::default());
        let image = deleted_image("coaching-sessions/abc/images/def.png");
        let ctx = context(vec![image.clone()], vec![Some(image)], 1);

        job(&store).run(&ctx).await.expect("the purge succeeds");

        let Ok(db) = Arc::try_unwrap(ctx.db) else {
            panic!("the job must not retain a handle to the database");
        };
        let log = db.into_transaction_log();
        let txn = log
            .iter()
            .find(|txn| {
                txn.statements()
                    .iter()
                    .any(|stmt| stmt.sql.contains("FOR UPDATE"))
            })
            .unwrap_or_else(|| panic!("no transaction took the row lock: {log:?}"));
        let sql: Vec<&str> = txn
            .statements()
            .iter()
            .map(|stmt| stmt.sql.as_str())
            .collect();

        let claim = sql.iter().position(|q| q.contains("FOR UPDATE"));
        let delete = sql.iter().position(|q| q.starts_with("DELETE"));
        assert!(
            matches!((claim, delete), (Some(claim), Some(delete)) if claim < delete),
            "the lock must be taken before, and in the same transaction as, the delete: {sql:?}"
        );
    }
}
