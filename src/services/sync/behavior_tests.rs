use super::{
    protocol::Mode,
    repository,
    tests::{Fixture, paired, runtime},
};
use crate::services::{
    custom_text::CustomTextService,
    database::{Database, FavoriteRepository, NoteRepository, TagRepository},
};

#[test]
fn receive_feedback_identifies_affected_lists_and_ignores_duplicate_receipts() {
    let source = Fixture::new();
    let target = Fixture::new();
    let item = source.capture("历史和收藏共用的内容");
    let history = source.packet(item, Mode::ManualContent, 1);
    let outcome = repository::apply_with_feedback(
        &target.db,
        &target.blobs,
        &history.event,
        &history.resources,
        "测试设备",
    )
    .unwrap();
    let change = outcome.change.unwrap();
    assert!(change.history_id.is_some());
    assert_eq!(change.favorite_id, None);
    assert!(!change.tags_changed);
    let duplicate = repository::apply_with_feedback(
        &target.db,
        &target.blobs,
        &history.event,
        &history.resources,
        "测试设备",
    )
    .unwrap();
    assert!(duplicate.change.is_none());

    let mut favorite = source.packet(item, Mode::ManualFavorite, 2);
    favorite.event.sender = history.event.sender.clone();
    let shared = repository::apply_with_feedback(
        &target.db,
        &target.blobs,
        &favorite.event,
        &favorite.resources,
        "测试设备",
    )
    .unwrap()
    .change
    .unwrap();
    assert_eq!(shared.item_id, change.item_id);
    assert_eq!(shared.history_id, change.history_id);
    assert!(shared.favorite_id.is_some());

    let favorite_target = Fixture::new();
    let favorite_only = repository::apply_with_feedback(
        &favorite_target.db,
        &favorite_target.blobs,
        &favorite.event,
        &favorite.resources,
        "测试设备",
    )
    .unwrap()
    .change
    .unwrap();
    assert_eq!(favorite_only.history_id, None);
    assert!(favorite_only.favorite_id.is_some());
}

#[test]
fn remote_sources_work_without_history_and_preserve_local_captures() {
    use crate::services::database::{HistoryRepository, query::HistoryQuery};

    let source = Fixture::new();
    let target = Fixture::new();
    let item = source.capture("来源测试");
    source.db.connect().unwrap().execute(
        "UPDATE clipboard_history SET last_source_app_name = '浏览器', last_source_app_path = 'C:/private/browser.exe' WHERE item_id = ?1",
        [item],
    ).unwrap();
    let packet = source.packet(item, Mode::ManualFavorite, 1);
    assert_eq!(packet.event.source_app_name.as_deref(), Some("浏览器"));
    assert!(
        !serde_json::to_string(&packet.event)
            .unwrap()
            .contains("private/browser.exe")
    );
    repository::apply_with_feedback(
        &target.db,
        &target.blobs,
        &packet.event,
        &packet.resources,
        "工作电脑",
    )
    .unwrap();
    assert_eq!(target.count("clipboard_history"), 0);
    let favorites = FavoriteRepository::with_blob_store(target.db.clone(), target.blobs.clone());
    let rows = favorites.query(&HistoryQuery::default()).unwrap();
    assert_eq!(rows[0].source_app_name.as_deref(), Some("浏览器"));
    assert_eq!(rows[0].source_device_name.as_deref(), Some("工作电脑"));
    assert!(rows[0].source_app_icon.is_none());
    for search in ["浏览器", "工作电脑"] {
        let query = HistoryQuery {
            search: Some(search.into()),
            ..HistoryQuery::default()
        };
        assert_eq!(favorites.query(&query).unwrap().len(), 1);
        assert_eq!(favorites.count(&query).unwrap(), 1);
        assert_eq!(favorites.ordered_ids(&query).unwrap().len(), 1);
    }
    let forwarded = target.packet(rows[0].item_id, Mode::ManualFavorite, 1);
    assert_eq!(
        forwarded.event.source_app_name,
        packet.event.source_app_name
    );

    let local = target.capture("来源测试");
    target
        .db
        .connect()
        .unwrap()
        .execute(
            "UPDATE clipboard_history SET last_source_app_name = '本机编辑器' WHERE item_id = ?1",
            [local],
        )
        .unwrap();
    let rows = favorites.query(&HistoryQuery::default()).unwrap();
    assert_eq!(rows[0].source_app_name.as_deref(), Some("本机编辑器"));
    assert_eq!(rows[0].source_device_name, None);
    let mut newer = packet.event.clone();
    newer.id = uuid::Uuid::new_v4().to_string();
    newer.sequence = 2;
    repository::apply_with_feedback(
        &target.db,
        &target.blobs,
        &newer,
        &packet.resources,
        "工作电脑",
    )
    .unwrap();
    let rows = HistoryRepository::new(target.db.clone(), target.blobs.clone())
        .query(&HistoryQuery::default())
        .unwrap();
    assert_eq!(rows[0].source_app_name.as_deref(), Some("本机编辑器"));
    assert_eq!(rows[0].source_device_name, None);
}

#[test]
fn missing_application_and_late_events_do_not_erase_remote_source() {
    use crate::services::database::{HistoryRepository, query::HistoryQuery};

    let source = Fixture::new();
    let target = Fixture::new();
    let item = source.capture("没有应用名称");
    let packet = source.packet(item, Mode::ManualContent, 3);
    repository::apply_with_feedback(
        &target.db,
        &target.blobs,
        &packet.event,
        &packet.resources,
        "家用电脑",
    )
    .unwrap();
    let mut late = source.packet(item, Mode::ManualFavorite, 2);
    late.event.sender = packet.event.sender.clone();
    late.event.source_app_name = Some("过期应用".into());
    repository::apply_with_feedback(
        &target.db,
        &target.blobs,
        &late.event,
        &late.resources,
        "旧设备名称",
    )
    .unwrap();
    let rows = HistoryRepository::new(target.db.clone(), target.blobs.clone())
        .query(&HistoryQuery::default())
        .unwrap();
    assert_eq!(rows[0].source_app_name, None);
    assert_eq!(rows[0].source_device_name.as_deref(), Some("家用电脑"));
    let history = HistoryRepository::new(target.db.clone(), target.blobs.clone());
    let query = HistoryQuery {
        search: Some("家用电脑".into()),
        ..HistoryQuery::default()
    };
    assert_eq!(history.query(&query).unwrap().len(), 1);
    assert_eq!(history.count(&query).unwrap(), 1);
    assert_eq!(history.ordered_ids(&query).unwrap().len(), 1);
    assert_eq!(target.count("favorites"), 1);
    for name in ["含\n换行".to_owned(), "a".repeat(513)] {
        let mut invalid = packet.event.clone();
        invalid.source_app_name = Some(name);
        assert!(
            invalid
                .validate(&invalid.sender, &invalid.recipient, &invalid.session)
                .is_err()
        );
    }
}

#[test]
fn latest_local_action_preserves_destination_and_ignores_remote_receipts_and_edits() {
    let local = Fixture::new();
    let remote = Fixture::new();
    let (config, _) = paired();
    let service = runtime(&local, config.clone());
    assert!(service.latest_upload().unwrap().is_none());
    let first = local.capture("本机复制");
    let second = local.capture("稍后收藏的内容");
    let favorites = FavoriteRepository::with_blob_store(local.db.clone(), local.blobs.clone());
    favorites.add_to_favorites(first).unwrap();
    let latest = service.latest_upload().unwrap().unwrap();
    assert_eq!((latest.item_id, latest.mode), (first, Mode::ManualFavorite));
    assert!(latest.preview.contains("本机复制"));
    favorites.add_to_favorites(first).unwrap();
    NoteRepository::new(local.db.clone())
        .save(second, "编辑备注不改变最近新增")
        .unwrap();
    let incoming = remote.packet(remote.capture("远端最新内容"), Mode::NewContent, 1);
    repository::apply(
        &local.db,
        &local.blobs,
        &incoming.event,
        &incoming.resources,
    )
    .unwrap();
    assert_eq!(service.latest_upload().unwrap().unwrap().item_id, first);
    local.capture("稍后收藏的内容");
    let latest = service.latest_upload().unwrap().unwrap();
    assert_eq!((latest.item_id, latest.mode), (second, Mode::ManualContent));
    let reopened = Database::open(local.db.path()).unwrap();
    assert_eq!(
        super::activity::latest(&reopened).unwrap().unwrap().item_id,
        second
    );
}

#[test]
fn manually_created_favorite_is_latest_even_when_it_also_has_history() {
    let local = Fixture::new();
    let custom = CustomTextService::new(local.db.clone(), local.blobs.clone());
    custom.create_manual_text_and_favorite("新建收藏").unwrap();
    assert_eq!(
        super::activity::latest(&local.db).unwrap().unwrap().mode,
        Mode::ManualFavorite
    );
    custom.create_manual_text("新建历史").unwrap();
    assert_eq!(
        super::activity::latest(&local.db).unwrap().unwrap().mode,
        Mode::ManualContent
    );
}

#[test]
fn history_and_favorite_deliveries_merge_metadata_without_changing_each_others_lists() {
    let source = Fixture::new();
    let target = Fixture::new();
    let item = source.capture("带标签的内容");
    let tags = TagRepository::new(source.db.clone());
    let work = tags.create("工作", "#123456").unwrap();
    tags.add_to_item(item, work.id).unwrap();
    NoteRepository::new(source.db.clone())
        .save(item, "发送方备注")
        .unwrap();
    let history = source.packet(item, Mode::ManualContent, 1);
    repository::apply(
        &target.db,
        &target.blobs,
        &history.event,
        &history.resources,
    )
    .unwrap();
    assert_eq!(target.count("clipboard_history"), 1);
    assert_eq!(target.count("favorites"), 0);
    assert_eq!(target.count("tags"), 1);
    assert_eq!(target.count("clipboard_item_notes"), 1);

    let mut favorite = source.packet(item, Mode::ManualFavorite, 2);
    favorite.event.sender = history.event.sender.clone();
    repository::apply(
        &target.db,
        &target.blobs,
        &favorite.event,
        &favorite.resources,
    )
    .unwrap();
    assert_eq!(target.count("clipboard_items"), 1);
    assert_eq!(target.count("clipboard_history"), 1);
    assert_eq!(target.count("favorites"), 1);
    assert_eq!(target.count("tags"), 1);
    let only_favorite = Fixture::new();
    repository::apply(
        &only_favorite.db,
        &only_favorite.blobs,
        &favorite.event,
        &favorite.resources,
    )
    .unwrap();
    assert_eq!(only_favorite.count("clipboard_history"), 0);
    assert_eq!(only_favorite.count("favorites"), 1);
}

#[test]
fn deleted_or_unfavorited_latest_target_is_not_silently_changed_to_history() {
    let local = Fixture::new();
    let item = local.capture("取消收藏后不可误发历史");
    let favorites = FavoriteRepository::with_blob_store(local.db.clone(), local.blobs.clone());
    favorites.add_to_favorites(item).unwrap();
    let favorite_id = local
        .db
        .connect()
        .unwrap()
        .query_row("SELECT id FROM favorites", [], |r| r.get(0))
        .unwrap();
    favorites.remove_from_favorites(favorite_id).unwrap();
    assert!(super::activity::latest(&local.db).unwrap().is_none());
    local.capture("取消收藏后不可误发历史");
    assert_eq!(
        super::activity::latest(&local.db).unwrap().unwrap().mode,
        Mode::ManualContent
    );
}

#[test]
fn receive_reuses_named_tags_keeps_local_tags_and_reports_note_conflicts() {
    let source = Fixture::new();
    let target = Fixture::new();
    let item = source.capture("保留本机整理");
    let local_item = target.capture("保留本机整理");
    let source_tags = TagRepository::new(source.db.clone());
    let incoming = source_tags.create("工作", "#123456").unwrap();
    source_tags.add_to_item(item, incoming.id).unwrap();
    let local_tags = TagRepository::new(target.db.clone());
    let existing = local_tags.create("工作", "#FFFFFF").unwrap();
    let personal = local_tags.create("个人", "#112233").unwrap();
    local_tags.add_to_item(local_item, personal.id).unwrap();
    NoteRepository::new(source.db.clone())
        .save(item, "远端备注")
        .unwrap();
    let notes = NoteRepository::new(target.db.clone());
    notes.save(local_item, "本机备注").unwrap();
    let packet = source.packet(item, Mode::ManualContent, 1);
    let outcome = repository::apply_with_feedback(
        &target.db,
        &target.blobs,
        &packet.event,
        &packet.resources,
        "测试设备",
    )
    .unwrap();
    assert!(outcome.applied && outcome.note_conflict);
    assert_eq!(target.count("tags"), 2);
    assert_eq!(target.count("clipboard_item_tags"), 2);
    assert_eq!(notes.load(local_item).unwrap(), "本机备注");
    let color: String = target
        .db
        .connect()
        .unwrap()
        .query_row("SELECT color FROM tags WHERE id = ?1", [existing.id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(color, "#FFFFFF");
}

#[test]
fn delayed_favorite_adds_favorite_without_reverting_newer_history_content() {
    let source = Fixture::new();
    let target = Fixture::new();
    let old = source.capture("旧内容");
    let old_favorite = source.packet(old, Mode::ManualFavorite, 1);
    let mut newer = source.packet(source.capture("后续更新内容"), Mode::ManualContent, 2);
    newer.event.sender = old_favorite.event.sender.clone();
    newer.event.item_uuid = old_favorite.event.item_uuid.clone();
    repository::apply(&target.db, &target.blobs, &newer.event, &newer.resources).unwrap();
    repository::apply(
        &target.db,
        &target.blobs,
        &old_favorite.event,
        &old_favorite.resources,
    )
    .unwrap();
    assert_eq!(target.count("favorites"), 1);
    assert_eq!(target.count("clipboard_history"), 1);
    let hash: Vec<u8> = target
        .db
        .connect()
        .unwrap()
        .query_row("SELECT content_hash FROM clipboard_items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(hash, newer.event.source_hash);
}
