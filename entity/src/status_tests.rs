use super::Status;

#[test]
fn is_completed_true_only_for_completed_and_wont_do() {
    assert!(Status::Completed.is_completed());
    assert!(Status::WontDo.is_completed());
    assert!(!Status::NotStarted.is_completed());
    assert!(!Status::InProgress.is_completed());
    assert!(!Status::OnHold.is_completed());
}
