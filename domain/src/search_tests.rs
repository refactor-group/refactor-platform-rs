use super::*;
use crate::user_roles;
use chrono::Utc;

fn user_with_roles(roles: Vec<user_roles::Model>) -> users::Model {
    users::Model {
        id: Id::new_v4(),
        email: "search-scope@test.dev".to_string(),
        first_name: "Scope".to_string(),
        last_name: "Test".to_string(),
        display_name: None,
        password: None,
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
        roles,
        invite_status: None,
        created_at: Utc::now().into(),
        updated_at: Utc::now().into(),
    }
}

fn role(role: Role, organization_id: Option<Id>) -> user_roles::Model {
    user_roles::Model {
        id: Id::new_v4(),
        role,
        organization_id,
        user_id: Id::new_v4(),
        created_at: Utc::now().into(),
        updated_at: Utc::now().into(),
    }
}

#[test]
fn scope_for_a_regular_member_has_membership_but_no_admin_orgs() {
    let org = Id::new_v4();
    let user = user_with_roles(vec![role(Role::User, Some(org))]);
    let scope = scope_for(&user);
    assert_eq!(scope.user_id, user.id);
    assert!(!scope.is_super_admin);
    assert!(scope.admin_org_ids.is_empty());
    assert_eq!(scope.member_org_ids, vec![org]);
}

#[test]
fn scope_for_an_org_admin_is_both_admin_and_member_of_that_org() {
    let admin_org = Id::new_v4();
    let member_org = Id::new_v4();
    let user = user_with_roles(vec![
        role(Role::Admin, Some(admin_org)),
        role(Role::User, Some(member_org)),
    ]);
    let scope = scope_for(&user);
    assert!(!scope.is_super_admin);
    assert_eq!(scope.admin_org_ids, vec![admin_org]);
    assert_eq!(scope.member_org_ids, vec![admin_org, member_org]);
}

#[cfg(feature = "mock")]
mod mock_tests {
    use super::*;
    use crate::coaching_relationships;
    use sea_orm::{DatabaseBackend, MockDatabase};

    fn super_admin_scope() -> Scope {
        Scope {
            user_id: Id::new_v4(),
            is_super_admin: true,
            admin_org_ids: vec![],
            member_org_ids: vec![],
        }
    }

    fn relationship(id: Id, organization_id: Id) -> coaching_relationships::Model {
        coaching_relationships::Model {
            id,
            organization_id,
            coach_id: Id::new_v4(),
            coachee_id: Id::new_v4(),
            slug: "coach-coachee".to_string(),
            created_at: Utc::now().into(),
            updated_at: Utc::now().into(),
        }
    }

    #[tokio::test]
    async fn super_admin_relationship_filter_alone_needs_no_query() {
        let db = MockDatabase::new(DatabaseBackend::Postgres).into_connection();
        let rel_id = Id::new_v4();

        let resolved =
            resolve_visible_relationship_ids(&db, &super_admin_scope(), None, Some(rel_id))
                .await
                .expect("resolves");

        assert_eq!(resolved, Some(vec![rel_id]));
        assert!(db.into_transaction_log().is_empty());
    }

    #[tokio::test]
    async fn super_admin_org_and_relationship_filters_intersect() {
        let rel_id = Id::new_v4();
        let org_id = Id::new_v4();
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![relationship(rel_id, org_id)]])
            .into_connection();

        let resolved =
            resolve_visible_relationship_ids(&db, &super_admin_scope(), Some(org_id), Some(rel_id))
                .await
                .expect("resolves");

        assert_eq!(resolved, Some(vec![rel_id]));
        let log = db.into_transaction_log();
        let sql = format!("{:?}", log[0]);
        assert!(sql.contains(r#"\"coaching_relationships\".\"id\" ="#));
        assert!(sql.contains(r#"\"coaching_relationships\".\"organization_id\" ="#));
    }

    #[tokio::test]
    async fn super_admin_relationship_outside_the_requested_org_resolves_to_nothing() {
        // The DB finds no relationship matching BOTH filters.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<coaching_relationships::Model>::new()])
            .into_connection();

        let resolved = resolve_visible_relationship_ids(
            &db,
            &super_admin_scope(),
            Some(Id::new_v4()),
            Some(Id::new_v4()),
        )
        .await
        .expect("resolves");

        assert_eq!(resolved, Some(vec![]));
    }

    #[tokio::test]
    async fn a_member_with_no_visible_relationships_never_reaches_the_searchers() {
        // One canned result: the visibility resolution finds nothing. Were any
        // searcher to run anyway, its query would find no canned result and fail.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<coaching_relationships::Model>::new()])
            .into_connection();
        let scope = Scope {
            user_id: Id::new_v4(),
            is_super_admin: false,
            admin_org_ids: vec![],
            member_org_ids: vec![Id::new_v4()],
        };
        let spec = Spec {
            q_raw: "quarterly".to_string(),
            query: "quarterly".to_string(),
            types: vec![
                HitType::CoachingSession,
                HitType::Goal,
                HitType::Action,
                HitType::Agreement,
                HitType::Topic,
            ],
            coaching_relationship_id: None,
            filters: Filters::default(),
            limit: 10,
            cursor: None,
        };

        let results = search(&db, &scope, spec).await.expect("searches");

        assert!(results.hits.is_empty());
        assert!(results.next_cursor.is_none());
        assert_eq!(
            db.into_transaction_log().len(),
            1,
            "only the visibility resolution may hit the database"
        );
    }
}

#[test]
fn scope_for_super_admin_requires_the_global_role_row() {
    let org = Id::new_v4();
    // An org-scoped SuperAdmin row is not the global role.
    let scoped = user_with_roles(vec![role(Role::SuperAdmin, Some(org))]);
    assert!(!scope_for(&scoped).is_super_admin);

    let global = user_with_roles(vec![role(Role::SuperAdmin, None)]);
    assert!(scope_for(&global).is_super_admin);
}

// --- merge_and_paginate: the cross-searcher response sort and page cut --------

mod merge_tests {
    use super::*;
    use crate::status::Status;
    use crate::topic_status::Status as TopicStatus;

    fn core(score: f32, id: Id) -> Core {
        Core {
            id,
            score,
            title: "t".to_string(),
            snippet: None,
            created_at: Utc::now().into(),
            updated_at: None,
            organization_id: Id::new_v4(),
        }
    }

    fn goal(score: f32, id: Id) -> Hit {
        Hit::Goal(GoalHit {
            core: core(score, id),
            coaching_relationship_id: Id::new_v4(),
            status: Status::NotStarted,
            created_in_session_id: None,
        })
    }

    fn action(score: f32, id: Id) -> Hit {
        Hit::Action(ActionHit {
            core: core(score, id),
            coaching_session_id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            goal_id: None,
            status: Status::InProgress,
            due_by: None,
            session_date: Utc::now().naive_utc(),
            session_display_title: String::new(),
        })
    }

    fn topic(score: f32, id: Id) -> Hit {
        Hit::Topic(TopicHit {
            core: core(score, id),
            coaching_session_id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            status: TopicStatus::Open,
            priority: None,
        })
    }

    fn ids(hits: &[Hit]) -> Vec<Id> {
        hits.iter().map(|h| h.core().id).collect()
    }

    #[test]
    fn merge_orders_by_score_desc_then_type_then_id() {
        let low = Id::from_u128(1);
        let high = Id::from_u128(2);
        let top = Id::from_u128(3);
        // Equal scores: Action sorts before Goal (type ASC), then id ASC
        // within a type. A higher score beats both regardless of type.
        let (hits, next) = merge_and_paginate(
            vec![
                goal(0.5, high),
                topic(0.9, top),
                goal(0.5, low),
                action(0.5, high),
            ],
            25,
        );
        assert_eq!(ids(&hits), vec![top, high, low, high]);
        assert_eq!(
            hits.iter().map(Hit::hit_type).collect::<Vec<_>>(),
            vec![
                HitType::Topic,
                HitType::Action,
                HitType::Goal,
                HitType::Goal
            ]
        );
        assert!(next.is_none());
    }

    #[test]
    fn a_full_page_without_overflow_has_no_next_cursor() {
        let (hits, next) =
            merge_and_paginate(vec![goal(0.3, Id::new_v4()), goal(0.2, Id::new_v4())], 2);
        assert_eq!(hits.len(), 2);
        assert!(next.is_none());
    }

    #[test]
    fn the_overflow_row_yields_a_cursor_keyed_on_the_last_returned_hit() {
        let last_returned = Id::from_u128(7);
        let (hits, next) = merge_and_paginate(
            vec![
                goal(0.9, Id::new_v4()),
                action(0.5, last_returned),
                topic(0.1, Id::new_v4()),
            ],
            2,
        );
        assert_eq!(hits.len(), 2);
        let cursor = Cursor::decode(&next.expect("cursor")).expect("decodes");
        assert_eq!(
            cursor,
            Cursor {
                score: 0.5,
                hit_type: HitType::Action,
                id: last_returned,
            }
        );
    }

    #[test]
    fn no_hits_is_an_empty_page_without_a_cursor() {
        let (hits, next) = merge_and_paginate(vec![], 25);
        assert!(hits.is_empty());
        assert!(next.is_none());
    }
}
