use super::*;
use entity::Id;
use sea_orm::{DatabaseBackend, MockDatabase, Transaction};

#[tokio::test]
async fn find_by_user_with_includes_filters_by_organization_when_supplied() -> Result<(), Error> {
    let user_id = Id::new_v4();
    let organization_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
        .into_connection();

    let _ = find_by_user_with_includes(
        &db,
        user_id,
        SessionQueryOptions {
            organization_id: Some(organization_id),
            ..Default::default()
        },
        IncludeOptions::default(),
    )
    .await?;

    assert_eq!(
        db.into_transaction_log(),
        [Transaction::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."coaching_session_series_id", "coaching_sessions"."ical_sequence", "coaching_sessions"."ical_recurrence_id", "coaching_sessions"."collab_document_name", "coaching_sessions"."date", "coaching_sessions"."duration_minutes", "coaching_sessions"."title", "coaching_sessions"."meeting_url", CAST("coaching_sessions"."provider" AS "text"), "coaching_sessions"."created_at", "coaching_sessions"."updated_at", "coaching_sessions"."hydrated_at", "coaching_sessions"."notice_given_at" FROM "refactor_platform"."coaching_sessions" INNER JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE ("coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2) AND "coaching_relationships"."organization_id" = $3"#,
            [user_id.into(), user_id.into(), organization_id.into()]
        )]
    );

    Ok(())
}

/// Omitting the organization must emit no organization predicate at all, so
/// callers that never send one keep today's behavior.
#[tokio::test]
async fn find_by_user_with_includes_omits_organization_predicate_when_absent() -> Result<(), Error>
{
    let user_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
        .into_connection();

    let _ = find_by_user_with_includes(
        &db,
        user_id,
        SessionQueryOptions::default(),
        IncludeOptions::default(),
    )
    .await?;

    let log = db.into_transaction_log();
    assert_eq!(
        log,
        [Transaction::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT "coaching_sessions"."id", "coaching_sessions"."coaching_relationship_id", "coaching_sessions"."coaching_session_series_id", "coaching_sessions"."ical_sequence", "coaching_sessions"."ical_recurrence_id", "coaching_sessions"."collab_document_name", "coaching_sessions"."date", "coaching_sessions"."duration_minutes", "coaching_sessions"."title", "coaching_sessions"."meeting_url", CAST("coaching_sessions"."provider" AS "text"), "coaching_sessions"."created_at", "coaching_sessions"."updated_at", "coaching_sessions"."hydrated_at", "coaching_sessions"."notice_given_at" FROM "refactor_platform"."coaching_sessions" INNER JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE "coaching_relationships"."coach_id" = $1 OR "coaching_relationships"."coachee_id" = $2"#,
            [user_id.into(), user_id.into()]
        )]
    );
    assert!(
        !format!("{log:?}").contains("organization_id"),
        "no organization predicate when unscoped"
    );

    Ok(())
}

/// Asserted separately from the list query: a shared helper would not prove
/// both call sites apply the filter.
#[tokio::test]
async fn find_counts_by_month_for_user_filters_by_organization_when_supplied() -> Result<(), Error>
{
    let user_id = Id::new_v4();
    let organization_id = Id::new_v4();
    let from_date = chrono::NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
    let to_date = chrono::NaiveDate::from_ymd_opt(2026, 5, 31).unwrap();
    let to_exclusive = chrono::NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
        .into_connection();

    let _ = find_counts_by_month_for_user(
        &db,
        user_id,
        from_date,
        to_date,
        "America/Los_Angeles",
        None,
        Some(organization_id),
    )
    .await?;

    assert_eq!(
        db.into_transaction_log(),
        [Transaction::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT to_char(date_trunc('month', "coaching_sessions"."date" AT TIME ZONE $1::text), 'YYYY-MM') AS "month", COUNT(*)::bigint AS "count" FROM "refactor_platform"."coaching_sessions" INNER JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE "coaching_sessions"."date" >= $2 AND "coaching_sessions"."date" < $3 AND ("coaching_relationships"."coach_id" = $4 OR "coaching_relationships"."coachee_id" = $5) AND "coaching_relationships"."organization_id" = $6 GROUP BY "month" ORDER BY "month" ASC"#,
            [
                "America/Los_Angeles".into(),
                from_date.into(),
                to_exclusive.into(),
                user_id.into(),
                user_id.into(),
                organization_id.into()
            ]
        )]
    );

    Ok(())
}

#[tokio::test]
async fn find_counts_by_month_for_user_omits_organization_predicate_when_absent(
) -> Result<(), Error> {
    let user_id = Id::new_v4();
    let from_date = chrono::NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
    let to_date = chrono::NaiveDate::from_ymd_opt(2026, 5, 31).unwrap();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
        .into_connection();

    let _ = find_counts_by_month_for_user(
        &db,
        user_id,
        from_date,
        to_date,
        "America/Los_Angeles",
        None,
        None,
    )
    .await?;

    let log = db.into_transaction_log();
    assert_eq!(
        log,
        [Transaction::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"SELECT to_char(date_trunc('month', "coaching_sessions"."date" AT TIME ZONE $1::text), 'YYYY-MM') AS "month", COUNT(*)::bigint AS "count" FROM "refactor_platform"."coaching_sessions" INNER JOIN "refactor_platform"."coaching_relationships" ON "coaching_sessions"."coaching_relationship_id" = "coaching_relationships"."id" WHERE "coaching_sessions"."date" >= $2 AND "coaching_sessions"."date" < $3 AND ("coaching_relationships"."coach_id" = $4 OR "coaching_relationships"."coachee_id" = $5) GROUP BY "month" ORDER BY "month" ASC"#,
            [
                "America/Los_Angeles".into(),
                from_date.into(),
                to_date.succ_opt().unwrap().into(),
                user_id.into(),
                user_id.into()
            ]
        )]
    );
    assert!(
        !format!("{log:?}").contains("organization_id"),
        "no organization predicate when unscoped"
    );

    Ok(())
}
