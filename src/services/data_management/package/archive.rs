use super::{Manifest, Result, Scope, invalid};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use crypto_box::aead::{OsRng, rand_core::RngCore};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};
use zeroize::Zeroizing;

const MAGIC: &[u8] = b"QCCLIPPKG2\n";
const CHUNK: usize = 1024 * 1024;
const MAX_TOTAL: u64 = 64 * 1024 * 1024 * 1024;

pub(super) fn is_encrypted(path: &Path) -> Result<bool> {
    let mut file = File::open(path)?;
    let mut header = [0; 12];
    file.read_exact(&mut header)?;
    if header.starts_with(super::super::MAGIC) {
        return Ok(false);
    }
    if !header.starts_with(MAGIC) || header[11] > 1 {
        return invalid("无法识别数据包格式");
    }
    Ok(header[11] == 1)
}

fn key(password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    let mut key = Zeroizing::new([0; 32]);
    let params = Params::new(64 * 1024, 3, 1, Some(32))
        .map_err(|_| super::Error::Message("密码派生参数无效".into()))?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|_| super::Error::Message("数据包密码处理失败".into()))?;
    Ok(key)
}

pub(super) fn write(
    root: &Path,
    manifest: &Manifest,
    path: &Path,
    password: Option<&str>,
) -> Result<()> {
    let mut plain = tempfile::tempfile()?;
    {
        let mut output = zstd::stream::write::Encoder::new(&mut plain, 3)?;
        let json = serde_json::to_vec(manifest)?;
        output.write_all(&(json.len() as u32).to_le_bytes())?;
        output.write_all(&json)?;
        let files = super::super::managed_files(root)?;
        let files: Vec<_> = files
            .into_iter()
            .filter(|p| {
                !matches!(
                    p.file_name().and_then(|n| n.to_str()),
                    Some("qcclip.sqlite3-wal" | "qcclip.sqlite3-shm")
                )
            })
            .collect();
        if files.len() > 1_000_000 {
            return invalid("数据包文件数量过多");
        }
        output.write_all(&(files.len() as u32).to_le_bytes())?;
        let mut total = 0u64;
        for file in files {
            let name = file
                .strip_prefix(root)
                .map_err(|_| super::Error::Message("数据包文件路径无效".into()))?
                .to_string_lossy()
                .replace('\\', "/");
            let length = fs::metadata(&file)?.len();
            total = total
                .checked_add(length)
                .ok_or_else(|| super::Error::Message("数据包大小溢出".into()))?;
            if length > super::super::MAX_ENTRY_BYTES || total > MAX_TOTAL {
                return invalid("数据包超过支持的大小");
            }
            output.write_all(&(name.len() as u32).to_le_bytes())?;
            output.write_all(&length.to_le_bytes())?;
            output.write_all(&super::super::hash_file(&file)?)?;
            output.write_all(name.as_bytes())?;
            io::copy(&mut File::open(file)?, &mut output)?;
        }
        output.finish()?;
    }
    use std::io::{Seek, SeekFrom};
    plain.seek(SeekFrom::Start(0))?;
    let mut output = atomic_write_file::AtomicWriteFile::open(path)?;
    output.write_all(MAGIC)?;
    output.write_all(&[u8::from(password.is_some())])?;
    if let Some(password) = password {
        let mut salt = [0; 16];
        let mut prefix = [0; 16];
        OsRng.fill_bytes(&mut salt);
        OsRng.fill_bytes(&mut prefix);
        output.write_all(&salt)?;
        output.write_all(&prefix)?;
        let derived = key(password, &salt)?;
        let cipher = XChaCha20Poly1305::new_from_slice(derived.as_ref())
            .map_err(|_| super::Error::Message("数据包密钥无效".into()))?;
        let header = [MAGIC, &[1], &salt, &prefix].concat();
        let mut index = 0u64;
        let mut buffer = Zeroizing::new(vec![0; CHUNK]);
        loop {
            let length = plain.read(&mut buffer)?;
            let final_chunk = length == 0;
            let mut nonce = [0; 24];
            nonce[..16].copy_from_slice(&prefix);
            nonce[16..].copy_from_slice(&index.to_le_bytes());
            let aad = [&header[..], &index.to_le_bytes(), &[u8::from(final_chunk)]].concat();
            let encrypted = cipher
                .encrypt(
                    XNonce::from_slice(&nonce),
                    Payload {
                        msg: &buffer[..length],
                        aad: &aad,
                    },
                )
                .map_err(|_| super::Error::Message("数据包加密失败".into()))?;
            output.write_all(&(encrypted.len() as u32).to_le_bytes())?;
            output.write_all(&encrypted)?;
            if final_chunk {
                break;
            }
            index += 1;
        }
    } else {
        io::copy(&mut plain, &mut output)?;
    }
    output.as_file().sync_all()?;
    output.commit()?;
    Ok(())
}

pub(super) fn read(path: &Path, root: &Path, password: Option<&str>) -> Result<Manifest> {
    let mut file = File::open(path)?;
    let mut magic = vec![0; MAGIC.len() + 1];
    file.read_exact(&mut magic)?;
    if magic == super::super::MAGIC {
        super::super::extract_archive(path, root)?;
        crate::services::database::Database::open(root.join("qcclip.sqlite3"))?;
        return super::snapshot::manifest(
            root,
            Scope {
                history: true,
                favorites: true,
                settings: root.join("config.json").is_file(),
                gallery: root.join("image_library").is_dir(),
            },
            false,
        );
    }
    if !magic.starts_with(MAGIC) {
        return invalid("无法识别数据包格式");
    }
    let mode = [magic[MAGIC.len()]];
    let mut compressed = tempfile::tempfile()?;
    match mode[0] {
        0 => {
            io::copy(&mut file.take(MAX_TOTAL + 1), &mut compressed)?;
        }
        1 => {
            let password = password
                .filter(|p| !p.is_empty())
                .ok_or_else(|| super::Error::Message("请输入此数据包的密码".into()))?;
            let mut salt = [0; 16];
            let mut prefix = [0; 16];
            file.read_exact(&mut salt)?;
            file.read_exact(&mut prefix)?;
            let derived = key(password, &salt)?;
            let cipher = XChaCha20Poly1305::new_from_slice(derived.as_ref())
                .map_err(|_| super::Error::Message("数据包密钥无效".into()))?;
            let header = [MAGIC, &[1], &salt, &prefix].concat();
            let mut index = 0u64;
            let mut total = 0u64;
            loop {
                let length = super::super::read_u32(&mut file)? as usize;
                if !(16..=CHUNK + 16).contains(&length) {
                    return invalid("数据包加密分块无效");
                }
                let final_chunk = length == 16;
                let mut encrypted = vec![0; length];
                file.read_exact(&mut encrypted)?;
                let mut nonce = [0; 24];
                nonce[..16].copy_from_slice(&prefix);
                nonce[16..].copy_from_slice(&index.to_le_bytes());
                let aad = [&header[..], &index.to_le_bytes(), &[u8::from(final_chunk)]].concat();
                let plain = Zeroizing::new(
                    cipher
                        .decrypt(
                            XNonce::from_slice(&nonce),
                            Payload {
                                msg: &encrypted,
                                aad: &aad,
                            },
                        )
                        .map_err(|_| super::Error::Message("密码错误或数据包已损坏".into()))?,
                );
                total += plain.len() as u64;
                if total > MAX_TOTAL {
                    return invalid("数据包超过支持的大小");
                }
                compressed.write_all(&plain)?;
                if final_chunk {
                    break;
                }
                index += 1;
            }
            let mut extra = [0];
            if file.read(&mut extra)? != 0 {
                return invalid("加密数据包包含多余内容");
            }
        }
        _ => return invalid("不支持的数据包加密方式"),
    }
    if compressed.metadata()?.len() > MAX_TOTAL {
        return invalid("数据包超过支持的大小");
    }
    use std::io::{Seek, SeekFrom};
    compressed.seek(SeekFrom::Start(0))?;
    let mut input = zstd::stream::read::Decoder::new(compressed)?;
    let json_length = super::super::read_u32(&mut input)? as usize;
    if json_length > 1024 * 1024 {
        return invalid("数据包清单过大");
    }
    let mut json = vec![0; json_length];
    input.read_exact(&mut json)?;
    let manifest: Manifest = serde_json::from_slice(&json)?;
    if manifest.version != 2 || !manifest.scope.any() {
        return invalid("不支持的数据包版本或数据范围");
    }
    let count = super::super::read_u32(&mut input)?;
    if count > 1_000_000 {
        return invalid("数据包文件数量过多");
    }
    let mut paths = std::collections::HashSet::new();
    let mut total = 0u64;
    for _ in 0..count {
        let name_length = super::super::read_u32(&mut input)? as usize;
        let length = super::super::read_u64(&mut input)?;
        total = total
            .checked_add(length)
            .ok_or_else(|| super::Error::Message("数据包大小溢出".into()))?;
        if name_length == 0
            || name_length > 32768
            || length > super::super::MAX_ENTRY_BYTES
            || total > MAX_TOTAL
        {
            return invalid("数据包文件大小无效");
        }
        let mut hash = [0; 32];
        input.read_exact(&mut hash)?;
        let mut name = vec![0; name_length];
        input.read_exact(&mut name)?;
        let name = String::from_utf8(name)
            .map_err(|_| super::Error::Message("数据包路径编码无效".into()))?;
        if name.contains(['\\', '\0', ':'])
            || name.split('/').any(|part| {
                part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' '])
            })
            || !paths.insert(name.to_lowercase())
        {
            return invalid("数据包路径重复或无效");
        }
        let target = super::super::safe_join(root, &name)?;
        if !allowed(&name) {
            return invalid("数据包包含不支持的文件");
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        let actual = super::super::copy_file_contents(&mut input, &mut output, length)?;
        if actual != hash {
            return invalid("数据包文件校验失败");
        }
    }
    let mut extra = [0];
    if input.read(&mut extra)? != 0 {
        return invalid("数据包包含多余内容");
    }
    Ok(manifest)
}

fn allowed(name: &str) -> bool {
    name == "qcclip.sqlite3"
        || name == "config.json"
        || name == "state.json"
        || name.starts_with("data/")
        || name.starts_with("image_library/")
        || name.starts_with("sound/")
}
