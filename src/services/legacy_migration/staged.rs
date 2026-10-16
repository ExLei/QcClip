use super::*;

pub(super) struct StagedMigration {
    connection: Connection,
}

impl StagedMigration {
    pub(super) fn create(path: &Path) -> Result<Self, MigrationError> {
        let connection = Connection::open(path)?;
        // 暂存只在本次迁移中使用，限制 SQLite 缓存，原始字节不随记录数累积在内存中。
        connection.execute_batch(
            "PRAGMA cache_size = -4096; PRAGMA temp_store = FILE;
            CREATE TABLE records (id INTEGER PRIMARY KEY, kind TEXT NOT NULL, legacy TEXT NOT NULL,
                primary_namespace TEXT, primary_key TEXT, source_key TEXT NOT NULL,
                content_hash BLOB NOT NULL, icon_id INTEGER);
            CREATE INDEX records_content_hash ON records(content_hash);
            CREATE TABLE formats (record_id INTEGER NOT NULL, format_order INTEGER NOT NULL,
                namespace TEXT NOT NULL, key TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL,
                native_id INTEGER, encoding TEXT, generated INTEGER NOT NULL, bytes BLOB NOT NULL,
                PRIMARY KEY(record_id, format_order));
            CREATE TABLE icons (id INTEGER PRIMARY KEY, bytes BLOB NOT NULL, sha256 BLOB NOT NULL,
                relative_path TEXT NOT NULL, width INTEGER NOT NULL, height INTEGER NOT NULL);
            BEGIN;",
        )?;
        Ok(Self { connection })
    }

    pub(super) fn open(path: &Path) -> Result<Self, MigrationError> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.execute_batch("PRAGMA cache_size = -4096;")?;
        Ok(Self { connection })
    }

    pub(super) fn finish(&self) -> Result<(), MigrationError> {
        self.connection.execute_batch("COMMIT;")?;
        Ok(())
    }

    pub(super) fn save_icon(&self, icon: &LegacyIcon) -> Result<i64, MigrationError> {
        self.connection.execute(
            "INSERT INTO icons (bytes, sha256, relative_path, width, height)
            VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                icon.bytes,
                icon.sha256.as_slice(),
                icon.relative_path,
                icon.width,
                icon.height
            ],
        )?;
        Ok(self.connection.last_insert_rowid())
    }

    pub(super) fn save_record(
        &self,
        legacy: &LegacyRecord,
        captured: &CapturedClipboard,
        hashed: &HashedClipboard,
        source_key: &str,
        icon_id: Option<i64>,
    ) -> Result<bool, MigrationError> {
        let duplicate = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM records WHERE content_hash = ?1)",
            [hashed.content_hash.as_slice()],
            |row| row.get(0),
        )?;
        let primary = captured.primary_format_identity.as_ref();
        self.connection.execute("INSERT INTO records (kind, legacy, primary_namespace, primary_key, source_key, content_hash, icon_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![legacy.kind, serde_json::to_string(legacy)?, primary.map(|value| &value.0),
                primary.map(|value| &value.1), source_key, hashed.content_hash.as_slice(), icon_id])?;
        let id = self.connection.last_insert_rowid();
        let mut statement = self.connection.prepare("INSERT INTO formats
            (record_id, format_order, namespace, key, name, kind, native_id, encoding, generated, bytes)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)")?;
        for format in &captured.formats {
            let bytes = format
                .bytes
                .as_deref()
                .ok_or_else(|| MigrationError::Invalid("暂存格式缺少原始字节".to_owned()))?;
            statement.execute(params![
                id,
                format.format_order,
                format.format_namespace,
                format.format_key,
                format.format_name,
                format.format_kind.as_str(),
                format.native_format_id,
                format.encoding,
                format.capture_state == FormatCaptureState::Generated,
                bytes
            ])?;
        }
        Ok(duplicate)
    }

    pub(super) fn load_record(&self, id: i64) -> Result<PreparedRecord, MigrationError> {
        let (json, primary_namespace, primary_key, source_key, icon_id, expected_hash) = self.connection.query_row(
            "SELECT legacy, primary_namespace, primary_key, source_key, icon_id, content_hash FROM records WHERE id = ?1", [id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?, row.get::<_, String>(3)?, row.get::<_, Option<i64>>(4)?, row.get::<_, Vec<u8>>(5)?)))?;
        let legacy: LegacyRecord = serde_json::from_str(&json)?;
        let mut statement = self.connection.prepare("SELECT namespace, key, name, kind, native_id,
            encoding, generated, bytes, format_order FROM formats WHERE record_id = ?1 ORDER BY format_order")?;
        let mut rows = statement.query([id])?;
        let mut formats = Vec::new();
        while let Some(row) = rows.next()? {
            let kind = FormatKind::from_str(&row.get::<_, String>(3)?)
                .ok_or_else(|| MigrationError::Invalid("暂存格式类型无效".to_owned()))?;
            let mut format = RawFormat::captured(
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                kind,
                row.get(7)?,
                row.get(8)?,
                true,
            );
            format.native_format_id = row.get(4)?;
            format.encoding = row.get(5)?;
            if row.get::<_, bool>(6)? {
                format.capture_state = FormatCaptureState::Generated;
            }
            formats.push(format);
        }
        let captured = CapturedClipboard {
            formats,
            primary_format_identity: primary_namespace.zip(primary_key),
            preview_candidate: Some(legacy.content.clone()),
            source_application: SourceApplication {
                name: legacy.source_app.clone(),
                path: None,
            },
        };
        let hashed = hash_clipboard(&captured)?;
        if hashed.content_hash.as_slice() != expected_hash {
            return Err(MigrationError::Invalid(
                "暂存内容与分析时的哈希不一致".to_owned(),
            ));
        }
        let preview = generate_preview(&captured)?;
        let icon = icon_id
            .map(|id| -> Result<_, MigrationError> {
                let (bytes, sha256, relative_path, width, height) = self.connection.query_row(
                    "SELECT bytes, sha256, relative_path, width, height FROM icons WHERE id = ?1",
                    [id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get::<_, Vec<u8>>(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )?;
                let sha256 = sha256
                    .try_into()
                    .map_err(|_| MigrationError::Invalid("暂存图标哈希无效".to_owned()))?;
                Ok(LegacyIcon {
                    bytes,
                    sha256,
                    relative_path,
                    width,
                    height,
                })
            })
            .transpose()?;
        Ok(PreparedRecord {
            legacy,
            captured,
            preview,
            hashed,
            source_key,
            icon,
        })
    }

    pub(super) fn for_each_record(
        &self,
        options: MigrationOptions,
        mut visit: impl FnMut(PreparedRecord) -> Result<(), MigrationError>,
    ) -> Result<(), MigrationError> {
        let mut statement = self.connection.prepare(
            "SELECT id FROM records
            WHERE (kind = 'clipboard' AND ?1) OR (kind = 'favorite' AND ?2) ORDER BY id",
        )?;
        let mut rows = statement.query(params![options.history, options.favorites])?;
        while let Some(row) = rows.next()? {
            visit(self.load_record(row.get(0)?)?)?;
        }
        Ok(())
    }
}
