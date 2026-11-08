use std::{io::Write, sync::{atomic::{self, AtomicBool}, Arc, OnceLock}};

use arc_swap::ArcSwap;
use serde::de::DeserializeOwned;

use super::Error;

pub struct AsyncRequest<T: Send + Sync> {
    request: ureq::Request,
    map_fn: fn(ureq::Response) -> Result<T, Error>,
    running: AtomicBool,
    pub result: ArcSwap<Option<Result<T, Error>>>
}

impl<T: Send + Sync + 'static> AsyncRequest<T> {
    pub fn new(request: ureq::Request, map_fn: fn(ureq::Response) -> Result<T, Error>) -> Self {
        AsyncRequest {
            request,
            map_fn,
            running: AtomicBool::new(false),
            result: ArcSwap::default()
        }
    }

    pub fn call(self: Arc<Self>) {
        self.result.store(Arc::new(None));
        self.running.store(true, atomic::Ordering::Release);
        std::thread::spawn(move || {
            let res = match self.request.clone().call() {
                Ok(v) => (self.map_fn)(v),
                Err(e) => Err(Error::from(e))
            };
            self.result.store(Arc::new(Some(res)));
            self.running.store(false, atomic::Ordering::Release);
        });
    }

    pub fn running(&self) -> bool {
        self.running.load(atomic::Ordering::Acquire)
    }
}

impl<T: Send + Sync + 'static + DeserializeOwned> AsyncRequest<T> {
    pub fn with_json_response(request: ureq::Request) -> AsyncRequest<T> {
        AsyncRequest::new(request, |res|
            Ok(serde_json::from_str(&res.into_string()?)?)
        )
    }
}

/// 所有連線共用的 TLS 設定：內建的 Mozilla 根憑證清單（ureq 原本只用這份）＋ Windows 憑證存放區的根憑證。
/// 防毒軟體的「HTTPS 掃描」、公司／學校網路會用自己的根憑證重新簽 GitHub 的憑證；那張根憑證 Windows 信任、
/// 內建清單沒有 → 原本會「invalid peer certificate: UnknownIssuer」連不上（朋友 11/09 回報）。
fn tls_config() -> Arc<rustls::ClientConfig> {
    static CFG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CFG.get_or_init(|| {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let native = rustls_native_certs::load_native_certs();
        for e in &native.errors {
            warn!("Failed to load some system root certificates: {}", e);
        }
        let (added, ignored) = roots.add_parsable_certificates(native.certs);
        info!("TLS roots: {} built-in + {} from Windows ({} ignored)", webpki_roots::TLS_SERVER_ROOTS.len(), added, ignored);
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Arc::new(
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("rustls protocol versions")
                .with_root_certificates(roots)
                .with_no_client_auth()
        )
    }).clone()
}

/// 自訂 timeout／轉址等設定時用這個起頭（TLS 設定已經套好）
pub fn agent_builder() -> ureq::AgentBuilder {
    ureq::AgentBuilder::new().tls_config(tls_config())
}

/// 一般請求共用的 agent
pub fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| agent_builder().build())
}

pub fn get_json<T: DeserializeOwned>(url: &str) -> Result<T, Error> {
    let res = agent().get(url).call()?;
    Ok(serde_json::from_str(&res.into_string()?)?)
}

pub fn get_github_json<T: DeserializeOwned>(url: &str) -> Result<T, Error> {
    let res = agent().get(url)
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .call()?;
    Ok(serde_json::from_str(&res.into_string()?)?)
}

pub fn download_file_buffered(res: ureq::Response, file: &mut std::fs::File, buffer: &mut [u8], mut add_bytes: impl FnMut(&[u8])) -> Result<(), Error> {
    let mut reader = res.into_reader();
    let mut buffer_pos = 0usize;
    loop {
        let read_bytes = reader.read(&mut buffer[buffer_pos..])?;

        let prev_buffer_pos = buffer_pos;
        buffer_pos += read_bytes;
        add_bytes(&buffer[prev_buffer_pos..buffer_pos]);

        if buffer_pos == buffer.len() {
            buffer_pos = 0;
            let written = file.write(&buffer)?;
            if written != buffer.len() {
                return Err(Error::OutOfDiskSpace);
            }
        }

        if read_bytes == 0 {
            break;
        }
    }

    // Download finished, flush the buffer
    if buffer_pos != 0 {
        let written = file.write(&buffer[..buffer_pos])?;
        if written != buffer_pos {
            return Err(Error::OutOfDiskSpace);
        }
    }

    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    /// 重現防毒／公司網路的 HTTPS 中間人：MITM_PROXY＝本機 mitmproxy（它的根憑證已加進 Windows 憑證存放區）。
    /// 只用內建清單（舊行為）會 UnknownIssuer；加上 Windows 根憑證就連得上。
    /// MITM_PROXY=http://127.0.0.1:8085 cargo test --lib tls_trusts_windows_roots -- --ignored
    #[test]
    #[ignore]
    fn tls_trusts_windows_roots() {
        let proxy = std::env::var("MITM_PROXY").expect("MITM_PROXY");
        let url = "https://github.com/tfluan0606/hachimi-tw/releases/latest";
        let old = ureq::AgentBuilder::new().proxy(ureq::Proxy::new(&proxy).unwrap()).redirects(0).build();
        let e = old.get(url).call().err().map(|e| e.to_string()).unwrap_or_default();
        assert!(e.contains("UnknownIssuer"), "舊設定應該失敗：{e}");
        let new = agent_builder().proxy(ureq::Proxy::new(&proxy).unwrap()).redirects(0).build();
        // 經過中間人代理，GitHub API 要拿到完整的成功回應
        let api = "https://api.github.com/repos/tfluan0606/hachimi-tw/releases/latest";
        let r = agent_builder().proxy(ureq::Proxy::new(&proxy).unwrap()).build().get(api).call().expect("經過代理呼叫 API");
        let v: serde_json::Value = serde_json::from_str(&r.into_string().unwrap()).unwrap();
        assert!(v["tag_name"].as_str().unwrap_or("").starts_with('v'), "{v}");
        // 不經代理：一般網路下也照常（get_github_json 走共用 agent）
        let v: serde_json::Value = get_github_json(api).expect("直接呼叫 API");
        eprintln!("latest = {}", v["tag_name"]);
        // TLS 握手過了就算數：拿到任何 HTTP 回應（含代理自己的 5xx）都代表憑證被信任
        match new.get(url).call() {
            Ok(_) | Err(ureq::Error::Status(..)) => {}
            Err(e) => panic!("新設定應該信任 Windows 的根憑證：{e}"),
        }
    }
}
