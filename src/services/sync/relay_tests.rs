use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use super::{
    config::DavConfig,
    protocol::{self, Mode, Request},
    tests::{Fixture, paired, runtime},
};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

type Failure = Arc<Mutex<Option<(String, String, u16, bool)>>>;

struct Relay {
    url: String,
    objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    failure: Failure,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    agent: ureq::Agent,
}

impl Relay {
    fn new() -> Self {
        let certified =
            rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])
                .unwrap();
        let cert = certified.cert.der().clone();
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der());
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let server = ServerConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![cert.clone()], key.into())
            .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert).unwrap();
        let tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let agent = ureq::AgentBuilder::new()
            .tls_config(Arc::new(tls))
            .timeout(Duration::from_secs(3))
            .redirects(0)
            .build();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let objects = Arc::new(Mutex::new(BTreeMap::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let request_log = requests.clone();
        let failure = Arc::new(Mutex::new(None));
        let fail_next = failure.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let state = objects.clone();
        let stopped = stop.clone();
        let server = Arc::new(server);
        let thread = thread::spawn(move || {
            let mut workers = Vec::new();
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let server = server.clone();
                        let state = state.clone();
                        let requests = request_log.clone();
                        let failure = fail_next.clone();
                        workers.push(thread::spawn(move || {
                            serve(stream, server, state, requests, failure)
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => break,
                }
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            url: format!("https://127.0.0.1:{port}"),
            objects,
            requests,
            failure,
            stop,
            thread: Some(thread),
            agent,
        }
    }
    fn config(&self) -> DavConfig {
        DavConfig {
            enabled: true,
            url: self.url.clone(),
            username: "test".into(),
            password: "secret".into(),
            poll_seconds: 1,
            ..Default::default()
        }
    }
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn serve(
    stream: TcpStream,
    config: Arc<ServerConfig>,
    objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    failure: Failure,
) {
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut stream = StreamOwned::new(ServerConnection::new(config).unwrap(), stream);
    let mut header = Vec::new();
    loop {
        let mut byte = [0];
        if stream.read_exact(&mut byte).is_err() {
            return;
        }
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
        if header.len() > 16384 {
            return;
        }
    }
    let header = String::from_utf8(header).unwrap();
    let mut lines = header.lines();
    let first = lines.next().unwrap();
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap();
    let path = parts.next().unwrap();
    requests.lock().unwrap().push((method.into(), path.into()));
    let length = lines
        .clone()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|s| s.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    let mut body = vec![0; length];
    if stream.read_exact(&mut body).is_err() {
        return;
    }
    let mut objects = objects.lock().unwrap();
    let fault = {
        let mut failure = failure.lock().unwrap();
        if failure
            .as_ref()
            .is_some_and(|(m, p, _, _)| m == method && p == path)
        {
            failure
                .take()
                .map(|(_, _, status, commit)| (status, commit))
        } else {
            None
        }
    };
    let (status, body) = if let Some((status, false)) = fault {
        (status, Vec::new())
    } else {
        let result = match method {
            "MKCOL" => {
                if objects.contains_key(path) {
                    (405, Vec::new())
                } else {
                    objects.insert(path.into(), Vec::new());
                    (201, Vec::new())
                }
            }
            "PUT" => {
                objects.insert(path.into(), body);
                (201, Vec::new())
            }
            "GET" => match objects.get(path) {
                Some(value) => (200, value.clone()),
                None => (404, Vec::new()),
            },
            "DELETE" => {
                objects.retain(|key, _| {
                    key != path && !key.starts_with(&format!("{}/", path.trim_end_matches('/')))
                });
                (204, Vec::new())
            }
            "PROPFIND" => (207, b"<d:multistatus xmlns:d='DAV:'/>".to_vec()),
            _ => (405, Vec::new()),
        };
        if let Some((status, _)) = fault {
            (status, Vec::new())
        } else {
            result
        }
    };
    drop(objects);
    let retry = if status == 429 || status == 503 {
        "Retry-After: 2\r\n"
    } else {
        ""
    };
    let response = format!(
        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\n{retry}Connection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .expect("测试服务写入响应头失败");
    stream.write_all(&body).expect("测试服务写入响应体失败");
    stream.flush().expect("测试服务刷新失败");
    stream.conn.send_close_notify();
    let _ = stream.flush();
}

#[test]
fn webdav_cache_delivers_text_and_large_resources_without_ack_polling() {
    let relay = Relay::new();
    let source = Fixture::new();
    let target = Fixture::new();
    let (mut a, mut b) = paired();
    a.lan_enabled = false;
    a.dav = relay.config();
    b.dav = relay.config();
    let sender = runtime(&source, a.clone());
    sender.test_relay_agent(relay.agent.clone());
    for text in [
        "缓存第一条".to_owned(),
        "缓存第二条".to_owned(),
        "大资源".repeat(30000),
    ] {
        let item = source.capture(&text);
        sender.upload(None, item, Mode::ManualContent).unwrap();
        sender.test_send_pending().unwrap();
    }
    let requests = relay.requests.lock().unwrap().clone();
    assert_eq!(
        requests.iter().filter(|(m, _)| m == "GET").count(),
        1,
        "首次上传读取自己的索引，后续上传使用本地副本"
    );
    assert_eq!(
        requests.iter().filter(|(m, _)| m == "PUT").count(),
        4,
        "两条小内容各一次 PUT，大内容对象与索引各一次"
    );
    assert!(
        requests
            .iter()
            .all(|(m, p)| m != "PROPFIND" && !p.contains("ack") && !p.contains("leases"))
    );
    let receiver = runtime(&target, b.clone());
    receiver.test_relay_agent(relay.agent.clone());
    relay.requests.lock().unwrap().clear();
    receiver.test_poll_dav().unwrap();
    assert_eq!(target.count("clipboard_history"), 3);
    assert_eq!(
        relay.requests.lock().unwrap().len(),
        2,
        "一个索引 GET 和一个大资源 GET"
    );
    relay.requests.lock().unwrap().clear();
    let restarted = runtime(&target, b);
    restarted.test_relay_agent(relay.agent.clone());
    restarted.test_poll_dav().unwrap();
    assert_eq!(target.count("clipboard_history"), 3);
    assert_eq!(
        relay.requests.lock().unwrap().len(),
        1,
        "重启后不得再次下载已接收资源"
    );
    for bytes in relay.objects.lock().unwrap().values() {
        let text = String::from_utf8_lossy(bytes);
        assert!(
            !text.contains("缓存第一条")
                && !text.contains("缓存第二条")
                && !text.contains("大资源")
        );
    }
}

#[test]
fn short_invitation_connects_both_devices_only_after_separate_confirmation() {
    let relay = Relay::new();
    let a = Fixture::new();
    let b = Fixture::new();
    let mut config_a = super::config::Config::fresh();
    let mut config_b = super::config::Config::fresh();
    config_a.lan_enabled = false;
    config_b.lan_enabled = false;
    config_a.dav = relay.config();
    config_b.dav = relay.config();
    let host = runtime(&a, config_a.clone());
    let guest = runtime(&b, config_b.clone());
    host.test_relay_agent(relay.agent.clone());
    guest.test_relay_agent(relay.agent.clone());
    let invitation = host.create_invitation().unwrap();
    assert_eq!(invitation.code.len(), 19);
    assert!(host.lookup_invitation(&invitation.code).is_err());
    let candidate = guest
        .lookup_invitation(&invitation.code.to_lowercase())
        .unwrap();
    assert_eq!(candidate.device.profile, config_a.profile);
    let guest_verification = candidate.verification(&config_b.profile);
    assert!(host.check_invitation().unwrap().candidate.is_none());
    assert!(host.config().peers.is_empty());
    assert!(guest.config().peers.is_empty());
    guest.confirm_candidate(candidate).unwrap();
    assert_eq!(guest.config().peers.len(), 1);
    assert!(host.config().peers.is_empty());
    assert!(!guest.config().enabled);
    for bytes in relay.objects.lock().unwrap().values() {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains(&config_a.profile.id));
        assert!(!text.contains(&config_b.profile.id));
    }
    let candidate = host.check_invitation().unwrap().candidate.unwrap();
    assert_eq!(candidate.device.profile, config_b.profile);
    assert_eq!(
        candidate.verification(&config_a.profile),
        guest_verification
    );
    host.confirm_candidate(candidate).unwrap();
    assert_eq!(host.config().peers.len(), 1);
    assert!(!host.config().enabled);
    assert!(guest.lookup_invitation(&invitation.code).is_err());
    for device in [&host, &guest] {
        device
            .change(
                |config| {
                    config.enabled = true;
                    Ok(())
                },
                true,
            )
            .unwrap();
    }
    let item = a.capture("短连接码配对后的实际发送");
    assert_eq!(host.manual(None, item, Mode::ManualContent).unwrap(), 1);
    host.test_send_pending().unwrap();
    guest.test_poll_dav().unwrap();
    assert_eq!(b.count("clipboard_history"), 1);
    let invitation = host.create_invitation().unwrap();
    let c = Fixture::new();
    let mut config_c = super::config::Config::fresh();
    config_c.dav = relay.config();
    let third = runtime(&c, config_c);
    third.test_relay_agent(relay.agent.clone());
    let candidate = third.lookup_invitation(&invitation.code).unwrap();
    host.cancel_invitation().unwrap();
    assert!(third.confirm_candidate(candidate).is_err());
    assert!(third.config().peers.is_empty());
}

#[test]
fn webdav_pairing_trust_is_reused_for_lan_after_removing_webdav_configuration() {
    let relay = Relay::new();
    let source = Fixture::new();
    let target = Fixture::new();
    let mut a = super::config::Config::fresh();
    let mut b = super::config::Config::fresh();
    a.lan_enabled = false;
    b.lan_enabled = false;
    a.dav = relay.config();
    b.dav = relay.config();
    let sender = runtime(&source, a.clone());
    let receiver = Arc::new(runtime(&target, b.clone()));
    sender.test_relay_agent(relay.agent.clone());
    receiver.test_relay_agent(relay.agent.clone());
    let invitation = sender.create_invitation().unwrap();
    receiver
        .confirm_candidate(receiver.lookup_invitation(&invitation.code).unwrap())
        .unwrap();
    sender
        .confirm_candidate(sender.check_invitation().unwrap().candidate.unwrap())
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    for device in [&sender, &*receiver] {
        device
            .change(
                |config| {
                    config.enabled = true;
                    config.lan_enabled = true;
                    config.dav = DavConfig::default();
                    config.peers[0].address = address.clone();
                    Ok(())
                },
                true,
            )
            .unwrap();
    }
    for (device, original, peer) in [(&sender, &a, &b), (&*receiver, &b, &a)] {
        let config = device.config();
        assert_eq!(config.profile, original.profile);
        assert_eq!(config.secret, original.secret);
        assert_eq!(config.peers.len(), 1);
        assert_eq!(config.peers[0].profile, peer.profile);
        assert!(config.dav.url.is_empty());
    }
    let relay_before_lan = relay.objects.lock().unwrap().clone();
    assert!(
        relay_before_lan
            .keys()
            .all(|path| { !path.ends_with("-offer") && !path.ends_with("-reply") })
    );
    let session = receiver.test_session();
    let mut packet = source.packet(source.capture("配对身份跨通道复用"), Mode::NewContent, 1);
    packet.event.sender = a.profile.id.clone();
    packet.event.recipient = b.profile.id.clone();
    packet.event.session = session.id.clone();
    let request_count = 2 + packet
        .event
        .formats
        .iter()
        .map(|format| format.resource.chunks)
        .sum::<usize>();
    listener.set_nonblocking(true).unwrap();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(12);
        for _ in 0..request_count {
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "局域网回归接收超时");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("局域网回归接收失败：{error}"),
                }
            };
            super::lan::prepare(&stream).unwrap();
            let envelope = super::lan::read(&mut stream).unwrap();
            let reply = receiver.receive_lan(&envelope).unwrap();
            super::lan::write(&mut stream, &reply).unwrap();
        }
    });
    sender
        .test_relay_send(&sender.config().peers[0], &packet, session)
        .unwrap();
    server.join().unwrap();
    assert_eq!(target.count("clipboard_history"), 1);
    assert_eq!(*relay.objects.lock().unwrap(), relay_before_lan);
    let snapshot = sender.snapshot();
    let config = snapshot.config.unwrap();
    assert_eq!(
        snapshot.statuses[&b.profile.id].channel(&config, &config.peers[0]),
        Some("LAN")
    );
}

#[test]
fn available_lan_delivers_without_webdav_requests() {
    let relay = Relay::new();
    let source = Fixture::new();
    let target = Fixture::new();
    let (mut a, mut b) = paired();
    a.dav = relay.config();
    b.dav = relay.config();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    a.peers[0].address = listener.local_addr().unwrap().to_string();
    let receiver = Arc::new(runtime(&target, b.clone()));
    let sender = runtime(&source, a.clone());
    sender.test_relay_agent(relay.agent.clone());
    let session = receiver.test_session();
    let mut packet = source.packet(source.capture("优先局域网"), Mode::NewContent, 1);
    packet.event.sender = a.profile.id.clone();
    packet.event.recipient = b.profile.id.clone();
    packet.event.session = session.id.clone();
    let request_count = 2 + packet
        .event
        .formats
        .iter()
        .map(|f| f.resource.chunks)
        .sum::<usize>();
    let server = thread::spawn(move || {
        for _ in 0..request_count {
            let (mut stream, _) = listener.accept().unwrap();
            super::lan::prepare(&stream).unwrap();
            let envelope = super::lan::read(&mut stream).unwrap();
            let response = receiver.receive_lan(&envelope).unwrap();
            super::lan::write(&mut stream, &response).unwrap();
        }
    });
    sender
        .test_relay_send(&a.peers[0], &packet, session)
        .unwrap();
    server.join().unwrap();
    assert_eq!(target.count("clipboard_history"), 1);
    assert!(relay.requests.lock().unwrap().is_empty());
}

#[test]
fn webdav_alone_trusts_and_delivers_without_lan() {
    let relay = Relay::new();
    let source = Fixture::new();
    let target = Fixture::new();
    let mut a = super::config::Config::fresh();
    let mut b = super::config::Config::fresh();
    for config in [&mut a, &mut b] {
        config.enabled = true;
        config.lan_enabled = false;
        config.port = 0;
        config.dav = relay.config();
    }
    let code = |config: &super::config::Config| {
        super::config::PairingCode::decode(
            &super::config::PairingCode {
                profile: config.profile.clone(),
                address: String::new(),
            }
            .encode()
            .unwrap(),
        )
        .unwrap()
    };
    let sender = runtime(&source, a.clone());
    let receiver = runtime(&target, b.clone());
    sender.pair(code(&b), String::new()).unwrap();
    receiver.pair(code(&a), String::new()).unwrap();
    sender.test_relay_agent(relay.agent.clone());
    receiver.test_relay_agent(relay.agent.clone());
    for (runtime, peer) in [(&sender, &b.profile.id), (&receiver, &a.profile.id)] {
        let snapshot = runtime.snapshot();
        let config = snapshot.config.unwrap();
        let status = snapshot.statuses.get(peer).cloned().unwrap_or_default();
        assert!(config.peers[0].address.is_empty());
        assert!(status.lan.is_none());
        assert_eq!(status.channel(&config, &config.peers[0]), Some("WebDAV"));
    }
    let item = source.capture("仅通过 WebDAV 发送，无局域网地址与握手");
    assert_eq!(sender.manual(None, item, Mode::ManualContent).unwrap(), 1);
    sender.test_send_pending().unwrap();
    receiver.test_poll_dav().unwrap();
    assert_eq!(target.count("clipboard_history"), 1);
}

#[test]
fn webdav_idle_poll_is_one_get_per_sender() {
    let relay = Relay::new();
    let target = Fixture::new();
    let (_, mut b) = paired();
    b.dav = relay.config();
    let receiver = runtime(&target, b);
    receiver.test_relay_agent(relay.agent.clone());
    receiver.test_poll_dav().unwrap();
    let requests = relay.requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "空闲拉取不得维护租约、建目录或扫描邮箱");
    assert_eq!(requests[0].0, "GET");
    drop(requests);
    receiver.test_dav().unwrap();
}

#[test]
fn webdav_manual_upload_does_not_require_receiver_online() {
    let relay = Relay::new();
    let source = Fixture::new();
    let target = Fixture::new();
    let (mut a, mut b) = paired();
    a.lan_enabled = false;
    a.dav = relay.config();
    b.dav = relay.config();
    let sender = runtime(&source, a);
    sender.test_relay_agent(relay.agent.clone());
    let item = source.capture("离线设备稍后拉取的内容");
    assert_eq!(sender.manual(None, item, Mode::ManualContent).unwrap(), 1);
    sender.test_send_pending().unwrap();
    assert_eq!(target.count("clipboard_history"), 0);
    let receiver = runtime(&target, b);
    receiver.test_relay_agent(relay.agent.clone());
    receiver.test_poll_dav().unwrap();
    assert_eq!(target.count("clipboard_history"), 1);
    relay.requests.lock().unwrap().clear();
    receiver.test_poll_dav().unwrap();
    assert_eq!(target.count("clipboard_history"), 1);
    assert_eq!(relay.requests.lock().unwrap().len(), 1);
}

#[test]
fn webdav_latest_upload_ignores_pinning_and_manual_order() {
    let relay = Relay::new();
    let source = Fixture::new();
    let target = Fixture::new();
    let (mut a, mut b) = paired();
    a.dav = relay.config();
    a.dav.auto_upload = false;
    b.dav = relay.config();
    let sender = runtime(&source, a);
    sender.test_relay_agent(relay.agent.clone());
    assert_eq!(sender.upload_latest().unwrap(), None);
    assert!(relay.requests.lock().unwrap().is_empty());

    let older = source.capture("置顶且排在前面的旧内容");
    let latest = source.capture("最近复制的内容");
    let connection = source.db.connect().unwrap();
    connection
        .execute(
            "UPDATE clipboard_history SET first_seen_at = 1, last_seen_at = 10 WHERE item_id = ?1",
            [latest],
        )
        .unwrap();
    connection.execute(
        "UPDATE clipboard_history SET first_seen_at = 1, last_seen_at = 2, is_pinned = 1 WHERE item_id = ?1",
        [older],
    ).unwrap();
    assert_eq!(sender.upload_latest().unwrap(), Some(1));
    sender.test_send_pending().unwrap();

    let receiver = runtime(&target, b);
    receiver.test_relay_agent(relay.agent.clone());
    receiver.test_poll_dav().unwrap();
    assert_eq!(target.count("clipboard_history"), 1);
    let actual: Vec<u8> = target
        .db
        .connect()
        .unwrap()
        .query_row("SELECT content_hash FROM clipboard_items", [], |r| r.get(0))
        .unwrap();
    let expected: Vec<u8> = connection
        .query_row(
            "SELECT content_hash FROM clipboard_items WHERE id = ?1",
            [latest],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn webdav_latest_favorite_goes_to_favorites_then_new_copy_goes_to_history() {
    let relay = Relay::new();
    let source = Fixture::new();
    let target = Fixture::new();
    let (mut a, mut b) = paired();
    a.dav = relay.config();
    a.dav.auto_upload = false;
    b.dav = relay.config();
    let sender = runtime(&source, a);
    sender.test_relay_agent(relay.agent.clone());
    let receiver = runtime(&target, b);
    receiver.test_relay_agent(relay.agent.clone());
    crate::services::custom_text::CustomTextService::new(source.db.clone(), source.blobs.clone())
        .create_manual_text_and_favorite("新增收藏而不是远端历史")
        .unwrap();
    sender.upload_latest().unwrap();
    sender.test_send_pending().unwrap();
    receiver.test_poll_dav().unwrap();
    assert_eq!(target.count("favorites"), 1);
    assert_eq!(target.count("clipboard_history"), 0);
    assert!(receiver.latest_upload().unwrap().is_none());

    source.capture("后续复制的内容");
    sender.upload_latest().unwrap();
    sender.test_send_pending().unwrap();
    receiver.test_poll_dav().unwrap();
    assert_eq!(target.count("favorites"), 1);
    assert_eq!(target.count("clipboard_history"), 1);
}

#[test]
fn webdav_cache_evicts_oldest_and_keeps_new_manual_uploads_after_restart() {
    let relay = Relay::new();
    let source = Fixture::new();
    let target = Fixture::new();
    let (mut a, mut b) = paired();
    a.lan_enabled = false;
    a.dav = relay.config();
    a.dav.cache_entries = 2;
    b.dav = relay.config();
    let sender = runtime(&source, a.clone());
    sender.test_relay_agent(relay.agent.clone());
    for text in ["最早的大资源".repeat(4000), "保留一".into()] {
        sender
            .upload(None, source.capture(&text), Mode::ManualContent)
            .unwrap();
        sender.test_send_pending().unwrap();
    }
    drop(sender);
    let sender = runtime(&source, a);
    sender.test_relay_agent(relay.agent.clone());
    sender
        .upload(None, source.capture("保留二"), Mode::ManualContent)
        .unwrap();
    sender.test_send_pending().unwrap();
    assert!(
        relay
            .objects
            .lock()
            .unwrap()
            .keys()
            .all(|p| !p.starts_with("/qcclip-v1/objects/") || p.ends_with('/')),
        "淘汰的外部资源应删除"
    );
    let receiver = runtime(&target, b);
    receiver.test_relay_agent(relay.agent.clone());
    receiver.test_poll_dav().unwrap();
    assert_eq!(target.count("clipboard_history"), 2);
}

#[test]
fn webdav_automatic_upload_switch_does_not_block_manual_upload() {
    let relay = Relay::new();
    let source = Fixture::new();
    let (mut a, _) = paired();
    a.lan_enabled = false;
    a.dav = relay.config();
    a.dav.auto_upload = false;
    a.dav.auto_pull = false;
    let sender = runtime(&source, a);
    sender.test_relay_agent(relay.agent.clone());
    let item = source.capture("仅手动上传");
    sender.automatic(&source.db, item, Mode::NewContent);
    sender.test_send_pending().unwrap();
    assert!(relay.requests.lock().unwrap().is_empty());
    sender.upload(None, item, Mode::ManualContent).unwrap();
    sender.test_send_pending().unwrap();
    relay.requests.lock().unwrap().clear();
    sender
        .upload(None, source.capture("第二次手动上传"), Mode::ManualContent)
        .unwrap();
    sender.test_send_pending().unwrap();
    assert_eq!(
        relay.requests.lock().unwrap().len(),
        1,
        "普通条目稳定上传只应 PUT 一次索引"
    );
}

#[test]
fn webdav_manual_fetch_postpones_auto_fetch_and_disabled_auto_has_no_requests() {
    let relay = Relay::new();
    let target = Fixture::new();
    let (_, mut b) = paired();
    b.dav = relay.config();
    b.dav.poll_seconds = 60;
    b.dav.auto_pull = false;
    let receiver = runtime(&target, b);
    receiver.test_relay_agent(relay.agent.clone());
    assert_eq!(receiver.test_auto_poll_dav().unwrap(), 0);
    assert!(relay.requests.lock().unwrap().is_empty());
    receiver
        .change(
            |c| {
                c.dav.auto_pull = true;
                Ok(())
            },
            false,
        )
        .unwrap();
    receiver.pull_dav().unwrap();
    assert_eq!(relay.requests.lock().unwrap().len(), 1);
    receiver.test_auto_poll_dav().unwrap();
    assert_eq!(
        relay.requests.lock().unwrap().len(),
        1,
        "手动拉取后不能紧接着重复自动检查"
    );
}

#[test]
fn webdav_cache_recovers_failed_publication_without_deleting_committed_resources() {
    for committed in [false, true] {
        let relay = Relay::new();
        let source = Fixture::new();
        let target = Fixture::new();
        let (mut a, mut b) = paired();
        a.lan_enabled = false;
        a.dav = relay.config();
        b.dav = relay.config();
        let sender = runtime(&source, a.clone());
        sender.test_relay_agent(relay.agent.clone());
        *relay.failure.lock().unwrap() = Some((
            "PUT".into(),
            format!(
                "/qcclip-v1/{}",
                super::dav_cache::index_path(&a.profile.key)
            ),
            500,
            committed,
        ));
        sender
            .upload(
                None,
                source.capture(&"中断后的资源".repeat(5000)),
                Mode::ManualContent,
            )
            .unwrap();
        assert!(sender.test_send_pending().is_err());
        let config = sender.config();
        drop(sender);
        let sender = runtime(&source, config);
        sender.test_relay_agent(relay.agent.clone());
        sender
            .upload(None, source.capture("恢复后继续上传"), Mode::ManualContent)
            .unwrap();
        sender.test_send_pending().unwrap();
        let receiver = runtime(&target, b);
        receiver.test_relay_agent(relay.agent.clone());
        receiver.test_poll_dav().unwrap();
        assert_eq!(
            target.count("clipboard_history"),
            if committed { 2 } else { 1 }
        );
        let resources = relay
            .objects
            .lock()
            .unwrap()
            .keys()
            .filter(|p| p.starts_with("/qcclip-v1/objects/") && !p.ends_with('/'))
            .count();
        assert_eq!(
            resources,
            usize::from(committed),
            "未提交的资源应回收，已提交资源必须保留"
        );
    }
}

#[test]
fn webdav_cache_enforces_byte_capacity_and_independent_target_encryption() {
    let relay = Relay::new();
    let source = Fixture::new();
    let target = Fixture::new();
    let third = Fixture::new();
    let (mut a, mut b) = paired();
    let mut c = super::config::Config::fresh();
    c.enabled = true;
    c.dav = relay.config();
    c.peers.push(super::config::Peer {
        profile: a.profile.clone(),
        address: String::new(),
        policy: Default::default(),
    });
    a.peers.push(super::config::Peer {
        profile: c.profile.clone(),
        address: String::new(),
        policy: Default::default(),
    });
    a.lan_enabled = false;
    a.dav = relay.config();
    a.dav.cache_mb = 1;
    b.dav = relay.config();
    let sender = runtime(&source, a.clone());
    sender.test_relay_agent(relay.agent.clone());
    for text in ["资源甲".repeat(100000), "资源乙".repeat(100000)] {
        sender
            .upload(
                Some(&b.profile.id),
                source.capture(&text),
                Mode::ManualContent,
            )
            .unwrap();
        sender.test_send_pending().unwrap();
    }
    sender
        .upload(
            Some(&c.profile.id),
            source.capture("仅第三台可见"),
            Mode::ManualContent,
        )
        .unwrap();
    sender.test_send_pending().unwrap();
    for (fixture, config) in [(&target, b), (&third, c)] {
        let receiver = runtime(fixture, config);
        receiver.test_relay_agent(relay.agent.clone());
        receiver.test_poll_dav().unwrap();
        assert_eq!(fixture.count("clipboard_history"), 1);
    }
    assert_eq!(
        relay
            .objects
            .lock()
            .unwrap()
            .keys()
            .filter(|p| p.starts_with("/qcclip-v1/indexes/") && !p.ends_with('/'))
            .count(),
        1,
        "多目标仍共用发送设备的一份索引"
    );
}

#[test]
fn webdav_cache_and_online_messages_are_separate_protocol_domains() {
    let (a, b) = paired();
    let request = Request::Hello {
        challenge: "不能冒充缓存".into(),
        profile: a.profile.clone(),
    };
    let online = protocol::seal(&a, &a.peers[0], &request).unwrap();
    assert!(protocol::open_cache::<Request>(&b, &online, "webdav-cache-entry-v1").is_err());
    let cached = protocol::seal_cache(&a, &a.peers[0], &request, "webdav-cache-entry-v1").unwrap();
    assert!(protocol::open::<Request>(&b, &cached).is_err());
    assert!(protocol::open_cache::<Request>(&b, &cached, "webdav-cache-object-v1").is_err());
    assert!(protocol::open_cache::<Request>(&b, &cached, "webdav-cache-entry-v1").is_ok());
}

#[test]
fn webdav_rate_limit_cools_down_all_operations_without_repeated_requests() {
    let relay = Relay::new();
    let target = Fixture::new();
    let (_, mut b) = paired();
    b.dav = relay.config();
    let path = format!(
        "/qcclip-v1/{}",
        super::dav_cache::index_path(&b.peers[0].profile.key)
    );
    *relay.failure.lock().unwrap() = Some(("GET".into(), path, 429, false));
    let receiver = runtime(&target, b);
    receiver.test_relay_agent(relay.agent.clone());
    assert!(receiver.pull_dav().unwrap_err().to_string().contains("429"));
    assert!(
        receiver
            .pull_dav()
            .unwrap_err()
            .to_string()
            .contains("限流")
    );
    assert_eq!(relay.requests.lock().unwrap().len(), 1);
    assert_eq!(receiver.snapshot().dav_requests, 1);
}

#[test]
fn webdav_new_local_items_upload_individually_and_corruption_never_ingests() {
    let relay = Relay::new();
    let source = Fixture::new();
    let target = Fixture::new();
    let (mut a, mut b) = paired();
    a.lan_enabled = false;
    a.dav = relay.config();
    b.dav = relay.config();
    let sender = runtime(&source, a.clone());
    sender.test_relay_agent(relay.agent.clone());
    for text in ["新增直接上传一", "新增直接上传二"] {
        sender.automatic(&source.db, source.capture(text), Mode::NewContent);
        sender.test_send_pending().unwrap();
    }
    assert_eq!(
        relay
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "PUT")
            .count(),
        2
    );
    let key = format!(
        "/qcclip-v1/{}",
        super::dav_cache::index_path(&a.profile.key)
    );
    let mut value: serde_json::Value =
        serde_json::from_slice(&relay.objects.lock().unwrap()[&key]).unwrap();
    value["entries"][0]["envelope"]["ciphertext"] = serde_json::json!("AAAA");
    relay
        .objects
        .lock()
        .unwrap()
        .insert(key, serde_json::to_vec(&value).unwrap());
    let receiver = runtime(&target, b);
    receiver.test_relay_agent(relay.agent.clone());
    assert!(receiver.test_poll_dav().is_err());
    assert_eq!(target.count("clipboard_items"), 1);
    assert_eq!(target.count("clipboard_history"), 1);
}

#[test]
fn webdav_mixed_cache_isolates_invalid_entries_and_keeps_receiving_new_uploads() {
    let relay = Relay::new();
    let source = Fixture::new();
    let target = Fixture::new();
    let (mut a, mut b) = paired();
    a.dav = relay.config();
    b.dav = relay.config();
    let sender = runtime(&source, a.clone());
    sender.test_relay_agent(relay.agent.clone());
    for index in 0..6 {
        source.capture(&format!("缓存条目{index}"));
        sender.upload_latest().unwrap();
        sender.test_send_pending().unwrap();
    }
    let key = format!(
        "/qcclip-v1/{}",
        super::dav_cache::index_path(&a.profile.key)
    );
    let mut index: serde_json::Value =
        serde_json::from_slice(&relay.objects.lock().unwrap()[&key]).unwrap();
    for position in 0..2 {
        let envelope =
            serde_json::from_value(index["entries"][position]["envelope"].clone()).unwrap();
        let (_, mut entry): (_, serde_json::Value) =
            protocol::open_cache(&b, &envelope, "webdav-cache-entry-v1").unwrap();
        if position == 0 {
            entry["event"]["mode"] = serde_json::json!("manual_metadata");
        } else {
            entry["event"].as_object_mut().unwrap().remove("formats");
        }
        index["entries"][position]["envelope"] = serde_json::to_value(
            protocol::seal_cache(&a, &a.peers[0], &entry, "webdav-cache-entry-v1").unwrap(),
        )
        .unwrap();
    }
    index["entries"][2]["envelope"]
        .as_object_mut()
        .unwrap()
        .remove("nonce");
    index["entries"][3]["envelope"]["ciphertext"] = serde_json::json!("AAAA");
    index["entries"][4]["raw_size"] = serde_json::json!(0);
    // 其他接收方的未知结构不能阻止本机读取，也不应算作本机失败。
    index["entries"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "envelope": { "sender": "another-recipient" }, "raw_size": "unknown"
        }));
    let invalid_entries = index["entries"].as_array().unwrap()[..5].to_vec();
    relay
        .objects
        .lock()
        .unwrap()
        .insert(key.clone(), serde_json::to_vec(&index).unwrap());
    let receiver = runtime(&target, b);
    receiver.test_relay_agent(relay.agent.clone());
    let error = receiver.test_poll_dav().unwrap_err().to_string();
    assert!(error.contains("5 条"), "{error}");
    assert!(error.contains("1 条"), "{error}");
    assert!(error.contains("同一版本"), "{error}");
    assert_eq!(target.count("clipboard_history"), 1);

    crate::services::custom_text::CustomTextService::new(source.db.clone(), source.blobs.clone())
        .create_manual_text_and_favorite("异常缓存后新增收藏")
        .unwrap();
    sender.upload_latest().unwrap();
    sender.test_send_pending().unwrap();
    // 保持前面的异常条目，确认后续上传仍能穿过这些条目接收。
    let mut index: serde_json::Value =
        serde_json::from_slice(&relay.objects.lock().unwrap()[&key]).unwrap();
    index["entries"].as_array_mut().unwrap()[..5].clone_from_slice(&invalid_entries);
    relay
        .objects
        .lock()
        .unwrap()
        .insert(key, serde_json::to_vec(&index).unwrap());
    receiver.test_poll_dav().unwrap_err();
    assert_eq!(target.count("favorites"), 1);
    assert_eq!(target.count("clipboard_history"), 1);
    assert_eq!(target.count("clipboard_items"), 2);
}

#[test]
fn webdav_concurrent_text_and_resource_uploads_preserve_all_index_entries() {
    let relay = Relay::new();
    let source = Fixture::new();
    let target = Fixture::new();
    let (mut a, mut b) = paired();
    a.lan_enabled = false;
    a.dav = relay.config();
    b.dav = relay.config();
    let sender = Arc::new(runtime(&source, a.clone()));
    sender.test_relay_agent(relay.agent.clone());
    let mut packets = Vec::new();
    for index in 0..8 {
        let text = if index % 2 == 0 {
            format!("并发资源{index}").repeat(5000)
        } else {
            format!("并发文本{index}")
        };
        let mut packet = source.packet(source.capture(&text), Mode::NewContent, index + 1);
        packet.event.sender = a.profile.id.clone();
        packet.event.recipient = b.profile.id.clone();
        packets.push(packet);
    }
    thread::scope(|scope| {
        for packet in &packets {
            let sender = &sender;
            let peer = &a.peers[0];
            let session = super::protocol::Session {
                id: packet.event.session.clone(),
                expires: protocol::now() + 30,
                profile: b.profile.clone(),
            };
            scope.spawn(move || sender.test_relay_send(peer, packet, session).unwrap());
        }
    });
    let receiver = runtime(&target, b);
    receiver.test_relay_agent(relay.agent.clone());
    receiver.test_poll_dav().unwrap();
    assert_eq!(target.count("clipboard_history"), 8);
    assert_eq!(
        relay
            .objects
            .lock()
            .unwrap()
            .keys()
            .filter(|p| p.starts_with("/qcclip-v1/objects/") && !p.ends_with('/'))
            .count(),
        4
    );
}
