//! 错误类型层次,对齐其它语言 SDK(Go sentinel / TS error 类层次)。
//!
//! Rust 用单一 [`Error`] enum 表达所有失败,变体按 HTTP 语义分类。
//! 调用方可直接 `match` 变体,等价于 Go 的 `errors.Is(err, ErrNotFound)`
//! 或 TS 的 `instanceof NotFoundError`。

use std::time::Duration;

/// SDK 统一错误类型。
///
/// 变体与服务端 HTTP 语义一一对应:
/// - [`Error::Auth`] —— 401 / 403
/// - [`Error::NotFound`] —— 404
/// - [`Error::Quota`] —— 422(配额超限)
/// - [`Error::RateLimit`] —— 429
/// - [`Error::Server`] —— 5xx
/// - [`Error::Api`] —— 其它 4xx
/// - [`Error::Network`] —— HTTP 传输层失败
/// - [`Error::Timeout`] —— 等待 sandbox 状态超时
/// - [`Error::PtyClosed`] —— 向已关闭的 PTY 会话写入
/// - [`Error::NotImplemented`] —— 服务端尚未实现该端点
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 401 / 403:认证 / 授权失败。
    #[error("sandbox API error {status}: {message} (auth)")]
    Auth {
        status: u16,
        message: String,
        request_id: Option<String>,
    },

    /// 404:资源不存在。
    #[error("sandbox API error 404: {message} (not found)")]
    NotFound {
        message: String,
        request_id: Option<String>,
    },

    /// 422:配额超限。
    #[error("sandbox API error 422: {message} (quota)")]
    Quota {
        message: String,
        request_id: Option<String>,
    },

    /// 429:限流。`retry_after` 来自 Retry-After 响应头(若有)。
    #[error("sandbox API error 429: {message} (rate limit)")]
    RateLimit {
        message: String,
        request_id: Option<String>,
        retry_after: Option<Duration>,
    },

    /// 5xx:服务端错误。
    #[error("sandbox API error {status}: {message} (server)")]
    Server {
        status: u16,
        message: String,
        request_id: Option<String>,
    },

    /// 其它未归类的 4xx。
    #[error("sandbox API error {status}: {message}")]
    Api {
        status: u16,
        message: String,
        request_id: Option<String>,
    },

    /// HTTP 传输层失败(连接拒绝、DNS、TLS 等)。
    #[error("network error: {0}")]
    Network(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// 等待 sandbox 进入目标状态时超过 deadline。
    #[error("timed out waiting for sandbox state (last state: {state}, elapsed: {elapsed:?})")]
    Timeout { state: String, elapsed: Duration },

    /// 向已关闭的 PTY 会话写入。
    #[error("PTY session is closed")]
    PtyClosed,

    /// 服务端尚未实现该端点(404 在版本探测语境下的语义化)。
    #[error("not implemented by server: {message}")]
    NotImplemented { message: String },

    /// 请求体序列化 / 响应体反序列化失败。
    #[error("serialization error: {0}")]
    Serde(#[source] serde_json::Error),

    /// 人类可读输入解析失败(如 "4GiB" / "30m")。
    #[error("parse error: {0}")]
    Parse(String),
}

impl Error {
    /// 由 HTTP 状态码 + 服务端 message + request id 构造对应变体。
    /// 对齐 Go `newAPIError` 的状态码分发逻辑。
    pub(crate) fn from_status(
        status: u16,
        message: String,
        request_id: Option<String>,
        retry_after: Option<Duration>,
    ) -> Self {
        match status {
            401 | 403 => Error::Auth {
                status,
                message,
                request_id,
            },
            404 => Error::NotFound {
                message,
                request_id,
            },
            422 => Error::Quota {
                message,
                request_id,
            },
            429 => Error::RateLimit {
                message,
                request_id,
                retry_after,
            },
            s if s >= 500 => Error::Server {
                status,
                message,
                request_id,
            },
            _ => Error::Api {
                status,
                message,
                request_id,
            },
        }
    }

    /// 该错误对应的 HTTP 状态码(若是 API 错误)。网络 / 解析等本地错误返回 `None`。
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Auth { status, .. }
            | Error::Server { status, .. }
            | Error::Api { status, .. } => Some(*status),
            Error::NotFound { .. } => Some(404),
            Error::Quota { .. } => Some(422),
            Error::RateLimit { .. } => Some(429),
            _ => None,
        }
    }

    /// 服务端返回的 X-Request-ID(若有),用于排障。
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Error::Auth { request_id, .. }
            | Error::NotFound { request_id, .. }
            | Error::Quota { request_id, .. }
            | Error::RateLimit { request_id, .. }
            | Error::Server { request_id, .. }
            | Error::Api { request_id, .. } => request_id.as_deref(),
            _ => None,
        }
    }
}

/// SDK 统一 Result 别名。
pub type Result<T> = std::result::Result<T, Error>;

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Serde(e)
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Network(Box::new(e))
    }
}
