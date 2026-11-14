//! Recall bot detail parsing, pinned to the shape the production API returns.

use super::*;

/// Captured from production on 2026-10-08 (ids changed).
const BOT_DETAIL: &str = r#"{
    "id": "b68c3681-0000-0000-0000-000000000000",
    "meeting_url": { "meeting_id": "rdm-nffa-jeu", "platform": "google_meet" },
    "metadata": { "coaching_session_id": "742a8efb-0000-0000-0000-000000000000" },
    "status_changes": [
        { "code": "joining_call", "message": null, "created_at": "2026-10-06T18:00:58Z" },
        { "code": "done", "message": null, "created_at": "2026-10-06T18:38:47Z" }
    ]
}"#;

#[test]
fn a_meeting_url_object_yields_the_meeting_id() {
    let detail: BotDetailResponse =
        serde_json::from_str(BOT_DETAIL).expect("the production shape parses");

    let info = bot_detail_to_info(detail);

    assert_eq!(info.id, "b68c3681-0000-0000-0000-000000000000");
    assert_eq!(info.meeting_id.as_deref(), Some("rdm-nffa-jeu"));
}

#[test]
fn a_missing_meeting_url_yields_no_meeting_id() {
    let detail: BotDetailResponse =
        serde_json::from_str(r#"{ "id": "bot-1", "meeting_url": null, "status_changes": [] }"#)
            .expect("parses");

    assert_eq!(bot_detail_to_info(detail).meeting_id, None);
}

#[test]
fn a_bot_list_with_object_meeting_urls_parses() {
    let list: BotListResponse =
        serde_json::from_str(&format!(r#"{{ "results": [{BOT_DETAIL}] }}"#))
            .expect("the list shape parses");

    assert_eq!(list.results.len(), 1);
}
