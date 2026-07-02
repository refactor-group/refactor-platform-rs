use super::batch_load_first_goal_titles;
use entity::{coaching_sessions_goals, goals, Id};
use sea_orm::{DatabaseBackend, MockDatabase};

fn goal_with_title(relationship_id: Id, title: Option<&str>) -> goals::Model {
    let now = chrono::Utc::now().fixed_offset();
    goals::Model {
        id: Id::new_v4(),
        coaching_relationship_id: relationship_id,
        created_in_session_id: None,
        user_id: Id::new_v4(),
        title: title.map(str::to_owned),
        body: None,
        status: entity::status::Status::InProgress,
        status_changed_at: None,
        completed_at: None,
        target_date: None,
        created_at: now,
        updated_at: now,
    }
}

fn link(session_id: Id, goal_id: Id) -> coaching_sessions_goals::Model {
    let now = chrono::Utc::now().fixed_offset();
    coaching_sessions_goals::Model {
        id: Id::new_v4(),
        coaching_session_id: session_id,
        goal_id,
        created_at: now,
        updated_at: now,
    }
}

/// A leading title-less goal must NOT drop the goal tier: the first goal that
/// actually has a title wins. Regression for the next() -> find_map fix.
#[tokio::test]
async fn first_goal_title_skips_leading_untitled_goal() {
    let session_id = Id::new_v4();
    let relationship_id = Id::new_v4();
    let untitled = goal_with_title(relationship_id, None);
    let titled = goal_with_title(relationship_id, Some("Real goal title"));

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![vec![
            (link(session_id, untitled.id), Some(untitled.clone())),
            (link(session_id, titled.id), Some(titled.clone())),
        ]])
        .into_connection();

    let map = batch_load_first_goal_titles(&db, &[session_id])
        .await
        .expect("loader should succeed");

    assert_eq!(
        map.get(&session_id).map(String::as_str),
        Some("Real goal title")
    );
}
