use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use thiserror::Error;
use uuid::Uuid;

use super::Database;
use crate::domain::clipboard::ClipboardItemId;

#[derive(Clone, Debug)]
pub struct NoteRepository {
    database: Database,
}

#[derive(Debug, Error)]
pub enum NoteError {
    #[error("无法连接备注数据库：{0}")]
    Database(#[from] super::DatabaseError),
    #[error("读写备注失败：{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("备注对应的内容不存在或尚未完成：{0}")]
    NotFound(ClipboardItemId),
}

impl NoteRepository {
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    pub fn load(&self, item_id: ClipboardItemId) -> Result<String, NoteError> {
        let connection = self.database.connect()?;
        connection
            .query_row(
                "SELECT COALESCE(n.note_text, '') FROM clipboard_items AS i
             LEFT JOIN clipboard_item_notes AS n ON n.item_id = i.id
             WHERE i.id = ?1 AND i.capture_state = 'ready'",
                [item_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(NoteError::NotFound(item_id))
    }

    pub fn save(&self, item_id: ClipboardItemId, text: &str) -> Result<(), NoteError> {
        let text = text.trim();
        let mut connection = self.database.connect_write()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM clipboard_items WHERE id = ?1 AND capture_state = 'ready')",
            [item_id], |row| row.get(0),
        )?;
        if !exists {
            return Err(NoteError::NotFound(item_id));
        }
        if text.is_empty() {
            transaction.execute(
                "DELETE FROM clipboard_item_notes WHERE item_id = ?1",
                [item_id],
            )?;
        } else {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis() as i64)
                .unwrap_or(0);
            transaction.execute(
                "INSERT INTO clipboard_item_notes (uuid, item_id, note_text, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)
                 ON CONFLICT(item_id) DO UPDATE SET note_text = excluded.note_text,
                                                   updated_at = excluded.updated_at",
                params![Uuid::new_v4().to_string(), item_id, text, timestamp],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
