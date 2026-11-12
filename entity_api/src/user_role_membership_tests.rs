use super::*;
use sea_orm::{DatabaseBackend, MockDatabase, Value};
use std::collections::BTreeMap;

/// The single-column row shape the membership select reads back.
fn member_row(user_id: Id) -> BTreeMap<String, Value> {
    BTreeMap::from([("user_id".to_string(), user_id.into())])
}

#[tokio::test]
async fn keeps_both_participants_when_both_are_members() -> Result<(), Error> {
    let coach_id = Id::new_v4();
    let coachee_id = Id::new_v4();

    // Rows come back reversed, so the asserted order can only come from the input.
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![member_row(coachee_id), member_row(coach_id)]])
        .into_connection();

    let retained = retain_organization_members(&db, &[coach_id, coachee_id], Id::new_v4()).await?;

    assert_eq!(retained, vec![coach_id, coachee_id]);

    Ok(())
}

#[tokio::test]
async fn drops_a_participant_removed_from_the_organization() -> Result<(), Error> {
    let coach_id = Id::new_v4();
    let coachee_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![member_row(coach_id)]])
        .into_connection();

    let retained = retain_organization_members(&db, &[coach_id, coachee_id], Id::new_v4()).await?;

    assert_eq!(retained, vec![coach_id]);

    Ok(())
}

#[tokio::test]
async fn drops_every_participant_when_none_are_members() -> Result<(), Error> {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([Vec::<BTreeMap<String, Value>>::new()])
        .into_connection();

    let retained =
        retain_organization_members(&db, &[Id::new_v4(), Id::new_v4()], Id::new_v4()).await?;

    assert!(retained.is_empty(), "retained: {retained:?}");

    Ok(())
}

/// A global SuperAdmin holds no organization row, so the query must also match
/// `role = SuperAdmin` with a null organization.
#[tokio::test]
async fn keeps_a_global_super_admin_without_an_organization_role() -> Result<(), Error> {
    let coach_id = Id::new_v4();
    let coachee_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![member_row(coachee_id)]])
        .into_connection();

    let retained = retain_organization_members(&db, &[coach_id, coachee_id], Id::new_v4()).await?;

    assert!(retained.contains(&coachee_id), "retained: {retained:?}");

    let sql = format!("{:?}", db.into_transaction_log()[0]);
    assert!(
        sql.contains("IS NULL"),
        "global SuperAdmin rows must be matched: {sql}"
    );

    Ok(())
}

#[tokio::test]
async fn returns_each_user_once_when_they_hold_several_roles() -> Result<(), Error> {
    let coach_id = Id::new_v4();

    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([vec![member_row(coach_id), member_row(coach_id)]])
        .into_connection();

    let retained = retain_organization_members(&db, &[coach_id], Id::new_v4()).await?;

    assert_eq!(
        retained.iter().filter(|id| **id == coach_id).count(),
        1,
        "retained: {retained:?}"
    );

    Ok(())
}

#[tokio::test]
async fn m5_06_one_exists_statement_keeps_input_order() -> Result<(), Error> {
    let ids: Vec<Id> = (0..3).map(|_| Id::new_v4()).collect();

    // Rows come back reversed, so the asserted order can only come from the input.
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results([ids
            .iter()
            .rev()
            .map(|&id| member_row(id))
            .collect::<Vec<_>>()])
        .into_connection();

    let retained = retain_organization_members(&db, &ids, Id::new_v4()).await?;

    assert_eq!(retained, ids);
    let log = db.into_transaction_log();
    assert_eq!(log.len(), 1, "{log:?}");
    let sql = &log[0].statements()[0].sql;
    let filter = &sql[sql.find(" WHERE ").unwrap_or(0)..];
    assert!(
        filter.contains(r#"EXISTS(SELECT 1 FROM "refactor_platform"."user_roles""#),
        "{sql}"
    );
    Ok(())
}
