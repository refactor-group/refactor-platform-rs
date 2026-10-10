//! The coach/coachee permission rule for reviewing suggestions.

use super::*;

fn people() -> People {
    People {
        coach_id: Id::new_v4(),
        coachee_id: Id::new_v4(),
    }
}

#[test]
fn the_coach_may_act_on_every_assignment() {
    let p = people();
    for stored in [
        vec![p.coach_id],
        vec![p.coachee_id],
        vec![p.coach_id, p.coachee_id],
        vec![],
    ] {
        assert!(may_act(Actor::Coach, p, &stored, None), "{stored:?}");
    }
}

#[test]
fn the_coachee_may_not_act_on_a_coach_only_suggestion() {
    let p = people();
    assert!(!may_act(Actor::Coachee, p, &[p.coach_id], None));
}

#[test]
fn the_coachee_may_act_on_coachee_only_shared_and_unassigned_suggestions() {
    let p = people();
    for stored in [vec![p.coachee_id], vec![p.coach_id, p.coachee_id], vec![]] {
        assert!(may_act(Actor::Coachee, p, &stored, None), "{stored:?}");
    }
}

#[test]
fn the_coachee_may_not_add_or_remove_the_coach() {
    let p = people();
    assert!(!may_act(Actor::Coachee, p, &[], Some(&[p.coach_id])));
    assert!(!may_act(
        Actor::Coachee,
        p,
        &[p.coach_id, p.coachee_id],
        Some(&[p.coachee_id])
    ));
}

#[test]
fn the_coachee_may_edit_without_changing_the_coach() {
    let p = people();
    assert!(may_act(
        Actor::Coachee,
        p,
        &[p.coachee_id],
        Some(&[p.coachee_id])
    ));
    assert!(may_act(Actor::Coachee, p, &[], Some(&[p.coachee_id])));
}

#[test]
fn actor_of_names_the_coach_the_coachee_or_nobody() {
    let p = people();
    assert_eq!(actor_of(p.coach_id, p), Some(Actor::Coach));
    assert_eq!(actor_of(p.coachee_id, p), Some(Actor::Coachee));
    assert_eq!(actor_of(Id::new_v4(), p), None);
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
