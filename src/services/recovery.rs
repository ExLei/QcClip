use thiserror::Error;

use super::{
    database::Database,
    storage::{RawBlobError, RawBlobStore},
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StartupRecoveryReport {
    pub pending_items_marked_corrupt: usize,
    pub pending_blobs_marked_corrupt: usize,
    pub pending_files_removed: usize,
}

#[derive(Debug, Error)]
pub enum RecoveryError {
    #[error("无法连接数据库：{0}")]
    Database(#[from] super::database::DatabaseError),
    #[error("读取启动恢复数据失败：{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("原始数据恢复失败：{0}")]
    Storage(#[from] RawBlobError),
}

/// 执行不读取正式原始文件的启动恢复。
pub fn reconcile_startup(
    database: &Database,
    blob_store: &RawBlobStore,
) -> Result<StartupRecoveryReport, RecoveryError> {
    let mut connection = database.connect()?;
    if !Database::maintenance_supported(&connection)? {
        tracing::info!("数据库处于兼容模式，已暂停启动恢复和未完成文件清理");
        return Ok(StartupRecoveryReport::default());
    }
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let pending_files_removed = blob_store.cleanup_pending_files()?;
    let pending_items_marked_corrupt = transaction.execute(
        "UPDATE clipboard_items
         SET capture_state = 'corrupt', updated_at = CAST(strftime('%s','now') AS INTEGER) * 1000
         WHERE capture_state = 'pending'",
        [],
    )?;
    let pending_blobs_marked_corrupt = transaction.execute(
        "UPDATE binary_blobs
         SET storage_state = 'corrupt', last_error = '应用上次退出时原始数据尚未提交'
         WHERE storage_state = 'pending'",
        [],
    )?;
    transaction.commit()?;
    Ok(StartupRecoveryReport {
        pending_items_marked_corrupt,
        pending_blobs_marked_corrupt,
        pending_files_removed,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::services::capture::{CaptureService, mock::MockClipboardSource};

    #[test]
    fn startup_recovery_cleans_pending_files_without_validating_committed_files() {
        let directory = tempfile::tempdir().expect("无法创建测试临时目录");
        let database =
            Database::open(directory.path().join("test.sqlite3")).expect("无法创建测试数据库");
        let store = RawBlobStore::new(directory.path()).expect("无法创建测试文件存储");
        let _pending = store
            .write_pending(b"pending raw data")
            .expect("无法写入未完成原始数据");
        let pending_directory = directory.path().join("data").join(".pending");
        let capture = CaptureService::with_inline_threshold(database.clone(), store.clone(), 0);
        capture
            .store_captured_clipboard(&MockClipboardSource::text("hello").capture())
            .expect("无法保存测试内容");
        let connection = database.connect().expect("无法连接数据库");
        let blob: (Vec<u8>, i64, String) = connection
            .query_row(
                "SELECT sha256, byte_length, relative_path FROM binary_blobs LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("无法查询文件索引");
        let path = directory.path().join(blob.2);
        fs::remove_file(path).expect("无法删除测试文件");
        connection
            .execute(
                "INSERT INTO clipboard_items (
                uuid, hash_version, content_hash, content_kind, capture_state,
                created_at, updated_at
             ) VALUES ('pending-item', 1, ?1, 'text', 'pending', 1, 1)",
                [[9_u8; 32].as_slice()],
            )
            .expect("无法插入未完成内容");
        drop(connection);

        let report = reconcile_startup(&database, &store).expect("启动恢复失败");
        assert_eq!(report.pending_items_marked_corrupt, 1);
        assert_eq!(report.pending_blobs_marked_corrupt, 0);
        assert_eq!(report.pending_files_removed, 1);
        assert_eq!(fs::read_dir(pending_directory).unwrap().count(), 0);
        let connection = database.connect().expect("无法重新连接数据库");
        let states: (String, String) = connection
            .query_row(
                "SELECT
                (SELECT storage_state FROM binary_blobs LIMIT 1),
                (SELECT capture_state FROM clipboard_items WHERE uuid = 'pending-item')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("无法查询恢复状态");
        assert_eq!(states, ("ready".to_owned(), "corrupt".to_owned()));
    }

    #[test]
    fn startup_recovery_marks_unfinished_blobs_without_reading_formal_files() {
        let directory = tempfile::tempdir().expect("无法创建测试临时目录");
        let database =
            Database::open(directory.path().join("test.sqlite3")).expect("无法创建测试数据库");
        let store = RawBlobStore::new(directory.path()).expect("无法创建测试文件存储");
        let connection = database.connect().expect("无法连接测试数据库");
        connection
            .execute(
                "INSERT INTO binary_blobs (
                    uuid, sha256, byte_length, relative_path, storage_state, created_at
                 ) VALUES ('pending-blob', zeroblob(32), 10, 'data/missing.bin.zst', 'pending', 1)",
                [],
            )
            .expect("无法插入未完成原始数据记录");
        drop(connection);

        let report = reconcile_startup(&database, &store).expect("启动恢复失败");
        assert_eq!(report.pending_blobs_marked_corrupt, 1);
        let connection = database.connect().expect("无法重新连接测试数据库");
        let state: String = connection
            .query_row(
                "SELECT storage_state FROM binary_blobs WHERE uuid = 'pending-blob'",
                [],
                |row| row.get(0),
            )
            .expect("无法查询未完成原始数据状态");
        assert_eq!(state, "corrupt");
    }

    #[test]
    fn startup_recovery_leaves_orphaned_content_for_deferred_cleanup() {
        let directory = tempfile::tempdir().expect("无法创建测试临时目录");
        let database =
            Database::open(directory.path().join("test.sqlite3")).expect("无法创建测试数据库");
        let store = RawBlobStore::new(directory.path()).expect("无法创建测试文件存储");
        let capture = CaptureService::with_inline_threshold(database.clone(), store.clone(), 0);
        let history_id = capture
            .store_captured_clipboard(&MockClipboardSource::text("启动清理内容").capture())
            .expect("无法保存测试内容");
        let connection = database.connect().expect("无法连接测试数据库");
        let path: String = connection
            .query_row(
                "SELECT b.relative_path
                 FROM clipboard_formats f
                 JOIN binary_blobs b ON b.id = f.blob_id
                 WHERE f.item_id = (SELECT item_id FROM clipboard_history WHERE id = ?1)",
                [history_id],
                |row| row.get(0),
            )
            .expect("无法查询原始数据路径");
        connection
            .execute("DELETE FROM clipboard_history WHERE id = ?1", [history_id])
            .expect("无法模拟历史删除");
        drop(connection);

        let report = reconcile_startup(&database, &store).expect("启动恢复失败");

        assert_eq!(report.pending_items_marked_corrupt, 0);
        let connection = database.connect().expect("无法重新连接测试数据库");
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM clipboard_items", [], |row| row.get(0))
            .expect("无法验证启动清理结果");
        assert_eq!(count, 1);
        assert!(directory.path().join(path).exists());
    }
}
