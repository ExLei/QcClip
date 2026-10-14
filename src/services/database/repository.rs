use rusqlite::Connection;

use super::{DatabaseInfo, connection::DatabaseError};

pub struct DatabaseRepository<'connection> {
    connection: &'connection Connection,
}

impl<'connection> DatabaseRepository<'connection> {
    pub fn new(connection: &'connection Connection) -> Self {
        Self { connection }
    }

    pub fn database_info(&self) -> Result<DatabaseInfo, DatabaseError> {
        let schema_version = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|source| DatabaseError::Inspect { source })?;
        let has_compatibility: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'database_compatibility')",
            [],
            |row| row.get(0),
        ).map_err(|source| DatabaseError::Inspect { source })?;
        if !has_compatibility {
            return Err(DatabaseError::Validation(
                "数据库缺少读写兼容声明，无法确认与当前程序的兼容性，请保留原库并使用对应版本的程序打开".to_owned(),
            ));
        }
        let (count, id, minimum_read_version, minimum_write_version): (i64, Option<i32>, Option<i32>, Option<i32>) =
            self.connection.query_row(
                "SELECT COUNT(*), MIN(id), MIN(minimum_read_version), MIN(minimum_write_version)
                 FROM database_compatibility",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ).map_err(|source| DatabaseError::Inspect { source })?;
        let (Some(minimum_read_version), Some(minimum_write_version)) =
            (minimum_read_version, minimum_write_version) else {
                return Err(DatabaseError::Validation("数据库读写兼容声明为空".to_owned()));
            };
        if schema_version < 1 || count != 1 || id != Some(1)
            || minimum_read_version < 1 || minimum_write_version < minimum_read_version
        {
            return Err(DatabaseError::Validation("数据库结构版本或读写兼容声明无效".to_owned()));
        }
        let foreign_keys: i32 = self
            .connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .map_err(|source| DatabaseError::Inspect { source })?;
        let journal_mode: String = self
            .connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(|source| DatabaseError::Inspect { source })?;
        let busy_timeout_millis: i64 = self
            .connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .map_err(|source| DatabaseError::Inspect { source })?;

        Ok(DatabaseInfo {
            schema_version,
            minimum_read_version,
            minimum_write_version,
            foreign_keys_enabled: foreign_keys == 1,
            journal_mode,
            busy_timeout_millis,
        })
    }
}
