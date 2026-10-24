use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use std::{
    collections::VecDeque,
    io::Read,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::{
    Result,
    config::{DavConfig, validate_dav_url},
    fail,
};

#[derive(Clone)]
pub struct Client {
    config: DavConfig,
    agent: ureq::Agent,
    base: String,
    traffic: Arc<Traffic>,
    initialized: Arc<Mutex<bool>>,
}

#[derive(Default)]
pub(super) struct Traffic {
    state: Mutex<(VecDeque<Instant>, Option<Instant>)>,
}

impl Traffic {
    fn begin(&self) -> Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(until) = state.1.filter(|until| *until > Instant::now()) {
            return fail(&format!(
                "WebDAV 正在等待服务器解除限流，请在 {} 秒后重试",
                until.saturating_duration_since(Instant::now()).as_secs() + 1
            ));
        }
        state
            .0
            .retain(|at| at.elapsed() < Duration::from_secs(1800));
        state.0.push_back(Instant::now());
        Ok(())
    }
    pub fn requests(&self) -> usize {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .0
            .retain(|at| at.elapsed() < Duration::from_secs(1800));
        state.0.len()
    }
    fn cooldown(&self, response: &ureq::Response) {
        let seconds = response
            .header("Retry-After")
            .and_then(|v| {
                v.parse::<u64>().ok().or_else(|| {
                    chrono::DateTime::parse_from_rfc2822(v).ok().map(|date| {
                        (date.timestamp().max(0) as u64).saturating_sub(super::protocol::now())
                    })
                })
            })
            .unwrap_or(60)
            .clamp(1, 86400);
        self.state.lock().unwrap_or_else(|e| e.into_inner()).1 =
            Some(Instant::now() + Duration::from_secs(seconds));
    }
}

impl Client {
    #[cfg(test)]
    pub fn with_agent(mut self, agent: Option<ureq::Agent>) -> Self {
        if let Some(agent) = agent {
            self.agent = agent;
        }
        self
    }
    pub fn new(config: &DavConfig) -> Result<Self> {
        let url = validate_dav_url(&config.url)?;
        Ok(Self {
            config: config.clone(),
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(8))
                .timeout_read(Duration::from_secs(30))
                .timeout_write(Duration::from_secs(60))
                .timeout(Duration::from_secs(180))
                .redirects(0)
                .build(),
            base: format!("{}/qcclip-v1/", url.as_str().trim_end_matches('/')),
            traffic: Arc::new(Traffic::default()),
            initialized: Arc::new(Mutex::new(false)),
        })
    }
    pub(super) fn with_traffic(mut self, traffic: Arc<Traffic>) -> Self {
        self.traffic = traffic;
        self
    }
    pub(super) fn matches(&self, config: &DavConfig) -> bool {
        self.config.url == config.url
            && self.config.username == config.username
            && self.config.password == config.password
    }
    fn execute(&self, method: &str, path: &str, body: Option<&[u8]>) -> Result<ureq::Response> {
        self.traffic.begin()?;
        let request = self
            .agent
            .request(method, &format!("{}{path}", self.base))
            .set(
                "Authorization",
                &format!(
                    "Basic {}",
                    B64.encode(format!("{}:{}", self.config.username, self.config.password))
                ),
            );
        let result = if let Some(body) = body {
            let request = if method == "PROPFIND" {
                request
                    .set("Depth", "1")
                    .set("Content-Type", "application/xml")
            } else {
                request.set("Content-Type", "application/octet-stream")
            };
            request.send_bytes(body)
        } else {
            request.call()
        };
        match result {
            Ok(response) => Ok(response),
            Err(ureq::Error::Status(status, response)) => {
                if status == 429 || status == 503 {
                    self.traffic.cooldown(&response);
                }
                if (method == "PUT" || method == "MKCOL") && (status == 404 || status == 409) {
                    *self.initialized.lock().unwrap_or_else(|e| e.into_inner()) = false;
                }
                Err(request_error(method, ureq::Error::Status(status, response)))
            }
            Err(error) => Err(request_error(method, error)),
        }
    }
    pub fn initialize(&self) -> Result<()> {
        if *self.initialized.lock().unwrap_or_else(|e| e.into_inner()) {
            return Ok(());
        }
        self.mkdir("")?;
        self.mkdir("indexes/")?;
        self.mkdir("objects/")?;
        *self.initialized.lock().unwrap_or_else(|e| e.into_inner()) = true;
        Ok(())
    }
    pub fn mkdir(&self, path: &str) -> Result<()> {
        // 405 表示目录已存在，不重复创建其父目录。
        match self.execute("MKCOL", path, None) {
            Ok(_) => Ok(()),
            Err(super::Error::Http { status: 405, .. }) => Ok(()),
            Err(error) => Err(error),
        }
    }
    pub fn put(&self, path: &str, bytes: &[u8]) -> Result<()> {
        self.execute("PUT", path, Some(bytes))?;
        Ok(())
    }
    pub fn get(&self, path: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        let response = match self.execute("GET", path, None) {
            Ok(response) => response,
            Err(super::Error::Http { status: 404, .. }) => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return fail("WebDAV 对象超过大小限制");
        }
        Ok(Some(bytes))
    }
    pub fn delete(&self, path: &str) -> Result<()> {
        match self.execute("DELETE", path, None) {
            Ok(_) => Ok(()),
            Err(super::Error::Http { status: 404, .. }) => Ok(()),
            Err(error) => Err(error),
        }
    }
    pub fn check_collection(&self, path: &str) -> Result<()> {
        let response = self.execute("PROPFIND", path, Some(b"<?xml version=\"1.0\"?><d:propfind xmlns:d=\"DAV:\"><d:prop><d:getcontentlength/><d:resourcetype/></d:prop></d:propfind>"))?;
        let mut xml = String::new();
        response
            .into_reader()
            .take(1024 * 1024 + 1)
            .read_to_string(&mut xml)?;
        if xml.len() > 1024 * 1024 {
            return fail("WebDAV 目录清单过大");
        }
        validate_collection(&xml)
    }
}

fn request_error(operation: &str, error: ureq::Error) -> super::Error {
    match error {
        ureq::Error::Status(status, _) => {
            let reason = match status {
                301..=308 => "服务器要求重定向，请填写最终 WebDAV 地址",
                401 => "认证失败，请检查账号和应用密码",
                403 => "服务器拒绝访问，请检查目录读写权限",
                404 => "远程目录不存在，请先创建配置地址对应的文件夹",
                409 => "父目录不存在，请先创建配置地址对应的文件夹",
                429 => "请求过于频繁，请降低轮询频率后重试",
                503 => "服务暂不可用或访问受限，请稍后重试",
                507 => "服务器存储空间或配额不足",
                _ => "服务器返回错误，请检查 WebDAV 服务状态",
            };
            super::Error::Http {
                status,
                message: format!("WebDAV {operation}失败（HTTP {status}）：{reason}"),
            }
        }
        ureq::Error::Transport(_) => super::Error::Message(format!(
            "WebDAV {operation}失败：连接失败、超时或 TLS 校验未通过，请检查服务器地址与连接协议"
        )),
    }
}

pub(super) fn validate_collection(xml: &str) -> Result<()> {
    let document = roxmltree::Document::parse(xml)
        .map_err(|_| super::Error::Message("WebDAV XML 清单无效".into()))?;
    if !document
        .root_element()
        .has_tag_name(("DAV:", "multistatus"))
    {
        return fail("WebDAV 返回的内容不是目录清单，请检查服务器地址");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread};

    #[test]
    fn mkdir_reports_authentication_missing_parent_and_unavailable_service() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            for status in [401, 409, 503] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    header.push(byte[0]);
                }
                write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
            }
        });
        let client = Client::new(&DavConfig {
            url: format!("http://{address}/QcClipSync"),
            ..Default::default()
        })
        .unwrap();
        for (status, reason) in [
            (401, "认证失败"),
            (409, "父目录不存在"),
            (503, "服务暂不可用"),
        ] {
            let error = client.initialize().err().unwrap().to_string();
            assert!(error.contains(&format!("HTTP {status}")));
            assert!(error.contains(reason));
        }
        worker.join().unwrap();
    }
}
