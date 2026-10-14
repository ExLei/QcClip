use super::*;
use crate::services::{
    capture::{CaptureService, mock::MockClipboardSource},
    database::{FavoriteRepository, HistoryRepository, query::HistoryQuery},
    storage::RawBlobStore,
};

#[test]
fn notes_are_shared_searchable_and_survive_favorite_removal() {
    let directory = tempfile::tempdir().expect("无法创建测试目录");
    let path = directory.path().join("test.sqlite3");
    let database = Database::open(&path).expect("无法创建数据库");
    let store = RawBlobStore::new(directory.path()).expect("无法创建数据存储");
    let capture = CaptureService::new(database.clone(), store.clone());
    let paste = crate::services::paste::PasteService::new(database.clone(), store.clone());
    let source = MockClipboardSource::text("原始内容").capture();
    let item_id = capture
        .store_captured_clipboard(&source)
        .expect("无法保存内容");
    let notes = NoteRepository::new(database.clone());
    let history = HistoryRepository::new(database.clone(), store);
    let favorites = FavoriteRepository::new(database.clone());
    notes
        .save(item_id, "测试环境 100%_\n第二行")
        .expect("无法保存备注");
    favorites.add_to_favorites(item_id).expect("无法收藏");
    let query = HistoryQuery {
        search: Some("100%_".to_owned()),
        ..HistoryQuery::default()
    };
    let history_rows = history.query(&query).expect("无法查询历史");
    let favorite_rows = favorites.query(&query).expect("无法查询收藏");
    assert_eq!(history_rows.len(), 1);
    assert_eq!(favorite_rows.len(), 1);
    assert_eq!(history_rows[0].note_text, favorite_rows[0].note_text);
    assert_eq!(history_rows[0].preview_text, "原始内容");
    assert_eq!(
        paste.item_plain_text(item_id).expect("无法读取待粘贴内容"),
        Some("原始内容".to_owned())
    );
    assert_eq!(history.count(&query).expect("无法统计历史"), 1);
    assert_eq!(favorites.count(&query).expect("无法统计收藏"), 1);
    assert_eq!(
        history.ordered_ids(&query).expect("无法查询历史顺序").len(),
        1
    );
    assert_eq!(
        favorites
            .ordered_ids(&query)
            .expect("无法查询收藏顺序")
            .len(),
        1
    );
    let no_match = HistoryQuery {
        search: Some("100X".to_owned()),
        ..query.clone()
    };
    assert!(history.query(&no_match).expect("无法查询历史").is_empty());
    assert!(favorites.query(&no_match).expect("无法查询收藏").is_empty());
    let connection = database.connect().expect("无法连接数据库");
    let identity: (i64, String, i64) = connection
        .query_row(
            "SELECT id, uuid, created_at FROM clipboard_item_notes WHERE item_id = ?1",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("无法读取备注身份");
    notes.save(item_id, "修改后的备注").expect("无法更新备注");
    let updated_identity: (i64, String, i64) = connection
        .query_row(
            "SELECT id, uuid, created_at FROM clipboard_item_notes WHERE item_id = ?1",
            [item_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("无法读取更新后的身份");
    assert_eq!(identity, updated_identity);
    assert_eq!(
        capture
            .store_captured_clipboard(&source)
            .expect("无法再次捕获"),
        item_id
    );
    favorites
        .remove_from_favorites(favorite_rows[0].history_id)
        .expect("无法取消收藏");
    let reopened = NoteRepository::new(Database::open(&path).expect("无法重新打开数据库"));
    assert_eq!(
        reopened.load(item_id).expect("无法读取备注"),
        "修改后的备注"
    );
    notes.save(item_id, " \n\t\u{3000}").expect("无法清空备注");
    assert_eq!(notes.load(item_id).expect("无法读取备注"), "");
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM clipboard_item_notes", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
    notes.save(item_id, "待清理备注").expect("无法保存备注");
    history
        .delete(history_rows[0].history_id)
        .expect("无法删除历史");
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM clipboard_item_notes", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
}

#[test]
fn missing_content_cannot_receive_a_note() {
    let directory = tempfile::tempdir().expect("无法创建测试目录");
    let database = Database::open(directory.path().join("test.sqlite3")).expect("无法创建数据库");
    let notes = NoteRepository::new(database);
    assert!(matches!(
        notes.save(999, "备注"),
        Err(NoteError::NotFound(999))
    ));
    assert!(matches!(notes.save(999, ""), Err(NoteError::NotFound(999))));
}
