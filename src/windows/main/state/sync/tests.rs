use super::*;
use crate::services::{database::Database, image_library::ImageLibraryService};

fn create_main(
    database: &Database,
    blobs: &RawBlobStore,
    root: &Path,
    cx: &mut Context<MainWindow>,
) -> MainWindow {
    MainWindow::new(
        HistoryRepository::new(database.clone(), blobs.clone()),
        FavoriteRepository::with_blob_store(database.clone(), blobs.clone()),
        TagRepository::new(database.clone()),
        CaptureService::new(database.clone(), blobs.clone()),
        CustomTextService::new(database.clone(), blobs.clone()),
        PasteService::new(database.clone(), blobs.clone()),
        blobs.clone(),
        ImageLibraryService::new(root.join("images")),
        None,
        ClipboardWriteSuppressor::default(),
        cx,
    )
}

#[test]
#[ignore = "需要 GPUI 桌面会话，验证同步局部更新保留列表交互状态"]
fn remote_updates_preserve_browsing_and_only_update_related_rows() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_path_buf();
    let database = Database::open(root.join("qcclip.sqlite3")).unwrap();
    let blobs = RawBlobStore::new(&root).unwrap();
    gpui_platform::application().run(move |cx| {
        gpui_component::init(cx);
        crate::config_runtime::ConfigRuntime::install(root.join("config.json"), cx);
        crate::state_runtime::StateRuntime::install(root.join("state.json"), cx);
        let custom = CustomTextService::new(database.clone(), blobs.clone());
        for text in ["原有内容一", "原有内容二", "原有内容三"] {
            custom.create_manual_text(text).unwrap();
        }
        let main = cx.new(|cx| {
            let mut main = create_main(&database, &blobs, &root, cx);
            main.active_tab = TabId::Clipboard;
            let query = HistoryQuery::default();
            let policy = crate::config::HistoryConfig::default();
            let snapshot = main
                .history_repository
                .ordered_snapshot_scoped(&query, &policy)
                .unwrap();
            let records = main
                .history_repository
                .query_by_ids_scoped(&query, &snapshot.ids, &policy)
                .unwrap();
            assert!(main.accept_query_result(0, Ok((snapshot, records.clone()))));
            main.replace_items(records.into_iter().map(record_to_list_item).collect());
            main.selected_item_id = Some(main.list_order_ids[1]);
            main.multi_selected_item_ids.push(main.list_order_ids[1]);
            main.multi_selection_anchor = main.selected_item_id;
            main.list_scroll_handle
                .as_ref()
                .set_offset(gpui::point(px(0.0), px(-120.0)));
            main.adaptive_item_heights
                .insert(main.list_order_ids[1], px(65.0));
            main
        });
        cx.spawn(async move |cx| {
            let initial = main.read_with(cx, |main, _| {
                (
                    main.list_order_ids.clone(),
                    main.selected_item_id,
                    main.multi_selected_item_ids.clone(),
                    main.list_scroll_handle.as_ref().offset(),
                )
            });
            let history_id = custom
                .create_manual_text_and_favorite("远端收藏专用内容")
                .unwrap();
            let item_id: i64 = database
                .connect()
                .unwrap()
                .query_row(
                    "SELECT item_id FROM clipboard_history WHERE id = ?1",
                    [history_id],
                    |row| row.get(0),
                )
                .unwrap();
            let favorite_id = FavoriteRepository::new(database.clone())
                .favorite_id_for_item(item_id)
                .unwrap()
                .unwrap();
            // 模拟仅在收藏表接收，不把测试用创建器附带的历史关系留在数据库。
            database
                .connect()
                .unwrap()
                .execute("DELETE FROM clipboard_history WHERE id = ?1", [history_id])
                .unwrap();
            main.update(cx, |main, cx| {
                main.receive_sync_changes(
                    vec![ReceivedChange {
                        item_id,
                        history_id: None,
                        favorite_id: Some(favorite_id),
                        tags_changed: false,
                    }],
                    cx,
                )
            });
            wait_for_updates(&main, cx).await;
            main.update(cx, |main, _| {
                assert_eq!(main.list_order_ids, initial.0);
                assert_eq!(main.selected_item_id, initial.1);
                assert_eq!(main.query_generation, 0);
            });

            let new_history = custom.create_manual_text("远端新增历史").unwrap();
            let new_item = database
                .connect()
                .unwrap()
                .query_row(
                    "SELECT item_id FROM clipboard_history WHERE id = ?1",
                    [new_history],
                    |row| row.get(0),
                )
                .unwrap();
            main.update(cx, |main, cx| {
                main.receive_sync_changes(
                    vec![ReceivedChange {
                        item_id: new_item,
                        history_id: Some(new_history),
                        favorite_id: None,
                        tags_changed: false,
                    }],
                    cx,
                )
            });
            wait_for_updates(&main, cx).await;
            main.update(cx, |main, _| {
                assert_eq!(main.list_order_ids[0], new_history);
                assert_eq!(main.list_order_ids.len(), 4);
                assert_eq!(main.loaded_items.len(), 4);
                assert_eq!(main.selected_item_id, initial.1);
                assert_eq!(main.multi_selected_item_ids, initial.2);
                assert_eq!(main.list_scroll_handle.as_ref().offset(), initial.3);
                assert_eq!(
                    main.adaptive_item_heights.get(&initial.1.unwrap()),
                    Some(&px(65.0))
                );
                assert_eq!(main.query_generation, 0);
            });

            // 收藏页收到只属于历史的条目时，也不更改已展示列表。
            main.update(cx, |main, _| {
                main.active_tab = TabId::Favorites;
                let query = HistoryQuery::default();
                let snapshot = main.favorite_repository.ordered_snapshot(&query).unwrap();
                let records = main
                    .favorite_repository
                    .query_by_ids(&query, &snapshot.ids)
                    .unwrap();
                main.accept_query_result(0, Ok((snapshot, records)));
                main.selected_item_id = Some(favorite_id);
            });
            main.update(cx, |main, cx| {
                main.receive_sync_changes(
                    vec![ReceivedChange {
                        item_id: new_item,
                        history_id: Some(new_history),
                        favorite_id: None,
                        tags_changed: false,
                    }],
                    cx,
                )
            });
            wait_for_updates(&main, cx).await;
            main.update(cx, |main, _| {
                assert_eq!(main.list_order_ids, vec![favorite_id]);
                assert_eq!(main.selected_item_id, Some(favorite_id));
                assert_eq!(main.query_generation, 0);
            });

            let tags = TagRepository::new(database.clone());
            let tag = tags.create("远端标签", "#123456").unwrap();
            tags.add_to_item(item_id, tag.id).unwrap();
            crate::services::database::NoteRepository::new(database.clone())
                .save(item_id, "远端补入的备注")
                .unwrap();
            main.update(cx, |main, cx| {
                main.list_drag_source_id = Some(favorite_id);
                main.receive_sync_changes(
                    vec![ReceivedChange {
                        item_id,
                        history_id: None,
                        favorite_id: Some(favorite_id),
                        tags_changed: true,
                    }],
                    cx,
                );
            });
            cx.background_executor()
                .timer(Duration::from_millis(100))
                .await;
            main.update(cx, |main, _| {
                assert!(main.sync_refresh_task.is_some());
                assert_eq!(main.list_drag_source_id, Some(favorite_id));
                assert!(main.tags.is_empty());
                main.list_drag_source_id = None;
            });
            wait_for_updates(&main, cx).await;
            main.update(cx, |main, _| {
                assert_eq!(main.list_order_ids, vec![favorite_id]);
                assert_eq!(main.selected_item_id, Some(favorite_id));
                assert_eq!(main.loaded_items[&favorite_id].note_text, "远端补入的备注");
                assert_eq!(main.loaded_items[&favorite_id].tag_ids, vec![tag.id]);
                assert!(main.tags.iter().any(|value| value.id == tag.id));
                assert_eq!(main.query_generation, 0);
            });
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}

async fn wait_for_updates(main: &Entity<MainWindow>, cx: &mut gpui::AsyncApp) {
    for _ in 0..100 {
        if main.read_with(cx, |main, _| main.sync_refresh_task.is_none()) {
            return;
        }
        cx.background_executor()
            .timer(Duration::from_millis(50))
            .await;
    }
    panic!("同步列表局部更新超时");
}
