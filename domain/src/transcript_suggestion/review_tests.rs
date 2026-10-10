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
