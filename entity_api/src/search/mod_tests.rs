use super::*;

// --- compile_query: the final-token prefix contract -------------------------

#[test]
fn plain_final_word_is_prefix_matched() {
    assert_eq!(
        compile_query("quarterly quart"),
        CompiledQuery::HeadAndPrefix("quarterly".to_string(), "quart:*".to_string())
    );
}

#[test]
fn single_token_becomes_a_bare_prefix_query() {
    assert_eq!(
        compile_query("quart"),
        CompiledQuery::Prefix("quart:*".to_string())
    );
}

#[test]
fn trailing_whitespace_opts_out_of_prefixing() {
    assert_eq!(
        compile_query("quart "),
        CompiledQuery::Plain("quart".to_string())
    );
}

#[test]
fn negated_final_token_is_never_prefixed() {
    assert_eq!(
        compile_query("review -draft"),
        CompiledQuery::Plain("review -draft".to_string())
    );
}

#[test]
fn any_or_operator_disables_prefixing() {
    // Adjacent: prefixing would strand `or` at the end of the head, where it
    // is dropped. Earlier: the AND-ed prefix would regroup `a | (b & c)` as
    // `(a | b) & c:*`, dropping documents that match only `a`. Trailing:
    // `alpha & or:*` would drop documents that match only `alpha`.
    for raw in [
        "dogs or cat",
        "dogs OR cat",
        "alpha or beta gam",
        "alpha OR beta gam",
        "alpha or",
        "alpha OR",
    ] {
        assert_eq!(compile_query(raw), CompiledQuery::Plain(raw.to_string()));
    }
}

#[test]
fn a_lone_or_is_still_prefix_matched() {
    // No left operand, so it is a partial word ("organization"), not an operator.
    assert_eq!(
        compile_query("or"),
        CompiledQuery::Prefix("or:*".to_string())
    );
}

#[test]
fn or_inside_a_word_does_not_disable_prefixing() {
    assert_eq!(
        compile_query("door cat"),
        CompiledQuery::HeadAndPrefix("door".to_string(), "cat:*".to_string())
    );
}

#[test]
fn quoted_final_token_is_never_prefixed() {
    assert_eq!(
        compile_query(r#"prep "quarterly review""#),
        CompiledQuery::Plain(r#"prep "quarterly review""#.to_string())
    );
}

#[test]
fn unbalanced_quote_disables_prefixing() {
    // The last token sits inside a still-open phrase.
    assert_eq!(
        compile_query(r#""quarterly rev"#),
        CompiledQuery::Plain(r#""quarterly rev"#.to_string())
    );
}

#[test]
fn punctuated_final_token_is_never_prefixed() {
    // The parser indexes `check-in` as `check-in` and `check`; stripping the
    // punctuation would yield `checkin:*`, which matches neither.
    for raw in ["review check-in", "review q3!", "coach's"] {
        assert_eq!(compile_query(raw), CompiledQuery::Plain(raw.to_string()));
    }
}

#[test]
fn symbol_only_final_token_falls_back_to_plain() {
    assert_eq!(
        compile_query("review !!"),
        CompiledQuery::Plain("review !!".to_string())
    );
}

/// Renders the match condition for `raw` as Postgres SQL plus its bind values.
fn match_sql(raw: &str) -> (String, Vec<sea_orm::Value>) {
    let statement = entity::goals::Entity::find()
        .select_only()
        .filter(FtsExpressions::over("txt").match_condition(&compile_query(raw)))
        .build(sea_orm::DbBackend::Postgres);
    (
        statement.sql,
        statement.values.map(|v| v.0).unwrap_or_default(),
    )
}

#[test]
fn prefix_branch_keeps_stop_words_through_the_simple_parse() {
    // `english` drops a stop word like `for` to an empty query; the `simple`
    // parse keeps it, so typing "for" on the way to "forecast" still matches.
    let (sql, values) = match_sql("for");
    assert!(
        sql.contains("(to_tsquery('english', $1) || to_tsquery('simple', $2))"),
        "{sql}"
    );
    assert_eq!(values, vec!["for:*".into(), "for:*".into()]);
}

#[test]
fn head_and_prefix_binds_the_lexeme_for_each_parse() {
    // Every placeholder needs its own value: a shared `$n` would be renumbered
    // by SeaQuery and leave a parameter unbound at execution.
    let (sql, values) = match_sql("q3 for");
    assert!(
        sql.contains(
            "(websearch_to_tsquery('english', $1) \
             && (to_tsquery('english', $2) || to_tsquery('simple', $3)))"
        ),
        "{sql}"
    );
    assert_eq!(values, vec!["q3".into(), "for:*".into(), "for:*".into()]);
}

// --- Cursor: bit-exact round trip -------------------------------------------

#[test]
fn cursor_round_trips_bit_exactly() {
    let cursor = Cursor {
        // A score with no short decimal rendering; bit-exactness matters.
        score: f32::from_bits(0x3e99_999a),
        hit_type: HitType::Goal,
        id: Id::new_v4(),
    };
    let decoded = Cursor::decode(&cursor.encode()).expect("round trip decodes");
    assert_eq!(decoded.score.to_bits(), cursor.score.to_bits());
    assert_eq!(decoded.hit_type, cursor.hit_type);
    assert_eq!(decoded.id, cursor.id);
}

#[test]
fn cursor_decode_rejects_garbage() {
    assert_eq!(Cursor::decode("not base64!!"), Err(CursorDecodeError));
    assert_eq!(Cursor::decode(""), Err(CursorDecodeError));
    // Valid base64, wrong length.
    assert_eq!(Cursor::decode("AAAA"), Err(CursorDecodeError));
    // Right length, unknown version byte.
    let mut bytes = vec![9u8];
    bytes.extend_from_slice(&[0u8; 21]);
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    assert_eq!(Cursor::decode(&raw), Err(CursorDecodeError));
}

// --- excerpt_title -----------------------------------------------------------

#[test]
fn excerpt_title_uses_the_body_when_short() {
    assert_eq!(
        excerpt_title(Some("Ship the report"), "Action"),
        "Ship the report"
    );
}

#[test]
fn excerpt_title_truncates_on_a_word_boundary() {
    let body = "sesquipedalian ".repeat(10);
    let title = excerpt_title(Some(&body), "Action");
    assert!(title.chars().count() <= 81, "got: {title}");
    assert!(title.ends_with("sesquipedalian…"), "cut mid-word: {title}");
}

#[test]
fn excerpt_title_falls_back_when_blank() {
    assert_eq!(excerpt_title(None, "Agreement"), "Agreement");
    assert_eq!(excerpt_title(Some("   "), "Agreement"), "Agreement");
}

// --- HitType ordering discipline ---------------------------------------------

#[test]
fn hit_type_order_is_alphabetical_by_wire_name() {
    // The derived Ord is the response sort's "type ASC"; both the merge and the
    // per-searcher cursor fold rely on it matching the wire names' order.
    let mut types = [
        HitType::TranscriptSuggestion,
        HitType::Topic,
        HitType::CoachingSession,
        HitType::Goal,
        HitType::Action,
        HitType::Agreement,
    ];
    types.sort();
    assert_eq!(
        types,
        [
            HitType::Action,
            HitType::Agreement,
            HitType::CoachingSession,
            HitType::Goal,
            HitType::Topic,
            HitType::TranscriptSuggestion,
        ]
    );
}

#[test]
fn all_is_in_merge_order_and_every_type_round_trips_its_cursor_code() {
    let mut sorted = HitType::ALL;
    sorted.sort();
    assert_eq!(sorted, HitType::ALL);
    for t in HitType::ALL {
        assert_eq!(HitType::from_code(t.code()), Some(t));
    }
}

#[test]
fn searchers_follow_all() {
    let types: Vec<HitType> = searchers().iter().map(|s| s.hit_type()).collect();
    assert_eq!(types, HitType::ALL);
}

// --- Searcher SQL contracts (MockDatabase transaction-log pins) ---------------
//
// Real FTS matching needs a live Postgres, but the *shape* of every searcher's
// SQL — the visibility predicate, filter columns, soft-delete guard, and cursor
// keyset — is pinned here so a scoping or filter regression fails in CI.

#[cfg(feature = "mock")]
mod mock_tests {
    use super::*;
    use chrono::Utc;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use std::collections::BTreeMap;

    fn visibility() -> Visibility {
        Visibility {
            user_id: Id::new_v4(),
            is_super_admin: false,
            admin_org_ids: vec![],
            member_org_ids: vec![],
        }
    }

    /// Run one searcher against an empty MockDatabase and return the Debug form
    /// of the single statement it executed (SQL with escaped quotes + values).
    async fn sql_of(searcher: &dyn Searcher, req: &Request<'_>) -> String {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![Vec::<BTreeMap<String, Value>>::new()])
            .into_connection();
        searcher.search(&db, req).await.expect("searcher query");
        let log = db.into_transaction_log();
        assert_eq!(log.len(), 1, "expected exactly one query");
        format!("{:?}", log[0])
    }

    fn request<'a>(
        visibility: &'a Visibility,
        visible: Option<&'a [Id]>,
        filters: &'a Filters,
        cursor: Option<&'a Cursor>,
    ) -> Request<'a> {
        Request {
            q: "quarterly review",
            visibility,
            visible_relationship_ids: visible,
            filters,
            cursor,
            fetch: 41,
        }
    }

    #[tokio::test]
    async fn every_searcher_scopes_by_the_resolved_relationship_set() {
        let visibility = visibility();
        let rel_id = Id::new_v4();
        let visible = [rel_id];
        let filters = Filters::default();
        for searcher in searchers() {
            let sql = sql_of(
                searcher.as_ref(),
                &request(&visibility, Some(&visible), &filters, None),
            )
            .await;
            assert!(
                sql.contains("websearch_to_tsquery"),
                "{:?} lost the FTS match condition: {sql}",
                searcher.hit_type()
            );
            assert!(
                sql.contains(r#"\"coaching_relationship_id\" IN"#),
                "{:?} lost the visibility predicate: {sql}",
                searcher.hit_type()
            );
            assert!(
                sql.contains(&rel_id.to_string()),
                "{:?} did not bind the visible set: {sql}",
                searcher.hit_type()
            );
            assert!(
                sql.contains("LIMIT") && sql.contains("41"),
                "{:?} lost the fetch limit: {sql}",
                searcher.hit_type()
            );
        }
    }

    #[tokio::test]
    async fn every_searcher_is_unrestricted_for_a_super_admin() {
        let visibility = Visibility {
            is_super_admin: true,
            ..visibility()
        };
        let filters = Filters::default();
        for searcher in searchers() {
            let sql = sql_of(
                searcher.as_ref(),
                &request(&visibility, None, &filters, None),
            )
            .await;
            assert!(
                !sql.contains(r#"\"coaching_relationship_id\" IN"#),
                "{:?} applied a visibility predicate to a super admin: {sql}",
                searcher.hit_type()
            );
        }
    }

    #[tokio::test]
    async fn the_topic_searcher_excludes_soft_deleted_topics() {
        let visibility = visibility();
        let filters = Filters::default();
        let sql = sql_of(
            &topic::Searcher,
            &request(&visibility, None, &filters, None),
        )
        .await;
        assert!(
            sql.contains(r#"\"deleted_at\" IS NULL"#),
            "missing soft-delete guard: {sql}"
        );
    }

    #[tokio::test]
    async fn the_suggestion_searcher_matches_only_pending_rows_with_a_literal_predicate() {
        let visibility = visibility();
        let filters = Filters::default();
        let sql = sql_of(
            &transcript_suggestion::Searcher,
            &request(&visibility, None, &filters, None),
        )
        .await;
        // Literal, not bound: a `$n` status can't prove the partial index
        // predicate under a generic plan.
        assert!(
            sql.contains(
                r#"\"transcript_suggestions\".\"status\" = 'pending' AND \"transcript_suggestions\".\"created_entity_id\" IS NULL"#
            ),
            "missing literal pending predicate: {sql}"
        );
        assert!(
            !sql.contains(r#"String(Some("pending"))"#),
            "pending status must not be a bind: {sql}"
        );
        assert!(
            sql.contains(r#"->> 'body'"#),
            "lost the body text expr: {sql}"
        );
    }

    #[tokio::test]
    async fn the_user_filter_means_participant_for_suggestions() {
        let visibility = visibility();
        let participant_rel = Id::new_v4();
        let visible_rel = Id::new_v4();
        let visible = [visible_rel];
        let filters = Filters {
            user_id: Some(Id::new_v4()),
            participant_relationship_ids: Some(vec![participant_rel]),
            ..Filters::default()
        };
        let sql = sql_of(
            &transcript_suggestion::Searcher,
            &request(&visibility, Some(&visible), &filters, None),
        )
        .await;
        assert!(sql.contains(&participant_rel.to_string()), "{sql}");
        assert!(!sql.contains(&visible_rel.to_string()), "{sql}");
        assert!(!sql.contains(r#"\"user_id\""#), "{sql}");
    }

    #[tokio::test]
    async fn the_user_filter_means_participant_for_sessions_and_creator_elsewhere() {
        let visibility = visibility();
        let participant_rel = Id::new_v4();
        let visible_rel = Id::new_v4();
        let visible = [visible_rel];
        let filters = Filters {
            user_id: Some(Id::new_v4()),
            participant_relationship_ids: Some(vec![participant_rel]),
            ..Filters::default()
        };
        let req = request(&visibility, Some(&visible), &filters, None);

        // Sessions scope on the participant-narrowed set and never touch the
        // creator column (sessions have none).
        let sql = sql_of(&coaching_session::Searcher, &req).await;
        assert!(sql.contains(&participant_rel.to_string()), "{sql}");
        assert!(!sql.contains(&visible_rel.to_string()), "{sql}");
        assert!(!sql.contains(r#"\"user_id\""#), "{sql}");

        // Content types keep the visible set and filter by the creator column.
        let sql = sql_of(&action::Searcher, &req).await;
        assert!(sql.contains(&visible_rel.to_string()), "{sql}");
        assert!(!sql.contains(&participant_rel.to_string()), "{sql}");
        assert!(sql.contains(r#"\"user_id\" ="#), "{sql}");
    }

    #[tokio::test]
    async fn content_filters_compile_into_column_predicates() {
        let visibility = visibility();

        let filters = Filters {
            status: Some(Status::InProgress),
            created: TimeRange {
                from: Some(Utc::now().into()),
                to_exclusive: Some(Utc::now().into()),
            },
            updated: TimeRange {
                from: Some(Utc::now().into()),
                to_exclusive: Some(Utc::now().into()),
            },
            ..Filters::default()
        };
        let sql = sql_of(&goal::Searcher, &request(&visibility, None, &filters, None)).await;
        assert!(sql.contains(r#"\"status\" ="#), "{sql}");
        assert!(sql.contains(r#"\"created_at\" >="#), "{sql}");
        assert!(sql.contains(r#"\"created_at\" <"#), "{sql}");
        assert!(sql.contains(r#"\"updated_at\" >="#), "{sql}");
        assert!(sql.contains(r#"\"updated_at\" <"#), "{sql}");

        let filters = Filters {
            coaching_session_id: Some(Id::new_v4()),
            goal_filter: GoalFilter::Unlinked,
            ..Filters::default()
        };
        let sql = sql_of(
            &action::Searcher,
            &request(&visibility, None, &filters, None),
        )
        .await;
        assert!(sql.contains(r#"\"coaching_session_id\" ="#), "{sql}");
        assert!(sql.contains(r#"\"goal_id\" IS NULL"#), "{sql}");

        let filters = Filters {
            topic_status: Some(TopicStatus::Discussed),
            ..Filters::default()
        };
        let sql = sql_of(
            &topic::Searcher,
            &request(&visibility, None, &filters, None),
        )
        .await;
        assert!(sql.contains(r#"\"status\" ="#), "{sql}");
    }

    #[tokio::test]
    async fn the_cursor_keyset_depends_on_the_type_ordering() {
        let visibility = visibility();
        let filters = Filters::default();
        let cursor = |hit_type| Cursor {
            score: 0.5,
            hit_type,
            id: Id::new_v4(),
        };

        // Goal sorts after Action: equal-score goal rows are still due (<=).
        let c = cursor(HitType::Action);
        let sql = sql_of(
            &goal::Searcher,
            &request(&visibility, None, &filters, Some(&c)),
        )
        .await;
        assert!(sql.contains("<="), "{sql}");
        assert!(!sql.contains(r#"\"id\" >"#), "{sql}");

        // Same type: strict-less OR (equal AND id greater).
        let c = cursor(HitType::Goal);
        let sql = sql_of(
            &goal::Searcher,
            &request(&visibility, None, &filters, Some(&c)),
        )
        .await;
        assert!(sql.contains(" OR "), "{sql}");
        assert!(sql.contains(r#"\"id\" >"#), "{sql}");

        // Goal sorts before Topic: equal scores were already served (<).
        let c = cursor(HitType::Topic);
        let sql = sql_of(
            &goal::Searcher,
            &request(&visibility, None, &filters, Some(&c)),
        )
        .await;
        assert!(!sql.contains("<="), "{sql}");
        assert!(!sql.contains(r#"\"id\" >"#), "{sql}");
        assert!(sql.contains("< $"), "{sql}");
    }

    // --- Snippet hydration ----------------------------------------------------

    fn core(id: Id) -> Core {
        Core {
            id,
            score: 0.5,
            title: "t".to_string(),
            snippet: None,
            created_at: Utc::now().into(),
            updated_at: None,
            organization_id: Id::new_v4(),
        }
    }

    fn goal_hit(id: Id) -> Hit {
        Hit::Goal(GoalHit {
            core: core(id),
            coaching_relationship_id: Id::new_v4(),
            status: Status::NotStarted,
            created_in_session_id: None,
        })
    }

    fn action_hit(id: Id) -> Hit {
        Hit::Action(ActionHit {
            core: core(id),
            coaching_session_id: Id::new_v4(),
            coaching_relationship_id: Id::new_v4(),
            goal_id: None,
            status: Status::InProgress,
            due_by: None,
            session_date: Utc::now().naive_utc(),
            session_title: String::new(),
        })
    }

    fn snippet_row(id: Id, snippet: &str) -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("id".to_string(), id.into()),
            ("snippet".to_string(), snippet.into()),
        ])
    }

    #[tokio::test]
    async fn snippet_hydration_batches_one_query_per_type_and_maps_by_id() {
        let action_id = Id::new_v4();
        let goal_a = Id::new_v4();
        let goal_b = Id::new_v4();
        let mut hits = vec![action_hit(action_id), goal_hit(goal_a), goal_hit(goal_b)];

        // Types are hydrated in HitType order: actions first, then goals. The
        // goal batch deliberately omits goal_b — its snippet must stay None.
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![snippet_row(
                action_id,
                "the \u{2}quarterly\u{3} action",
            )]])
            .append_query_results(vec![vec![snippet_row(
                goal_a,
                "the \u{2}quarterly\u{3} goal",
            )]])
            .into_connection();

        hydrate_snippets(&db, "quarterly", &mut hits)
            .await
            .expect("hydrates");

        assert_eq!(
            hits[0].core().snippet.as_deref(),
            Some("the <mark>quarterly</mark> action")
        );
        assert_eq!(
            hits[1].core().snippet.as_deref(),
            Some("the <mark>quarterly</mark> goal")
        );
        assert_eq!(hits[2].core().snippet, None);

        let log = db.into_transaction_log();
        assert_eq!(log.len(), 2, "one batched query per type present");
        let first = format!("{:?}", log[0]);
        assert!(first.contains("ts_headline"), "{first}");
        // Debug-formatting escapes the control-character markers.
        assert!(first.contains(r"StartSel=\u{2}, StopSel=\u{3}"), "{first}");
        // Both goal ids travel in the second (goal) batch.
        let second = format!("{:?}", log[1]);
        assert!(second.contains(&goal_a.to_string()) && second.contains(&goal_b.to_string()));
    }
}
