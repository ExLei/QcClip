use std::{
    collections::{HashMap, HashSet},
    io::Write,
    path::PathBuf,
};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

use super::{
    Result,
    config::{Config, MAX_EVENT_BYTES, Peer},
    fail,
    protocol::{self, Envelope, Event},
    repository::Packet,
    webdav::Client,
};

pub(super) const MAX_INDEX_BYTES: usize = 4 * 1024 * 1024;
const INLINE_BYTES: usize = 16 * 1024;
const FORMAT: &str = "qcclip-cache-v1";
const ENTRY_PURPOSE: &str = "webdav-cache-entry-v1";
const OBJECT_PURPOSE: &str = "webdav-cache-object-v1";

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct ResourceData {
    pub id: String,
    #[serde(with = "protocol::bytes")]
    pub data: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Entry {
    pub event: Event,
    pub object: Option<String>,
    pub resources: Vec<ResourceData>,
}

#[derive(Serialize, Deserialize)]
struct Bundle {
    event: String,
    resources: Vec<ResourceData>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Slot {
    raw_size: usize,
    object: Option<String>,
    envelope: Envelope,
}
impl Slot {
    fn validate(&self) -> Result<()> {
        if self.raw_size > MAX_EVENT_BYTES
            || self
                .object
                .as_ref()
                .is_some_and(|id| Uuid::parse_str(id).is_err())
        {
            return fail("WebDAV 缓存条目格式或容量无效");
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Index<T = Slot> {
    format: String,
    entries: Vec<T>,
}
impl Default for Index {
    fn default() -> Self {
        Self {
            format: FORMAT.into(),
            entries: Vec::new(),
        }
    }
}
impl Index {
    fn decode(bytes: &[u8]) -> Result<Self> {
        let index: Self = decode_index(bytes)?;
        for slot in &index.entries {
            slot.validate()?;
        }
        Ok(index)
    }
}

fn decode_index<T: DeserializeOwned>(bytes: &[u8]) -> Result<Index<T>> {
    let index: Index<T> = serde_json::from_slice(bytes)?;
    if index.format != FORMAT || index.entries.len() > 1000 {
        return fail("WebDAV 缓存索引格式或容量无效，请确认两台设备均使用同一版本");
    }
    Ok(index)
}

pub(super) fn device_key(key: &[u8; 32]) -> String {
    protocol::route(key, key)
}
pub(super) fn index_path(key: &[u8; 32]) -> String {
    format!("indexes/{}", device_key(key))
}
pub(super) fn object_path(key: &[u8; 32], id: &str) -> Result<String> {
    if Uuid::parse_str(id).is_err() {
        return fail("WebDAV 缓存资源标识无效");
    }
    Ok(format!("objects/{}-{id}", device_key(key)))
}
pub(super) fn scope(config: &Config) -> String {
    format!(
        "{}|{}|{}",
        config.dav.url.trim_end_matches('/'),
        config.dav.username,
        device_key(&config.profile.key)
    )
}

pub(super) struct Prepared {
    slot: Slot,
    pub object: Option<(String, Vec<u8>)>,
}

pub(super) fn prepare(config: &Config, peer: &Peer, packet: &Packet) -> Result<Prepared> {
    let raw_size = packet.event.raw_size();
    if raw_size > config.dav.cache_mb * 1024 * 1024 {
        return fail("条目超过 WebDAV 缓存总容量，请调大缓存或选择较小条目");
    }
    let mut entry = Entry {
        event: packet.event.clone(),
        object: None,
        resources: packet
            .resources
            .iter()
            .map(|(id, data)| ResourceData {
                id: id.clone(),
                data: data.clone(),
            })
            .collect(),
    };
    entry.event.session = "webdav-cache-v1".into();
    entry.event.expires = 0;
    entry
        .event
        .validate_cache(&config.profile.id, &peer.profile.id)?;
    let object = if raw_size > INLINE_BYTES {
        let id = Uuid::new_v4().to_string();
        entry.object = Some(id.clone());
        let bundle = Bundle {
            event: entry.event.id.clone(),
            resources: std::mem::take(&mut entry.resources),
        };
        Some((
            id,
            serde_json::to_vec(&protocol::seal_cache(
                config,
                peer,
                &bundle,
                OBJECT_PURPOSE,
            )?)?,
        ))
    } else {
        None
    };
    let slot = Slot {
        raw_size,
        object: entry.object.clone(),
        envelope: protocol::seal_cache(config, peer, &entry, ENTRY_PURPOSE)?,
    };
    if serde_json::to_vec(&slot)?.len() + 128 > MAX_INDEX_BYTES {
        return fail("条目元数据超过 WebDAV 索引大小上限");
    }
    Ok(Prepared { slot, object })
}

#[derive(Default)]
pub(super) struct Cache {
    scope: String,
    index: Index,
    loaded: bool,
    garbage: HashSet<String>,
    journal: PathBuf,
    inflight: HashSet<String>,
}

impl Cache {
    fn save_journal(&self) -> Result<()> {
        if let Some(parent) = self.journal.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = AtomicWriteFile::open(&self.journal)?;
        file.write_all(&serde_json::to_vec(&self.garbage)?)?;
        file.commit()?;
        Ok(())
    }
    fn cleanup(&mut self, client: &Client, config: &Config) {
        let live: HashSet<_> = self
            .index
            .entries
            .iter()
            .filter_map(|slot| slot.object.clone())
            .collect();
        let pending: Vec<_> = self.garbage.iter().cloned().collect();
        for id in pending {
            if self.inflight.contains(&id) {
                continue;
            }
            if live.contains(&id) {
                self.garbage.remove(&id);
                continue;
            }
            let result =
                object_path(&config.profile.key, &id).and_then(|path| client.delete(&path));
            match result {
                Ok(()) => {
                    self.garbage.remove(&id);
                }
                Err(error) => {
                    tracing::warn!("清理 WebDAV 缓存资源失败：{error}");
                    break;
                }
            }
        }
        if let Err(error) = self.save_journal() {
            tracing::warn!("保存 WebDAV 缓存清理记录失败：{error}");
        }
    }
    fn load(&mut self, client: &Client, config: &Config, journal: PathBuf) -> Result<()> {
        let scope = scope(config);
        if self.scope != scope {
            let garbage = if journal.exists() {
                serde_json::from_slice(&std::fs::read(&journal)?)?
            } else {
                HashSet::new()
            };
            *self = Self {
                scope,
                garbage,
                journal,
                ..Self::default()
            };
        }
        client.initialize()?;
        if !self.loaded {
            self.index = match client.get(&index_path(&config.profile.key), MAX_INDEX_BYTES)? {
                Some(bytes) => Index::decode(&bytes)?,
                None => Index::default(),
            };
            self.loaded = true;
        }
        Ok(())
    }
    pub fn stage(
        &mut self,
        client: &Client,
        config: &Config,
        prepared: &Prepared,
        journal: PathBuf,
    ) -> Result<()> {
        self.load(client, config, journal)?;
        if let Some((id, _)) = &prepared.object {
            self.garbage.insert(id.clone());
            self.inflight.insert(id.clone());
            if let Err(error) = self.save_journal() {
                self.inflight.remove(id);
                return Err(error);
            }
        }
        Ok(())
    }
    pub fn abandon(&mut self, config: &Config, prepared: &Prepared) {
        if self.scope == scope(config)
            && let Some((id, _)) = &prepared.object
        {
            self.inflight.remove(id);
        }
    }
    pub fn publish(
        &mut self,
        client: &Client,
        config: &Config,
        prepared: &Prepared,
        journal: PathBuf,
        ensure_current: impl Fn() -> Result<()>,
    ) -> Result<()> {
        self.load(client, config, journal)?;
        let mut next = self.index.clone();
        next.entries.push(prepared.slot.clone());
        // 每个设备只修改自己的索引；按实际上传顺序淘汰，无需读取其他设备的目录。
        let mut removed = Vec::new();
        loop {
            let over_capacity = next.entries.len() > config.dav.cache_entries
                || next
                    .entries
                    .iter()
                    .map(|slot| slot.raw_size as u64)
                    .sum::<u64>()
                    > (config.dav.cache_mb * 1024 * 1024) as u64;
            if !over_capacity && serde_json::to_vec(&next)?.len() <= MAX_INDEX_BYTES {
                break;
            }
            if next.entries.len() <= 1 {
                return fail("条目元数据超过 WebDAV 索引大小上限");
            }
            if let Some(id) = next.entries.remove(0).object {
                removed.push(id);
            }
        }
        self.garbage.extend(removed);
        self.save_journal()?;
        let result = (|| {
            ensure_current()?;
            client.put(
                &index_path(&config.profile.key),
                &serde_json::to_vec(&next)?,
            )
        })();
        if let Err(error) = result {
            // 响应丢失时不能假定上传未成功，下次操作先重新读取权威索引再清理。
            self.loaded = false;
            return Err(error);
        }
        self.index = next;
        self.abandon(config, prepared);
        if !self.garbage.is_empty() {
            self.cleanup(client, config);
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct ReadOutcome {
    pub entries: Vec<Entry>,
    pub rejected: usize,
    pub first_error: Option<super::Error>,
}

pub(super) fn read(client: &Client, config: &Config, sender: &Peer) -> Result<ReadOutcome> {
    let Some(bytes) = client.get(&index_path(&sender.profile.key), MAX_INDEX_BYTES)? else {
        return Ok(ReadOutcome::default());
    };
    // 索引外层仍需完整有效；单条的结构、解密和版本错误分别隔离。
    let index: Index<serde_json::Value> = decode_index(&bytes)?;
    let route = protocol::route(&sender.profile.key, &config.profile.key);
    let mut outcome = ReadOutcome::default();
    for (position, value) in index.entries.into_iter().enumerate() {
        if value
            .get("envelope")
            .and_then(|envelope| envelope.get("sender"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|recipient| recipient != route)
        {
            continue;
        }
        let decoded = (|| -> Result<Entry> {
            let slot: Slot = serde_json::from_value(value)?;
            slot.validate()?;
            let (peer, entry): (_, Entry) =
                protocol::open_cache(config, &slot.envelope, ENTRY_PURPOSE)?;
            entry
                .event
                .validate_cache(&sender.profile.id, &config.profile.id)?;
            if peer.profile.key != sender.profile.key
                || slot.raw_size != entry.event.raw_size()
                || slot.object != entry.object
                || (entry.object.is_some() && !entry.resources.is_empty())
            {
                return fail("WebDAV 索引与加密条目不一致");
            }
            Ok(entry)
        })();
        match decoded {
            Ok(entry) => outcome.entries.push(entry),
            Err(error) => {
                if let super::Error::Json(source) = &error {
                    tracing::warn!(条目位置 = position + 1, 错误类别 = ?source.classify(), 行 = source.line(), 列 = source.column(), "跳过无法解析的 WebDAV 缓存条目，请检查两端版本或重新上传");
                } else {
                    tracing::warn!(
                        条目位置 = position + 1,
                        "跳过无效的 WebDAV 缓存条目：{error}"
                    );
                }
                outcome.rejected += 1;
                if outcome.first_error.is_none() {
                    outcome.first_error = Some(error);
                }
            }
        }
    }
    outcome.entries.sort_by_key(|entry| entry.event.sequence);
    Ok(outcome)
}

pub(super) fn resources(
    client: &Client,
    config: &Config,
    peer: &Peer,
    entry: &Entry,
) -> Result<HashMap<String, Vec<u8>>> {
    let resources = if let Some(id) = &entry.object {
        let Some(bytes) = client.get(
            &object_path(&peer.profile.key, id)?,
            entry
                .event
                .raw_size()
                .saturating_mul(2)
                .saturating_add(1024 * 1024),
        )?
        else {
            return fail("WebDAV 缓存资源已被淘汰或缺失，可重新上传该条目");
        };
        let envelope: Envelope = serde_json::from_slice(&bytes)?;
        let (sender, bundle): (_, Bundle) =
            protocol::open_cache(config, &envelope, OBJECT_PURPOSE)?;
        if sender.profile.key != peer.profile.key || bundle.event != entry.event.id {
            return fail("WebDAV 缓存资源与条目身份不匹配");
        }
        bundle.resources
    } else {
        entry.resources.clone()
    };
    let expected: HashMap<_, _> = entry
        .event
        .formats
        .iter()
        .map(|f| &f.resource)
        .chain(entry.event.files.iter().map(|f| &f.resource))
        .map(|r| (r.id.as_str(), r))
        .collect();
    let mut result = HashMap::new();
    for resource in resources {
        let Some(descriptor) = expected.get(resource.id.as_str()) else {
            return fail("WebDAV 资源未在条目中声明");
        };
        if resource.data.len() != descriptor.length
            || protocol::hash(&resource.data) != descriptor.hash
            || result.insert(resource.id, resource.data).is_some()
        {
            return fail("WebDAV 资源长度、哈希或身份无效");
        }
    }
    if result.len() != expected.len() {
        return fail("WebDAV 条目资源不完整");
    }
    Ok(result)
}
