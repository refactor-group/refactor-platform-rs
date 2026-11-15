//! The participant permission rule for reviewing suggestions.

use super::*;
use crate::coaching_relationship_kind::Kind;

fn relationship(kind: Kind) -> coaching_relationships::Model {
    let epoch = DateTime::<Utc>::UNIX_EPOCH.fixed_offset();
    coaching_relationships::Model {
        id: Id::new_v4(),
        organization_id: Id::new_v4(),
        coach_id: Id::new_v4(),
        coachee_id: Id::new_v4(),
        kind,
        slug: String::new(),
        created_at: epoch,
        updated_at: epoch,
    }
}

fn reviewer(relationship: &coaching_relationships::Model, user_id: Id) -> Reviewer {
    reviewer_of(user_id, relationship).expect("a participant")
}

#[test]
fn the_coach_may_act_on_every_assignment() {
    let r = relationship(Kind::Coaching);
    let coach = reviewer(&r, r.coach_id);
    for stored in [
        vec![r.coach_id],
        vec![r.coachee_id],
        vec![r.coach_id, r.coachee_id],
        vec![],
    ] {
        assert!(may_act(coach, &stored, None), "{stored:?}");
    }
}

#[test]
fn the_coachee_may_not_act_on_a_coach_only_suggestion() {
    let r = relationship(Kind::Coaching);
    assert!(!may_act(reviewer(&r, r.coachee_id), &[r.coach_id], None));
}

#[test]
fn the_coachee_may_act_on_coachee_only_shared_and_unassigned_suggestions() {
    let r = relationship(Kind::Coaching);
    let coachee = reviewer(&r, r.coachee_id);
    for stored in [vec![r.coachee_id], vec![r.coach_id, r.coachee_id], vec![]] {
        assert!(may_act(coachee, &stored, None), "{stored:?}");
    }
}

#[test]
fn the_coachee_may_not_add_or_remove_the_coach() {
    let r = relationship(Kind::Coaching);
    let coachee = reviewer(&r, r.coachee_id);
    assert!(!may_act(coachee, &[], Some(&[r.coach_id])));
    assert!(!may_act(
        coachee,
        &[r.coach_id, r.coachee_id],
        Some(&[r.coachee_id])
    ));
}

#[test]
fn the_coachee_may_edit_without_changing_the_coach() {
    let r = relationship(Kind::Coaching);
    let coachee = reviewer(&r, r.coachee_id);
    assert!(may_act(coachee, &[r.coachee_id], Some(&[r.coachee_id])));
    assert!(may_act(coachee, &[], Some(&[r.coachee_id])));
}

#[test]
fn a_colleague_in_the_coachee_seat_may_act_on_and_move_the_other_colleague() {
    let r = relationship(Kind::Colleague);
    let colleague = reviewer(&r, r.coachee_id);
    assert!(may_act(colleague, &[r.coach_id], None));
    assert!(may_act(colleague, &[], Some(&[r.coach_id])));
    assert!(may_act(
        colleague,
        &[r.coach_id, r.coachee_id],
        Some(&[r.coachee_id])
    ));
}

#[test]
fn reviewer_of_names_the_other_participant_or_nobody() {
    let r = relationship(Kind::Coaching);
    assert_eq!(reviewer(&r, r.coach_id).other_id, r.coachee_id);
    assert_eq!(reviewer(&r, r.coachee_id).other_id, r.coach_id);
    assert_eq!(reviewer_of(Id::new_v4(), &r), None);
}

#[test]
fn edits_replace_exactly_the_body_due_date_and_assignees() {
    let epoch = DateTime::<Utc>::UNIX_EPOCH.fixed_offset();
    let stored = ActionProposal {
        action: ActionWithAssignees {
            action: actions::Model {
                id: Id::new_v4(),
                coaching_session_id: Id::new_v4(),
                goal_id: Some(Id::new_v4()),
                user_id: Id::new_v4(),
                body: Some("Old body".to_string()),
                due_by: Some(epoch),
                status: ActionStatus::InProgress,
                status_changed_at: epoch,
                created_at: epoch,
                updated_at: epoch,
            },
            assignee_ids: vec![Id::new_v4()],
        },
        due_phrase: Some("by Friday".to_string()),
    };
    let edits = ActionEdits {
        body: "New body".to_string(),
        due_by: None,
        assignee_ids: vec![Id::new_v4(), Id::new_v4()],
        goal_id: stored.action.action.goal_id,
    };

    let result = edits.onto(&stored);

    assert_eq!(result.action.action.body.as_deref(), Some("New body"));
    assert_eq!(result.action.action.due_by, None);
    assert_eq!(result.action.assignee_ids, edits.assignee_ids);
    let unchanged = |p: &ActionProposal| actions::Model {
        body: None,
        due_by: None,
        ..p.action.action.clone()
    };
    assert_eq!(unchanged(&result), unchanged(&stored));
    assert_eq!(result.due_phrase, stored.due_phrase);
}

#[test]
fn edits_without_a_goal_deserialize_as_none_and_with_one_as_some() {
    let none: ActionEdits = serde_json::from_value(serde_json::json!({
        "body": "x", "due_by": null, "assignee_ids": [] }))
    .expect("parses");
    assert_eq!(none.goal_id, None);
    let id = Id::new_v4();
    let some: ActionEdits = serde_json::from_value(serde_json::json!({
        "body": "x", "due_by": null, "assignee_ids": [], "goal_id": id }))
    .expect("parses");
    assert_eq!(some.goal_id, Some(id));
}
