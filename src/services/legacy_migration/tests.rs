use std::io::Write;

use super::*;

const LEGACY_SCHEMA: &str = "
    CREATE TABLE clipboard (id INTEGER PRIMARY KEY, content TEXT NOT NULL, html_content TEXT,
        content_type TEXT NOT NULL, image_id TEXT, item_order INTEGER NOT NULL DEFAULT 0,
        is_pinned INTEGER NOT NULL DEFAULT 0, paste_count INTEGER NOT NULL DEFAULT 0,
        source_app TEXT, source_icon_hash TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
    CREATE TABLE favorites (id TEXT PRIMARY KEY, title TEXT NOT NULL, content TEXT NOT NULL,
        html_content TEXT, content_type TEXT NOT NULL, image_id TEXT, group_name TEXT NOT NULL DEFAULT '全部',
        item_order INTEGER NOT NULL DEFAULT 0, paste_count INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
    CREATE TABLE groups (name TEXT PRIMARY KEY, icon TEXT, color TEXT, order_index INTEGER);
    CREATE TABLE clipboard_data (id INTEGER PRIMARY KEY, target_kind TEXT, target_id TEXT,
        format_name TEXT, raw_data BLOB, is_primary INTEGER, format_order INTEGER);
    PRAGMA user_version = 7;";

struct Fixture {
    _directory: TempDir,
    old_root: PathBuf,
    new_root: PathBuf,
    old: Connection,
    service: DataManagementService,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().expect("创建测试目录失败");
        let old_root = directory.path().join("old");
        let new_root = directory.path().join("new");
        fs::create_dir_all(&old_root).expect("创建旧版目录失败");
        fs::create_dir_all(&new_root).expect("创建新版目录失败");
        let old = Connection::open(old_root.join("quickclipboard.db")).expect("创建旧版数据库失败");
        old.execute_batch(LEGACY_SCHEMA).expect("创建旧版表失败");
        let service = DataManagementService::legacy_test_service(&new_root);
        Self {
            _directory: directory,
            old_root,
            new_root,
            old,
            service,
        }
    }

    fn favorite(&self, id: &str, text: &str, group: &str, title: &str) {
        self.old.execute("INSERT INTO favorites (id, title, content, content_type, group_name, paste_count, created_at, updated_at)
            VALUES (?1, ?2, ?3, 'text', ?4, 3, 1700000000, 1700000100)", params![id, title, text, group]).expect("插入收藏失败");
    }

    fn history(&self, id: i64, text: &str) {
        self.old.execute("INSERT INTO clipboard (id, content, content_type, item_order, is_pinned, paste_count, source_app, created_at, updated_at)
            VALUES (?1, ?2, 'text', ?1, 1, 5, '记事本', 1700000000, 1700000200)", params![id, text]).expect("插入历史失败");
    }

    fn icon(&self, ids: &[i64]) -> (String, Vec<u8>) {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(32, 32, image::Rgba([20, 80, 120, 255]))
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        let hash = self.icon_bytes(ids, &bytes);
        (hash, bytes)
    }

    fn icon_bytes(&self, ids: &[i64], bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        let hash: String = digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        fs::create_dir_all(self.old_root.join("app_icons")).unwrap();
        fs::write(
            self.old_root.join("app_icons").join(format!("{hash}.png")),
            bytes,
        )
        .unwrap();
        for id in ids {
            self.old
                .execute(
                    "UPDATE clipboard SET source_icon_hash = ?1 WHERE id = ?2",
                    params![hash, id],
                )
                .unwrap();
        }
        hash
    }

    fn connection(&self) -> Connection {
        let connection =
            Connection::open(self.new_root.join("qcclip.sqlite3")).expect("打开新版数据库失败");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .unwrap();
        connection
    }

    fn migrate(&self, options: MigrationOptions) -> MigrationReport {
        let prepared = prepare(&self.old_root, &self.new_root, options).expect("分析失败");
        migrate(&prepared, options, &self.service, &AtomicUsize::new(0)).expect("迁移失败")
    }

    fn count(&self, table: &str) -> i64 {
        self.connection()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("计数失败")
    }
}

#[test]
fn merges_history_and_favorites_with_multiple_tags_and_titles() {
    let fixture = Fixture::new();
    fixture.history(1, "共同内容");
    fixture.favorite("a", "共同内容", "工作", "工作标题");
    fixture.favorite("b", "共同内容", "生活", "生活标题");
    fixture.old.execute_batch("INSERT INTO groups VALUES ('工作', 'ti ti-folder', '#dc2626', 1),
        ('生活', 'ti ti-folder', '#3b82f6', 2), ('空分组', 'ti ti-folder', '#f00', 3), ('全部', '', '', 0);").unwrap();
    let prepared = prepare(
        &fixture.old_root,
        &fixture.new_root,
        MigrationOptions {
            history: true,
            favorites: true,
        },
    )
    .unwrap();
    assert_eq!(prepared.preview.duplicate_contents, 2);
    let report = fixture.migrate(MigrationOptions {
        history: true,
        favorites: true,
    });
    assert_eq!(report.added_contents, 1);
    assert_eq!(report.merged_contents, 2);
    assert_eq!(fixture.count("clipboard_history"), 1);
    assert_eq!(fixture.count("favorites"), 1);
    assert_eq!(fixture.count("tags"), 3);
    assert_eq!(fixture.count("clipboard_item_tags"), 2);
    let connection = fixture.connection();
    let note: String = connection
        .query_row("SELECT note_text FROM clipboard_item_notes", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(note, "工作标题\n\n生活标题");
    let (first, last, pinned, app): (i64, i64, bool, String) = connection.query_row(
        "SELECT first_seen_at, last_seen_at, is_pinned, last_source_app_name FROM clipboard_history", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).unwrap();
    assert_eq!(
        (first, last, pinned, app.as_str()),
        (1700000000000, 1700000200000, true, "记事本")
    );
    assert_eq!(
        connection
            .query_row("SELECT paste_count FROM favorites", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        6
    );
    assert_eq!(
        fixture
            .old
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        7
    );
}

#[test]
fn favorites_only_and_repeated_import_preserve_user_changes() {
    let fixture = Fixture::new();
    fixture.history(1, "历史");
    fixture.favorite("a", "收藏", "全部", "1412514");
    let report = fixture.migrate(MigrationOptions::default());
    assert_eq!(fixture.count("clipboard_history"), 0);
    assert_eq!(fixture.count("tags"), 0);
    assert!(report.backup.is_file());
    assert_eq!(
        fixture
            .connection()
            .query_row("SELECT note_text FROM clipboard_item_notes", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "1412514"
    );
    fixture.connection().execute_batch("UPDATE clipboard_item_notes SET note_text = '用户新备注'; UPDATE favorites SET is_pinned = 1;").unwrap();
    let prepared = prepare(
        &fixture.old_root,
        &fixture.new_root,
        MigrationOptions {
            history: true,
            favorites: true,
        },
    )
    .unwrap();
    assert_eq!(prepared.preview.already_imported, 1);
    let repeated = fixture.migrate(MigrationOptions::default());
    assert_eq!(repeated.skipped, 1);
    assert_eq!(repeated.added_contents, 0);
    let connection = fixture.connection();
    assert_eq!(
        connection
            .query_row("SELECT paste_count FROM favorites", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        connection
            .query_row("SELECT note_text FROM clipboard_item_notes", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "用户新备注"
    );
    assert!(
        connection
            .query_row("SELECT is_pinned FROM favorites", [], |row| row
                .get::<_, bool>(0))
            .unwrap()
    );
    connection
        .execute_batch("DELETE FROM favorites; DELETE FROM clipboard_items;")
        .unwrap();
    assert_eq!(
        fixture.migrate(MigrationOptions::default()).added_contents,
        0
    );
}

#[test]
fn raw_format_sets_do_not_merge_by_display_text() {
    let fixture = Fixture::new();
    fixture.favorite("a", "相同显示文本", "Work", "");
    fixture.favorite("b", "相同显示文本", "work", "");
    for (id, html) in [("a", "<b>相同显示文本</b>"), ("b", "<i>相同显示文本</i>")] {
        fixture.old.execute("INSERT INTO clipboard_data (target_kind, target_id, format_name, raw_data, is_primary, format_order)
            VALUES ('favorite', ?1, 'HTML Format', ?2, 1, 0)", params![id, encode_html(html)]).unwrap();
    }
    let report = fixture.migrate(MigrationOptions::default());
    assert_eq!(report.added_contents, 2);
    assert_eq!(fixture.count("tags"), 1);
    assert_eq!(fixture.count("favorites"), 2);
    assert_eq!(fixture.connection().query_row("SELECT COUNT(*) FROM clipboard_formats WHERE format_key = 'registered:HTML Format' AND capture_state = 'captured'", [], |row| row.get::<_, i64>(0)).unwrap(), 2);
}

#[test]
fn image_cache_is_converted_and_missing_images_are_skipped() {
    let fixture = Fixture::new();
    let images = fixture.old_root.join("clipboard_images");
    fs::create_dir(&images).unwrap();
    let image_path = images.join("test.png");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([20, 80, 120, 255]))
        .save(&image_path)
        .unwrap();
    let content = format!(
        "files:{}",
        serde_json::json!({ "files": [{ "path": image_path }], "operation": "copy" })
    );
    fixture.favorite("a", &content, "全部", "");
    fixture.favorite("b", "image:missing", "全部", "");
    fixture.old.execute_batch("UPDATE favorites SET content_type = 'image', image_id = CASE id WHEN 'a' THEN 'test' ELSE 'missing' END;").unwrap();
    let prepared = prepare(
        &fixture.old_root,
        &fixture.new_root,
        MigrationOptions {
            history: true,
            favorites: true,
        },
    )
    .unwrap();
    assert_eq!(prepared.preview.skipped, 1);
    let record = StagedMigration::open(&prepared.staged_path)
        .unwrap()
        .load_record(1)
        .unwrap();
    assert_eq!(record.captured.formats.len(), 1);
    assert_eq!(record.captured.formats[0].format_key, "registered:PNG");
    fs::remove_file(image_path).unwrap();
    migrate(
        &prepared,
        MigrationOptions::default(),
        &fixture.service,
        &AtomicUsize::new(0),
    )
    .unwrap();
    assert_eq!(fixture.count("favorites"), 1);
    assert_eq!(
        fixture
            .connection()
            .query_row("SELECT content_kind FROM clipboard_items", [], |row| row
                .get::<_, String>(
                0
            ))
            .unwrap(),
        "image"
    );
}

#[test]
fn transaction_failure_rolls_back_records_and_new_blob_files() {
    let fixture = Fixture::new();
    fixture.favorite("a", &"很长内容".repeat(20000), "测试分组", "标题");
    fixture.connection().execute_batch("CREATE TRIGGER test_reject_favorite BEFORE INSERT ON favorites BEGIN SELECT RAISE(ABORT, '测试拒绝收藏写入'); END;").unwrap();
    let prepared = prepare(
        &fixture.old_root,
        &fixture.new_root,
        MigrationOptions {
            history: true,
            favorites: true,
        },
    )
    .unwrap();
    assert!(
        migrate(
            &prepared,
            MigrationOptions::default(),
            &fixture.service,
            &AtomicUsize::new(0)
        )
        .is_err()
    );
    for table in [
        "clipboard_items",
        "clipboard_formats",
        "tags",
        "binary_blobs",
        "legacy_import_records",
    ] {
        assert_eq!(fixture.count(table), 0);
    }
    let data_files = fs::read_dir(fixture.new_root.join("data"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .count();
    assert_eq!(data_files, 0);
    assert_eq!(
        fixture
            .old
            .query_row("SELECT COUNT(*) FROM favorites", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn backup_can_restore_the_pre_migration_database() {
    let fixture = Fixture::new();
    fixture.favorite("a", "待迁移", "全部", "");
    let report = fixture.migrate(MigrationOptions::default());
    assert_eq!(fixture.count("favorites"), 1);
    fixture.service.stage_import(&report.backup).unwrap();
    assert!(super::super::data_management::apply_pending_import(&fixture.new_root).unwrap());
    assert_eq!(fixture.count("favorites"), 0);
}

#[test]
fn zip_input_and_directory_input_share_import_identity() {
    let fixture = Fixture::new();
    fixture.favorite("a", "来自 ZIP", "全部", "标题");
    fixture.migrate(MigrationOptions::default());
    let archive = fixture._directory.path().join("old.zip");
    let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
    zip.start_file(
        "quickclipboard.db",
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(&fs::read(fixture.old_root.join("quickclipboard.db")).unwrap())
        .unwrap();
    zip.finish().unwrap();
    let prepared = prepare(&archive, &fixture.new_root, MigrationOptions::default()).unwrap();
    assert_eq!(prepared.preview.already_imported, 1);
    assert_eq!(
        migrate(
            &prepared,
            MigrationOptions::default(),
            &fixture.service,
            &AtomicUsize::new(0)
        )
        .unwrap()
        .skipped,
        1
    );
}

#[test]
fn unsafe_archive_paths_are_rejected() {
    let fixture = Fixture::new();
    let archive = fixture._directory.path().join("unsafe.zip");
    let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
    zip.start_file("../escape", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(b"test").unwrap();
    zip.finish().unwrap();
    assert!(prepare(&archive, &fixture.new_root, MigrationOptions::default()).is_err());
    assert!(!fixture._directory.path().join("escape").exists());
}

#[test]
fn custom_storage_settings_take_priority_over_default_database() {
    let fixture = Fixture::new();
    let custom = fixture._directory.path().join("custom");
    fs::create_dir(&custom).unwrap();
    fs::copy(
        fixture.old_root.join("quickclipboard.db"),
        custom.join("quickclipboard.db"),
    )
    .unwrap();
    fs::write(
        fixture.old_root.join("settings.json"),
        serde_json::to_vec(&serde_json::json!({
            "use_custom_storage": true, "custom_storage_path": custom,
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(resolve_source_root(&fixture.old_root).unwrap(), custom);
}

#[test]
fn snapshot_includes_wal_without_changing_source_database() {
    let fixture = Fixture::new();
    fixture
        .old
        .execute_batch("PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;")
        .unwrap();
    fixture.favorite("wal", "尚未做检查点", "全部", "");
    let source_path = fixture.old_root.join("quickclipboard.db");
    let main_before = fs::read(&source_path).unwrap();
    let wal_before = fs::read(fixture.old_root.join("quickclipboard.db-wal")).unwrap();
    let prepared = prepare(
        &fixture.old_root,
        &fixture.new_root,
        MigrationOptions {
            history: true,
            favorites: true,
        },
    )
    .unwrap();
    assert_eq!(prepared.preview.favorites, 1);
    assert_eq!(fs::read(source_path).unwrap(), main_before);
    assert_eq!(
        fs::read(fixture.old_root.join("quickclipboard.db-wal")).unwrap(),
        wal_before
    );
    migrate(
        &prepared,
        MigrationOptions::default(),
        &fixture.service,
        &AtomicUsize::new(0),
    )
    .unwrap();
    assert_eq!(fixture.count("favorites"), 1);
}

#[test]
fn existing_tag_color_and_notes_are_preserved_when_merging() {
    let fixture = Fixture::new();
    fixture.favorite("a", "内容", "工作", "旧标题一");
    fixture.migrate(MigrationOptions::default());
    fixture.connection().execute_batch("UPDATE tags SET color = '#112233'; UPDATE clipboard_item_notes SET note_text = '已有备注';").unwrap();
    fixture.favorite("b", "内容", "工作", "旧标题二");
    fixture
        .old
        .execute_batch("INSERT INTO groups VALUES ('工作', '', '#dc2626', 1);")
        .unwrap();
    let prepared = prepare(
        &fixture.old_root,
        &fixture.new_root,
        MigrationOptions {
            history: true,
            favorites: true,
        },
    )
    .unwrap();
    assert!(
        prepared
            .preview
            .warnings
            .iter()
            .any(|warning| warning.contains("#112233"))
    );
    let report = migrate(
        &prepared,
        MigrationOptions::default(),
        &fixture.service,
        &AtomicUsize::new(0),
    )
    .unwrap();
    assert_eq!(report.added_tags, 0);
    assert_eq!(fixture.count("favorites"), 1);
    let connection = fixture.connection();
    assert_eq!(
        connection
            .query_row("SELECT color FROM tags", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "#112233"
    );
    let note: String = connection
        .query_row("SELECT note_text FROM clipboard_item_notes", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(note, "已有备注\n\n旧标题二");
}

#[test]
fn repeated_import_does_not_recreate_deleted_groups() {
    let fixture = Fixture::new();
    fixture
        .old
        .execute_batch("INSERT INTO groups VALUES ('空分组', '', '#dc2626', 1);")
        .unwrap();
    fixture.migrate(MigrationOptions::default());
    assert_eq!(fixture.count("tags"), 1);
    fixture
        .connection()
        .execute_batch("DELETE FROM tags;")
        .unwrap();
    fixture.migrate(MigrationOptions::default());
    assert_eq!(fixture.count("tags"), 0);
}

#[test]
fn source_icons_are_migrated_and_deduplicated_from_directory_and_zip() {
    for use_zip in [false, true] {
        let fixture = Fixture::new();
        fixture.history(1, "来源一");
        fixture.history(2, "来源二");
        let (hash, bytes) = fixture.icon(&[1, 2]);
        let archive = fixture._directory.path().join("icons.zip");
        let source = if use_zip {
            let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
            for (name, data) in [
                (
                    "quickclipboard.db".to_owned(),
                    fs::read(fixture.old_root.join("quickclipboard.db")).unwrap(),
                ),
                (format!("app_icons/{hash}.png"), bytes.clone()),
            ] {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(&data).unwrap();
            }
            zip.finish().unwrap();
            &archive
        } else {
            &fixture.old_root
        };
        let prepared = prepare(
            source,
            &fixture.new_root,
            MigrationOptions {
                history: true,
                favorites: false,
            },
        )
        .unwrap();
        assert_eq!(prepared.preview.warning_count, 0);
        let report = migrate(
            &prepared,
            MigrationOptions {
                history: true,
                favorites: false,
            },
            &fixture.service,
            &AtomicUsize::new(0),
        )
        .unwrap();
        assert_eq!(report.imported_icons, 2);
        assert_eq!(fixture.count("application_icon_assets"), 1);
        assert_eq!(fixture.count("application_icon_cache"), 0);
        let connection = fixture.connection();
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM clipboard_history WHERE last_source_app_icon_id IS NOT NULL", [], |row| row.get::<_, i64>(0)).unwrap(), 2);
        let (path, sha256, length, width, height): (String, Vec<u8>, i64, u32, u32) = connection.query_row(
            "SELECT relative_path, sha256, byte_length, width, height FROM application_icon_assets", [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))).unwrap();
        assert_eq!(fs::read(fixture.new_root.join(path)).unwrap(), bytes);
        assert_eq!(sha256, Sha256::digest(&bytes).to_vec());
        assert_eq!((length, width, height), (bytes.len() as i64, 32, 32));
        assert!(
            connection
                .prepare("PRAGMA foreign_key_check")
                .unwrap()
                .query([])
                .unwrap()
                .next()
                .unwrap()
                .is_none()
        );
        let repeated = fixture.migrate(MigrationOptions {
            history: true,
            favorites: false,
        });
        assert_eq!((repeated.imported_icons, repeated.skipped), (0, 2));
    }
}

#[test]
fn repeated_import_fills_missing_icons_without_changing_records_or_statistics() {
    let fixture = Fixture::new();
    fixture.history(1, "已迁移内容");
    fixture.favorite("a", "已迁移内容", "工作", "旧标题");
    let options = MigrationOptions {
        history: true,
        favorites: true,
    };
    fixture.migrate(options);
    let connection = fixture.connection();
    connection.execute_batch("UPDATE clipboard_item_notes SET note_text = '用户备注'; UPDATE clipboard_history SET is_pinned = 0;").unwrap();
    let before: (i64, i64, i64, i64, i64, String) = connection.query_row(
        "SELECT capture_count, paste_count, sort_order, first_seen_at, last_seen_at, uuid FROM clipboard_history", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))).unwrap();
    fixture.icon(&[1]);
    let prepared = prepare(
        &fixture.old_root,
        &fixture.new_root,
        MigrationOptions {
            history: true,
            favorites: true,
        },
    )
    .unwrap();
    assert_eq!(prepared.preview.already_imported, 2);
    let report = fixture.migrate(options);
    assert_eq!(report.imported_icons, 1);
    assert_eq!(
        (
            report.added_contents,
            report.added_history,
            report.added_favorites,
            report.added_tags
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(report.skipped, 1);
    let after = connection.query_row(
        "SELECT capture_count, paste_count, sort_order, first_seen_at, last_seen_at, uuid FROM clipboard_history", [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))).unwrap();
    assert_eq!(before, after);
    assert!(
        !connection
            .query_row("SELECT is_pinned FROM clipboard_history", [], |row| row
                .get::<_, bool>(0))
            .unwrap()
    );
    assert_eq!(
        connection
            .query_row("SELECT note_text FROM clipboard_item_notes", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap(),
        "用户备注"
    );
    assert_eq!(
        connection
            .query_row("SELECT paste_count FROM favorites", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(fixture.migrate(options).imported_icons, 0);
}

#[test]
fn icon_backfill_preserves_changed_sources_existing_icons_and_deleted_history() {
    let fixture = Fixture::new();
    for id in 1..=5 {
        fixture.history(id, &format!("内容{id}"));
    }
    let options = MigrationOptions {
        history: true,
        favorites: false,
    };
    fixture.migrate(options);
    fixture.icon(&[1, 2, 3, 4, 5]);
    let connection = fixture.connection();
    connection
        .execute_batch(
            "UPDATE clipboard_history SET last_source_app_name = '其他应用' WHERE id = 1;
        DELETE FROM clipboard_history WHERE id = 2;
        DELETE FROM clipboard_history WHERE id = 3; DELETE FROM clipboard_items WHERE id = 3;
        UPDATE clipboard_history SET last_source_app_name = NULL WHERE id = 4;",
        )
        .unwrap();
    assert_eq!(fixture.migrate(options).imported_icons, 1);
    assert_eq!(fixture.count("clipboard_history"), 3);
    assert_eq!(fixture.count("clipboard_items"), 4);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM clipboard_history WHERE last_source_app_icon_id IS NULL",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    let original: i64 = connection
        .query_row(
            "SELECT last_source_app_icon_id FROM clipboard_history WHERE id = 5",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut different_png = Vec::new();
    image::RgbaImage::from_pixel(32, 32, image::Rgba([180, 10, 20, 255]))
        .write_to(
            &mut Cursor::new(&mut different_png),
            image::ImageFormat::Png,
        )
        .unwrap();
    fixture.icon_bytes(&[5], &different_png);
    assert_eq!(fixture.migrate(options).imported_icons, 0);
    assert_eq!(fixture.count("application_icon_assets"), 1);
    assert_eq!(
        connection
            .query_row(
                "SELECT last_source_app_icon_id FROM clipboard_history WHERE id = 5",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        original
    );
}

#[test]
fn invalid_source_icon_caches_warn_without_losing_history() {
    let fixture = Fixture::new();
    for id in 1..=5 {
        fixture.history(id, &format!("内容{id}"));
    }
    let (hash, _) = fixture.icon(&[1]);
    fs::remove_file(
        fixture
            .old_root
            .join("app_icons")
            .join(format!("{hash}.png")),
    )
    .unwrap();
    fixture
        .old
        .execute(
            "UPDATE clipboard SET source_icon_hash = '../escape' WHERE id = 2",
            [],
        )
        .unwrap();
    fixture.icon_bytes(&[3], b"invalid PNG");
    let (mismatched, _) = fixture.icon(&[4]);
    fs::write(
        fixture
            .old_root
            .join("app_icons")
            .join(format!("{mismatched}.png")),
        b"modified",
    )
    .unwrap();
    let mut large_png = Vec::new();
    image::RgbaImage::from_pixel(257, 32, image::Rgba([20, 80, 120, 255]))
        .write_to(&mut Cursor::new(&mut large_png), image::ImageFormat::Png)
        .unwrap();
    fixture.icon_bytes(&[5], &large_png);
    let prepared = prepare(
        &fixture.old_root,
        &fixture.new_root,
        MigrationOptions {
            history: true,
            favorites: true,
        },
    )
    .unwrap();
    assert_eq!(prepared.preview.warning_count, 4);
    assert_eq!(prepared.preview.skipped, 0);
    let report = fixture.migrate(MigrationOptions {
        history: true,
        favorites: false,
    });
    assert_eq!((report.added_history, report.imported_icons), (5, 0));
    assert_eq!(fixture.count("application_icon_assets"), 0);
}

#[test]
fn transaction_failure_removes_new_icons_but_preserves_existing_files() {
    for existing_file in [false, true] {
        let fixture = Fixture::new();
        fixture.history(1, "图标历史");
        fixture.favorite("a", "稍后失败的收藏", "全部", "");
        let (_, bytes) = fixture.icon(&[1]);
        let prepared = prepare(
            &fixture.old_root,
            &fixture.new_root,
            MigrationOptions {
                history: true,
                favorites: true,
            },
        )
        .unwrap();
        let record = StagedMigration::open(&prepared.staged_path)
            .unwrap()
            .load_record(1)
            .unwrap();
        let icon = record.icon.as_ref().unwrap();
        let path = fixture.new_root.join(&icon.relative_path);
        if existing_file {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &bytes).unwrap();
        }
        fixture
            .connection()
            .execute_batch(
                "CREATE TRIGGER test_reject_favorite BEFORE INSERT ON favorites
            BEGIN SELECT RAISE(ABORT, '测试拒绝收藏写入'); END;",
            )
            .unwrap();
        assert!(
            migrate(
                &prepared,
                MigrationOptions {
                    history: true,
                    favorites: true
                },
                &fixture.service,
                &AtomicUsize::new(0)
            )
            .is_err()
        );
        for table in [
            "clipboard_items",
            "clipboard_history",
            "application_icon_assets",
            "legacy_import_records",
        ] {
            assert_eq!(fixture.count(table), 0);
        }
        assert_eq!(path.exists(), existing_file);
        if existing_file {
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
    }
}

#[test]
fn older_schema_without_icon_hash_still_migrates() {
    let fixture = Fixture::new();
    fixture
        .old
        .execute_batch("ALTER TABLE clipboard DROP COLUMN source_icon_hash;")
        .unwrap();
    fixture.history(1, "更早版本");
    let report = fixture.migrate(MigrationOptions {
        history: true,
        favorites: false,
    });
    assert_eq!((report.added_history, report.imported_icons), (1, 0));
}

#[test]
fn favorites_analysis_does_not_read_unselected_history_or_groups_for_history_only() {
    let fixture = Fixture::new();
    fixture.favorite("a", "可迁移收藏", "全部", "");
    fixture
        .old
        .execute_batch("DROP TABLE clipboard; INSERT INTO groups VALUES ('空分组', '', '#f00', 1);")
        .unwrap();
    let options = MigrationOptions::default();
    let prepared = prepare(&fixture.old_root, &fixture.new_root, options).unwrap();
    assert_eq!(
        (prepared.preview.history, prepared.preview.favorites),
        (0, 1)
    );
    assert!(
        migrate(
            &prepared,
            MigrationOptions {
                history: true,
                favorites: true
            },
            &fixture.service,
            &AtomicUsize::new(0)
        )
        .is_err()
    );
    migrate(&prepared, options, &fixture.service, &AtomicUsize::new(0)).unwrap();
    assert_eq!(fixture.count("favorites"), 1);

    let history_fixture = Fixture::new();
    history_fixture.history(1, "可迁移历史");
    history_fixture
        .old
        .execute_batch("DROP TABLE favorites; DROP TABLE groups;")
        .unwrap();
    let prepared = prepare(
        &history_fixture.old_root,
        &history_fixture.new_root,
        MigrationOptions {
            history: true,
            favorites: false,
        },
    )
    .unwrap();
    assert_eq!((prepared.preview.history, prepared.preview.groups), (1, 0));
}

#[test]
fn oversized_raw_formats_and_excess_format_counts_skip_only_affected_records() {
    let fixture = Fixture::new();
    fixture.favorite("a", "超大格式", "全部", "");
    fixture.favorite("b", "格式过多", "全部", "");
    fixture.favorite("c", "正常收藏", "全部", "");
    fixture.favorite("d", &"a".repeat(33 * 1024 * 1024), "全部", "");
    fixture.old.execute("INSERT INTO clipboard_data (target_kind, target_id, format_name, raw_data, is_primary, format_order)
        VALUES ('favorite', 'a', 'Rich Text Format', zeroblob(?1), 1, 0)", [(MAX_FORMAT_BYTES + 1) as i64]).unwrap();
    fixture.old.execute_batch("WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM numbers WHERE n < 257)
        INSERT INTO clipboard_data (target_kind, target_id, format_name, raw_data, is_primary, format_order)
        SELECT 'favorite', 'b', 'CF_UNICODETEXT', X'61000000', 1, n FROM numbers;").unwrap();
    let prepared = prepare(
        &fixture.old_root,
        &fixture.new_root,
        MigrationOptions::default(),
    )
    .unwrap();
    assert_eq!(
        (prepared.preview.favorites, prepared.preview.skipped),
        (1, 3)
    );
    assert!(
        prepared
            .preview
            .warnings
            .iter()
            .any(|warning| warning.contains("64 MB"))
    );
    assert!(
        prepared
            .preview
            .warnings
            .iter()
            .any(|warning| warning.contains("256"))
    );
    let report = migrate(
        &prepared,
        MigrationOptions::default(),
        &fixture.service,
        &AtomicUsize::new(0),
    )
    .unwrap();
    assert_eq!((report.added_favorites, report.skipped), (1, 3));
}

#[test]
fn record_reader_accepts_more_than_one_hundred_thousand_records() {
    let fixture = Fixture::new();
    fixture.old.execute_batch("WITH RECURSIVE numbers(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM numbers WHERE n < 100001)
        INSERT INTO favorites (id, title, content, content_type, created_at, updated_at)
        SELECT CAST(n AS TEXT), '', '重复内容', 'text', 1700000000, 1700000100 FROM numbers;").unwrap();
    let mut count = 0;
    read_records(&fixture.old, MigrationOptions::default(), |_| {
        count += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(count, 100001);
}

#[test]
fn prepared_data_can_exceed_previous_memory_limit_and_is_cleaned_on_drop() {
    let fixture = Fixture::new();
    let raw = vec![b'a'; 4 * 1024 * 1024];
    fixture.old.execute_batch("BEGIN;").unwrap();
    for index in 0..129 {
        let id = index.to_string();
        fixture.favorite(&id, "", "全部", "");
        fixture.old.execute("INSERT INTO clipboard_data (target_kind, target_id, format_name, raw_data, is_primary, format_order)
            VALUES ('favorite', ?1, 'Rich Text Format', ?2, 1, 0)", params![id, raw]).unwrap();
    }
    fixture.old.execute_batch("COMMIT;").unwrap();
    let prepared = prepare(
        &fixture.old_root,
        &fixture.new_root,
        MigrationOptions::default(),
    )
    .unwrap();
    assert_eq!(prepared.preview.favorites, 129);
    let scratch_path = prepared._scratch.path().to_path_buf();
    assert!(fs::metadata(&prepared.staged_path).unwrap().len() > 512 * 1024 * 1024);
    let report = migrate(
        &prepared,
        MigrationOptions::default(),
        &fixture.service,
        &AtomicUsize::new(0),
    )
    .unwrap();
    assert_eq!((report.added_contents, report.merged_contents), (1, 128));
    drop(prepared);
    assert!(!scratch_path.exists());
}

#[test]
fn zip_skips_unrelated_gallery_settings_and_unselected_icons() {
    let fixture = Fixture::new();
    fixture.favorite("a", "ZIP 收藏", "全部", "");
    let archive = fixture._directory.path().join("gallery.zip");
    let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
    for (name, bytes) in [
        (
            "quickclipboard.db",
            fs::read(fixture.old_root.join("quickclipboard.db")).unwrap(),
        ),
        (
            "image_library/unused.png",
            "图库不会迁移".as_bytes().to_vec(),
        ),
        ("app_icons/unused.png", b"unused icon".to_vec()),
        ("settings.json", b"{}".to_vec()),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&bytes).unwrap();
    }
    zip.finish().unwrap();
    let prepared = prepare(&archive, &fixture.new_root, MigrationOptions::default()).unwrap();
    assert!(
        !prepared
            ._scratch
            .path()
            .join("source/image_library/unused.png")
            .exists()
    );
    assert!(
        !prepared
            ._scratch
            .path()
            .join("source/settings.json")
            .exists()
    );
    assert!(
        !prepared
            ._scratch
            .path()
            .join("source/app_icons/unused.png")
            .exists()
    );
    assert!(
        prepared
            .preview
            .warnings
            .iter()
            .any(|warning| warning.contains("图库"))
    );
    assert!(
        prepared
            .preview
            .warnings
            .iter()
            .any(|warning| warning.contains("设置"))
    );
    assert_eq!(prepared.preview.favorites, 1);
}

#[test]
fn missing_staged_format_rolls_back_without_leaving_imported_data() {
    let fixture = Fixture::new();
    fixture.favorite("a", "第一条", "全部", "");
    fixture.favorite("b", "第二条", "全部", "");
    let prepared = prepare(
        &fixture.old_root,
        &fixture.new_root,
        MigrationOptions::default(),
    )
    .unwrap();
    Connection::open(&prepared.staged_path)
        .unwrap()
        .execute("DELETE FROM formats WHERE record_id = 2", [])
        .unwrap();
    assert!(matches!(
        migrate(
            &prepared,
            MigrationOptions::default(),
            &fixture.service,
            &AtomicUsize::new(0)
        ),
        Err(MigrationError::RolledBack { .. })
    ));
    assert_eq!(fixture.count("favorites"), 0);
    assert_eq!(fixture.count("clipboard_items"), 0);
}
