use sea_orm_migration::prelude::*;
use std::fs::File;
use std::io::Read;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        let stringify_err = |err| -> DbErr {
            let string = format!("Migration error: {:?}", err);
            DbErr::Migration(string)
        };
        let mut file =
            File::open("migration/src/base_refactor_platform_rs.sql").map_err(stringify_err)?;

        let mut sql = String::new();

        file.read_to_string(&mut sql).map_err(stringify_err)?;

        process_sql(&sql);

        db.execute_unprepared(&sql).await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

// format sql as valid sql statements
fn process_sql(sql: &str) -> String {
    sql.replace(';', ";\n")
        .lines()
        .filter(|line| !line.trim().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}
