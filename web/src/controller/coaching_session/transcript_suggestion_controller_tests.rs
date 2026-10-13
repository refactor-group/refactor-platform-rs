use axum::http::StatusCode;
use axum::response::IntoResponse;
use domain::transcript_suggestion::Status;

use super::listed_status;

#[test]
fn an_absent_or_pending_status_lists_pending_suggestions() {
    assert_eq!(listed_status(None).ok(), Some(Status::Pending));
    assert_eq!(listed_status(Some("pending")).ok(), Some(Status::Pending));
}

#[test]
fn a_dismissed_status_lists_dismissed_suggestions() {
    assert_eq!(
        listed_status(Some("dismissed")).ok(),
        Some(Status::Dismissed)
    );
}

#[test]
fn any_other_status_is_a_validation_error() {
    for param in ["accepted", "Dismissed", ""] {
        let status = listed_status(Some(param))
            .err()
            .map(|err| err.into_response().status());
        assert_eq!(status, Some(StatusCode::UNPROCESSABLE_ENTITY), "{param:?}");
    }
}
