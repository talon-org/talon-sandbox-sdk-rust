//! 低级 HTTP 客户端,对齐其它语言 SDK(Go client.go)。
//!
//! [`Client`] 封装 base URL + API key + reqwest 客户端,提供 get/post/delete 与
//! 统一的错误映射。资源模块(sandbox/fs/env/...)都通过它发请求。
//! `Client` 可安全 `Clone`(内部 reqwest::Client 是 Arc 共享的)并跨任务并发使用。

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::errors::{Error, Result};
use crate::types::ErrorBody;

/// 默认 API 服务地址(未显式指定且无 env 时)。
pub(crate) const DEFAULT_SERVER: &str = "http://localhost:18080";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// 所有 SDK 操作的 HTTP 客户端。
///
/// 通过 [`Client::builder`] 或 [`Client::new`] 构造。可 `Clone`,克隆共享底层连接池。
#[derive(Clone)]
pub struct Client {
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

/// [`Client`] 的构建器。
pub struct ClientBuilder {
    base_url: String,
    api_key: Option<String>,
    timeout: Duration,
}

impl Client {
    /// 用默认配置创建 client。
    ///
    /// `server` 为空时回退到 `TALON_SANDBOX_SERVER` 环境变量,再回退到
    /// `http://localhost:18080`。API key 从参数或 `TALON_SANDBOX_API_KEY` 读取
    /// (见 [`Client::builder`])。
    pub fn new(server: impl Into<String>) -> Self {
        Self::builder().server(server).build()
    }

    /// 返回一个构建器,用于设置 server / api_key / timeout。
    pub fn builder() -> ClientBuilder {
        ClientBuilder {
            base_url: resolve_server(None),
            api_key: env_api_key(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// 配置的 API base URL(已去除尾部斜杠)。
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    // ─── 内部 HTTP 助手 ───────────────────────────────────────────────────────

    /// 发 GET,把 JSON 响应反序列化成 `T`。
    pub(crate) async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let req = self.http.get(self.url(path));
        self.send(req).await
    }

    /// 发 POST(JSON body),把 JSON 响应反序列化成 `T`。
    pub(crate) async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let req = self.http.post(self.url(path)).json(body);
        self.send(req).await
    }

    /// 发 POST,不关心响应体(只校验状态码)。
    pub(crate) async fn post_no_content<B: Serialize>(&self, path: &str, body: &B) -> Result<()> {
        let req = self.http.post(self.url(path)).json(body);
        self.send_discard(req).await
    }

    /// 发 PUT raw bytes(用于 fs 写文件)。
    pub(crate) async fn put_bytes(&self, path: &str, body: Vec<u8>) -> Result<()> {
        let req = self
            .http
            .put(self.url(path))
            .header("Content-Type", "application/octet-stream")
            .body(body);
        self.send_discard(req).await
    }

    /// 发 GET 拿原始字节(用于 fs 读文件)。
    pub(crate) async fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let req = self.auth(self.http.get(self.url(path)));
        let resp = req.send().await.map_err(|e| Error::Network(Box::new(e)))?;
        let resp = self.check_status(resp).await?;
        let bytes = resp.bytes().await.map_err(|e| Error::Network(Box::new(e)))?;
        Ok(bytes.to_vec())
    }

    /// 发 DELETE。
    pub(crate) async fn delete(&self, path: &str) -> Result<()> {
        let req = self.http.delete(self.url(path));
        self.send_discard(req).await
    }

    /// 把 base HTTP URL 转成 WebSocket URL(用于 PTY)。
    pub(crate) fn ws_url(&self, path: &str) -> String {
        let base = self
            .base_url
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        format!("{base}{path}")
    }

    /// Authorization 头值(`Bearer <key>`),无 key 时返回 `None`。
    /// PTY WebSocket 握手用得到。
    pub(crate) fn auth_header(&self) -> Option<String> {
        self.api_key.as_ref().map(|k| format!("Bearer {k}"))
    }

    // ─── 私有 ─────────────────────────────────────────────────────────────────

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// 给请求附加 Accept + Authorization 头。
    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = req.header("Accept", "application/json");
        match &self.api_key {
            Some(k) => req.bearer_auth(k),
            None => req,
        }
    }

    /// 发请求 + 反序列化成 `T`,统一错误映射。
    async fn send<T: DeserializeOwned>(&self, req: reqwest::RequestBuilder) -> Result<T> {
        let resp = self
            .auth(req)
            .send()
            .await
            .map_err(|e| Error::Network(Box::new(e)))?;
        let resp = self.check_status(resp).await?;
        let bytes = resp.bytes().await.map_err(|e| Error::Network(Box::new(e)))?;
        serde_json::from_slice(&bytes).map_err(Error::Serde)
    }

    /// 发请求,丢弃响应体(只校验状态码)。
    async fn send_discard(&self, req: reqwest::RequestBuilder) -> Result<()> {
        let resp = self
            .auth(req)
            .send()
            .await
            .map_err(|e| Error::Network(Box::new(e)))?;
        self.check_status(resp).await?;
        Ok(())
    }

    /// 校验状态码:非 2xx 时读 body 的 `{"error":...}` + X-Request-ID + Retry-After,
    /// 映射成对应 [`Error`] 变体。对齐 Go `do` 的错误处理。
    async fn check_status(&self, resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let code = status.as_u16();
        let request_id = resp
            .headers()
            .get("X-Request-ID")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let retry_after = resp
            .headers()
            .get("Retry-After")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs);
        let body = resp.text().await.unwrap_or_default();
        // 优先解析 {"error": "..."},失败则用原始 body 当 message。
        let message = serde_json::from_str::<ErrorBody>(&body)
            .map(|b| b.error)
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or(body);
        Err(Error::from_status(code, message, request_id, retry_after))
    }
}

impl ClientBuilder {
    /// 设置 API 服务地址(空则走 env / 默认)。
    pub fn server(mut self, server: impl Into<String>) -> Self {
        let s = server.into();
        if !s.is_empty() {
            self.base_url = trim_trailing_slash(&s);
        }
        self
    }

    /// 设置 Bearer API key(`ask_...`)。
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// 设置每请求超时(默认 30s)。
    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = d;
        self
    }

    /// 构建 [`Client`]。
    pub fn build(self) -> Client {
        let http = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .expect("reqwest client build");
        Client {
            base_url: self.base_url,
            api_key: self.api_key,
            http,
        }
    }
}

fn trim_trailing_slash(s: &str) -> String {
    s.trim_end_matches('/').to_string()
}

/// 解析 server URL:显式 > TALON_SANDBOX_SERVER env > 默认。
fn resolve_server(explicit: Option<&str>) -> String {
    if let Some(s) = explicit {
        if !s.is_empty() {
            return trim_trailing_slash(s);
        }
    }
    match std::env::var("TALON_SANDBOX_SERVER") {
        Ok(s) if !s.is_empty() => trim_trailing_slash(&s),
        _ => DEFAULT_SERVER.to_string(),
    }
}

/// 从 TALON_SANDBOX_API_KEY env 读 api key。
fn env_api_key() -> Option<String> {
    std::env::var("TALON_SANDBOX_API_KEY").ok().filter(|s| !s.is_empty())
}
