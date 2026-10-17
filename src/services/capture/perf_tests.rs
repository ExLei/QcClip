use std::{hint::black_box, time::Instant};

use rusqlite::params;
use serde_json::{Value, json};

use super::{CaptureService, hash_clipboard};
use crate::{
    domain::capture::{CapturedClipboard, FormatKind, RawFormat},
    services::{database::Database, storage::RawBlobStore},
};

fn plain(value: &str) -> CapturedClipboard {
    CapturedClipboard {
        formats: vec![RawFormat::captured(
            "windows",
            "CF_UNICODETEXT",
            "CF_UNICODETEXT",
            FormatKind::Text,
            value
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect(),
            0,
            true,
        )],
        primary_format_identity: Some(("windows".to_owned(), "CF_UNICODETEXT".to_owned())),
        preview_candidate: None,
        source_application: Default::default(),
    }
}

fn rich(value: &str, variant: usize, private_bytes: usize) -> CapturedClipboard {
    let mut captured = plain(value);
    captured.formats.push(RawFormat::captured(
        "windows",
        "registered:HTML Format",
        "HTML Format",
        FormatKind::Html,
        format!("<span style='color:rgb({variant},0,0)'>{value}</span>").into_bytes(),
        1,
        true,
    ));
    for index in 0..23 {
        captured.formats.push(RawFormat::captured(
            "windows",
            format!("registered:基准格式{index}"),
            format!("基准格式{index}"),
            FormatKind::Custom,
            vec![index as u8; private_bytes],
            captured.formats.len() as u32,
            true,
        ));
    }
    captured
}

fn seed(database: &Database, count: usize) {
    let mut connection = database.connect_write().expect("无法连接性能测试库");
    let transaction = connection.transaction().expect("无法创建准备事务");
    for index in 0..count {
        let value = format!("基准历史{index:08} {}", "abcdefghijklmnop".repeat(32));
        let captured = plain(&value);
        let hashed = hash_clipboard(&captured).expect("无法计算准备数据哈希");
        transaction.prepare(
            "INSERT INTO clipboard_items (uuid,hash_version,content_hash,content_kind,created_at,updated_at)
             VALUES (?1,2,?2,'text',1,1)"
        ).expect("无法准备内容语句").execute(params![format!("种子内容{index}"), hashed.content_hash.as_slice()]).expect("无法插入准备内容");
        let item_id = transaction.last_insert_rowid();
        transaction.prepare(
            "INSERT INTO clipboard_formats (uuid,item_id,format_order,format_namespace,format_key,format_name,format_kind,
                is_primary,storage_kind,inline_data,byte_length,data_hash,restore_state,capture_state,preview_text,preview_char_count,preview_content_kind,created_at)
             VALUES (?1,?2,0,'windows','CF_UNICODETEXT','CF_UNICODETEXT','text',1,'inline',?3,?4,?5,'restorable','captured',?6,?7,'text',1)"
        ).expect("无法准备格式语句").execute(params![format!("种子格式{index}"),item_id,captured.formats[0].bytes,
            captured.formats[0].bytes.as_ref().expect("缺少准备字节").len() as i64,hashed.format_hashes[0].as_ref().expect("缺少格式哈希").as_slice(),value,value.chars().count() as i64]).expect("无法插入准备格式");
        transaction.prepare(
            "UPDATE clipboard_items SET capture_state='ready',format_count=1,total_bytes=?1,preview_text=?2,preview_char_count=?3 WHERE id=?4"
        ).expect("无法准备内容统计语句").execute(params![captured.formats[0].bytes.as_ref().expect("缺少准备字节").len() as i64,value,value.chars().count() as i64,item_id]).expect("无法完成准备内容");
        transaction.prepare(
            "INSERT INTO clipboard_history (uuid,item_id,first_seen_at,last_seen_at,sort_order,updated_at) VALUES (?1,?2,1,1,?2,1)"
        ).expect("无法准备历史语句").execute(params![format!("种子历史{index}"),item_id]).expect("无法插入准备历史");
    }
    transaction.commit().expect("无法完成准备事务");
}

fn measure(mut operation: impl FnMut(usize), count: usize) -> Value {
    let mut elapsed = Vec::with_capacity(count);
    for index in 0..count + 5 {
        let started = Instant::now();
        operation(index);
        let millis = started.elapsed().as_secs_f64() * 1000.0;
        if index >= 5 {
            elapsed.push(millis);
        }
    }
    elapsed.sort_by(f64::total_cmp);
    json!({"samples":count,"mean_ms":elapsed.iter().sum::<f64>()/count as f64,
        "p50_ms":elapsed[count / 2],"p95_ms":elapsed[(count * 95 / 100).min(count-1)],"max_ms":elapsed[count-1]})
}

#[test]
#[ignore = "手动评估入库耗时，创建临时数据库，不操作真实历史"]
fn capture_storage_latency() {
    let sizes = std::env::var("QCCLIP_CAPTURE_BENCH_SIZES")
        .unwrap_or_else(|_| "1000,10000,50000".to_owned());
    let samples: usize = std::env::var("QCCLIP_CAPTURE_BENCH_SAMPLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(40);
    assert!(samples > 0, "样本数量必须大于零");
    let mut results = Vec::new();
    for size in sizes
        .split(',')
        .map(|value| value.parse::<usize>().expect("历史数量无效"))
    {
        let directory = tempfile::tempdir().expect("无法创建性能测试目录");
        let database =
            Database::open(directory.path().join("perf.sqlite3")).expect("无法创建性能测试库");
        let blobs = RawBlobStore::new(directory.path()).expect("无法创建性能测试文件存储");
        let service = CaptureService::new(database.clone(), blobs);
        seed(&database, size);
        for merge in [false, true] {
            // 仅在临时库切换版本，利用兼容保护关闭同文合并，作为固定成本的近似对照。
            database
                .connect_write()
                .expect("无法连接性能测试库")
                .pragma_update(None, "user_version", if merge { 1 } else { 2 })
                .expect("无法设置参考路径");
            let name = format!("{size}-{merge}");
            let mut record = |scenario: &str, stats: Value| {
                let row = json!({"history":size,"merge":merge,"scenario":scenario,"timing":stats});
                println!("{row}");
                results.push(row);
            };
            record(
                "连接与结构校验",
                measure(
                    |_| {
                        black_box(database.connect_write().expect("无法连接测试库"));
                    },
                    samples,
                ),
            );
            let new_captures: Vec<_> = (0..samples + 5)
                .map(|index| plain(&format!("新文本-{name}-{index}")))
                .collect();
            record(
                "新短文本",
                measure(
                    |index| {
                        black_box(
                            service
                                .store_captured_clipboard(&new_captures[index])
                                .expect("无法保存新文本"),
                        );
                    },
                    samples,
                ),
            );
            let repeat = plain(&format!("重复-{name}"));
            service
                .store_captured_clipboard(&repeat)
                .expect("无法准备重复文本");
            record(
                "重复短文本",
                measure(
                    |_| {
                        black_box(
                            service
                                .store_captured_clipboard(&repeat)
                                .expect("无法保存重复文本"),
                        );
                    },
                    samples,
                ),
            );
            let long_prefix = "abcdefgh".repeat(8192);
            let long_captures: Vec<_> = (0..samples + 5)
                .map(|index| plain(&format!("{long_prefix}-{name}-{index}")))
                .collect();
            record(
                "新64KiB文本",
                measure(
                    |index| {
                        black_box(
                            service
                                .store_captured_clipboard(&long_captures[index])
                                .expect("无法保存长文本"),
                        );
                    },
                    samples,
                ),
            );
            record(
                "重复64KiB文本",
                measure(
                    |_| {
                        black_box(
                            service
                                .store_captured_clipboard(&long_captures[0])
                                .expect("无法重复保存长文本"),
                        );
                    },
                    samples,
                ),
            );
            for (label, private_bytes) in [
                ("25格式小内容", 128),
                ("25格式大内容", 256 * 1024),
                ("25格式低压缩率大内容", 128 * 1024),
            ] {
                let value = format!("多格式-{name}-{label}");
                let mut first = rich(&value, 0, private_bytes);
                let mut second = rich(&value, 1, private_bytes);
                if label.contains("低压缩率") {
                    for (index, format) in first.formats.iter_mut().enumerate().skip(2) {
                        let mut state = index as u32;
                        for byte in format.bytes.as_mut().expect("缺少私有格式字节") {
                            state ^= state << 13;
                            state ^= state >> 17;
                            state ^= state << 5;
                            *byte = state as u8;
                        }
                        second.formats[index].bytes = format.bytes.clone();
                    }
                }
                service
                    .store_captured_clipboard(&first)
                    .expect("无法准备多格式内容");
                record(
                    &format!("{label}重复复制"),
                    measure(
                        |_| {
                            black_box(
                                service
                                    .store_captured_clipboard(&first)
                                    .expect("无法重复保存多格式内容"),
                            );
                        },
                        samples,
                    ),
                );
                let plain = plain(&value);
                service
                    .store_captured_clipboard(&plain)
                    .expect("无法准备纯文本补充");
                record(
                    &format!("{label}后复制纯文本"),
                    measure(
                        |_| {
                            black_box(
                                service
                                    .store_captured_clipboard(&plain)
                                    .expect("无法保存纯文本补充"),
                            );
                        },
                        samples,
                    ),
                );
                record(
                    &format!("{label}更改HTML"),
                    measure(
                        |index| {
                            black_box(
                                service
                                    .store_captured_clipboard(if index % 2 == 0 {
                                        &first
                                    } else {
                                        &second
                                    })
                                    .expect("无法更新格式内容"),
                            );
                        },
                        samples,
                    ),
                );
            }
            if merge {
                database.connect_write().expect("无法连接测试库").execute_batch(
                    "UPDATE clipboard_formats SET preview_text=NULL,preview_char_count=NULL,preview_content_kind=NULL
                     WHERE item_id IN (SELECT id FROM clipboard_items ORDER BY id LIMIT 32);"
                ).expect("无法准备旧记录回退场景");
                let captures: Vec<_> = (0..samples + 5)
                    .map(|index| plain(&format!("旧记录回退-{name}-{index}")))
                    .collect();
                record(
                    "32条旧记录缺少预览时新短文本",
                    measure(
                        |index| {
                            black_box(
                                service
                                    .store_captured_clipboard(&captures[index])
                                    .expect("无法验证旧记录回退"),
                            );
                        },
                        samples,
                    ),
                );
            }
        }
    }
    if let Ok(output) = std::env::var("QCCLIP_CAPTURE_BENCH_OUTPUT") {
        std::fs::write(
            output,
            serde_json::to_vec_pretty(&results).expect("无法编码性能结果"),
        )
        .expect("无法保存性能结果");
    }
}
