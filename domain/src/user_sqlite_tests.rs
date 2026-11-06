//! Checks what a working-hours update leaves in the `users` row.

use chrono::NaiveTime;
use sea_orm::{EntityTrait, Value};

use super::{parse_working_hours, update};
use crate::error::{DomainErrorKind, Error};
use crate::test_utils::sqlite::{self, seed_user, within_time_limit};
use crate::{users, IntoUpdateMap, UpdateMap};

fn time(hour: u32, minute: u32) -> NaiveTime {
    NaiveTime::from_hms_opt(hour, minute, 0).expect("valid test time")
}

/// A user update built from raw working-hours strings, as a request sends them.
struct Changes {
    first_name: Option<&'static str>,
    working_hours_start: Option<&'static str>,
    working_hours_end: Option<&'static str>,
}

impl IntoUpdateMap for Changes {
    fn into_update_map(self) -> UpdateMap {
        let mut map = UpdateMap::new();
        let patch = parse_working_hours(self.working_hours_start, self.working_hours_end)
            .expect("test values are on the step");
        if let Some(first_name) = self.first_name {
            map.insert(
                "first_name".to_string(),
                Some(Value::String(Some(first_name.to_string()))),
            );
        }
        if let Some(start) = patch.start {
            map.insert(
                "working_hours_start".to_string(),
                Some(Value::ChronoTime(Some(start))),
            );
        }
        if let Some(end) = patch.end {
            map.insert(
                "working_hours_end".to_string(),
                Some(Value::ChronoTime(Some(end))),
            );
        }
        map
    }
}

#[tokio::test]
async fn updating_only_the_end_keeps_the_stored_start() {
    within_time_limit(async {
        let db = sqlite::database().await;
        let user_id = seed_user(&db).await;

        let changes = Changes {
            first_name: None,
            working_hours_start: None,
            working_hours_end: Some("18:00"),
        };
        update(&db, user_id, changes).await?;

        let row = users::Entity::find_by_id(user_id)
            .one(&db)
            .await
            .expect("the user is read")
            .expect("the user exists");
        assert_eq!(row.working_hours_start, time(8, 0));
        assert_eq!(row.working_hours_end, time(18, 0));
        Ok::<(), Error>(())
    })
    .await
    .expect("the update succeeds");
}

#[tokio::test]
async fn a_start_after_the_stored_end_writes_nothing() {
    within_time_limit(async {
        let db = sqlite::database().await;
        let user_id = seed_user(&db).await;

        let changes = Changes {
            first_name: Some("Changed"),
            working_hours_start: Some("18:00"),
            working_hours_end: None,
        };
        let err = update(&db, user_id, changes)
            .await
            .expect_err("18:00 is after the stored 17:30 end");
        assert_eq!(
            err.error_kind,
            DomainErrorKind::Validation(
                "working_hours_start must be before working_hours_end".to_string()
            )
        );

        let row = users::Entity::find_by_id(user_id)
            .one(&db)
            .await
            .expect("the user is read")
            .expect("the user exists");
        assert_eq!(row.first_name, "Test");
        assert_eq!(row.working_hours_start, time(8, 0));
        assert_eq!(row.working_hours_end, time(17, 30));
    })
    .await;
}
