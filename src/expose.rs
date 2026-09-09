//! 端口暴露模块,对齐 Go SDK expose.go。
//!
//! 提供三个 `pub(crate)` 自由函数供 `Sandbox` 转调:
//! - [`expose`] —— 注册端口对外访问,返回 preview URL
//! - [`unexpose`] —— 取消端口暴露
//! - [`exposed`] —— 查询当前所有已暴露端口

use serde::Deserialize;
use serde_json::json;

use crate::client::Client;
use crate::errors::{Error, Result};
use crate::types::{ExposeOpts, ExposedPort};

// ─── 内部 wire DTO ────────────────────────────────────────────────────────────

/// expose 单端口响应体:`POST /v1/sandboxes/{id}/expose` 返回的 JSON 对象。
#[derive(Debug, Deserialize)]
struct ExposeResponse {
    /// 可外部访问的 preview URL。
    url: String,
    // 其余字段(port/signed/expires_at)暂不使用,用 `_` 占位避免反序列化警告。
    #[allow(dead_code)]
    #[serde(default)]
    port: i32,
    #[allow(dead_code)]
    #[serde(default)]
    signed: bool,
    #[allow(dead_code)]
    #[serde(default)]
    expires_at: String,
}

/// exposed 列表响应体:`GET /v1/sandboxes/{id}/expose` 返回的 JSON 对象。
#[derive(Debug, Deserialize)]
struct ExposedListResponse {
    /// 当前已暴露的端口列表。
    #[serde(default)]
    ports: Vec<ExposedPort>,
}

// ─── 公开(crate 内)自由函数 ──────────────────────────────────────────────────

/// 注册 `port` 对外访问,返回 preview URL。
///
/// 端点:`POST /v1/sandboxes/{sandbox_id}/expose`
///
/// 请求 body:
/// - `port`(必填):要暴露的端口号
/// - `sign`(可选):是否申请签名 URL,仅 `opts.sign == true` 时加入
/// - `ttl`(可选):签名 URL 有效期字符串(如 "1h"),直传服务端,不做本地转换
/// - `subdomain`(可选):自定义子域名前缀,非空时加入
///
/// 若服务端返回 404 且消息中不含 "sandbox"/"port",视为旧版本不支持该端点,
/// 返回 [`Error::NotImplemented`]。
pub(crate) async fn expose(
    client: &Client,
    sandbox_id: &str,
    port: u16,
    opts: &ExposeOpts,
) -> Result<String> {
    // 按 Go 实现:port 必填,其余字段按实际值有条件加入。
    let mut body = json!({ "port": port });

    if opts.sign {
        body["sign"] = json!(true);
    }
    if let Some(ttl) = &opts.ttl {
        if !ttl.is_empty() {
            // Go 直传字符串,不转秒数,Rust 同样直传。
            body["ttl"] = json!(ttl);
        }
    }
    if let Some(subdomain) = &opts.subdomain {
        if !subdomain.is_empty() {
            body["subdomain"] = json!(subdomain);
        }
    }

    let path = format!("/v1/sandboxes/{sandbox_id}/expose");
    let resp: ExposeResponse = client.post(&path, &body).await.map_err(|e| {
        if is_endpoint_missing(&e) {
            return Error::NotImplemented {
                message: "expose endpoint not yet available on this server".into(),
            };
        }
        wrap_port_error(e, "expose", port)
    })?;

    Ok(resp.url)
}

/// 取消 `port` 的外部暴露。
///
/// 端点:`DELETE /v1/sandboxes/{sandbox_id}/expose/{port}`
///
/// 若服务端返回 404 且消息中不含 "sandbox"/"port",视为旧版本不支持该端点,
/// 返回 [`Error::NotImplemented`]。
pub(crate) async fn unexpose(client: &Client, sandbox_id: &str, port: u16) -> Result<()> {
    let path = format!("/v1/sandboxes/{sandbox_id}/expose/{port}");
    client.delete(&path).await.map_err(|e| {
        if is_endpoint_missing(&e) {
            return Error::NotImplemented {
                message: "expose endpoint not yet available on this server".into(),
            };
        }
        wrap_port_error(e, "unexpose", port)
    })
}

/// 查询当前所有已暴露端口。
///
/// 端点:`GET /v1/sandboxes/{sandbox_id}/expose`
///
/// 返回响应体 `{ports:[...]}` 中的列表。若服务端返回 404 且消息中不含
/// "sandbox"/"port",视为旧版本不支持该端点,返回 [`Error::NotImplemented`]。
pub(crate) async fn exposed(client: &Client, sandbox_id: &str) -> Result<Vec<ExposedPort>> {
    let path = format!("/v1/sandboxes/{sandbox_id}/expose");
    let resp: ExposedListResponse = client.get(&path).await.map_err(|e| {
        if is_endpoint_missing(&e) {
            return Error::NotImplemented {
                message: "expose endpoint not yet available on this server".into(),
            };
        }
        match e {
            Error::NotFound {
                message,
                request_id,
            } => Error::NotFound {
                message,
                request_id,
            },
            other => other,
        }
    })?;

    Ok(resp.ports)
}

// ─── 内部辅助 ─────────────────────────────────────────────────────────────────

/// 判断 404 是否表示服务端根本没有实现该端点(而非资源不存在)。
///
/// 对齐 Go `endpointMissing`:404 消息不含 "sandbox" 或 "port" 时,认为是
/// chi 路由默认的 "404 page not found",即该路由在当前服务版本中不存在。
/// 消息含这两个词时,则是业务层的真实 not-found,应透传给调用方。
fn is_endpoint_missing(err: &Error) -> bool {
    match err {
        Error::NotFound { message, .. } => {
            let msg = message.to_lowercase();
            !msg.contains("sandbox") && !msg.contains("port")
        }
        _ => false,
    }
}

/// 把任意错误包装成含端口号上下文的错误。
///
/// 对齐 Go:`fmt.Errorf("expose port %d: %w", port, err)`。
/// Rust 没有 `%w` 链,直接把 message 前缀化,同时保留原始变体类型。
fn wrap_port_error(err: Error, op: &str, port: u16) -> Error {
    // 对各变体的 message 字段加前缀,保留其它字段不变。
    match err {
        Error::NotFound {
            message,
            request_id,
        } => Error::NotFound {
            message: format!("{op} port {port}: {message}"),
            request_id,
        },
        Error::Auth {
            status,
            message,
            request_id,
        } => Error::Auth {
            status,
            message: format!("{op} port {port}: {message}"),
            request_id,
        },
        Error::Quota {
            message,
            request_id,
        } => Error::Quota {
            message: format!("{op} port {port}: {message}"),
            request_id,
        },
        Error::RateLimit {
            message,
            request_id,
            retry_after,
        } => Error::RateLimit {
            message: format!("{op} port {port}: {message}"),
            request_id,
            retry_after,
        },
        Error::Server {
            status,
            message,
            request_id,
        } => Error::Server {
            status,
            message: format!("{op} port {port}: {message}"),
            request_id,
        },
        Error::Api {
            status,
            message,
            request_id,
        } => Error::Api {
            status,
            message: format!("{op} port {port}: {message}"),
            request_id,
        },
        // 网络 / 解析等本地错误无 message 字段,直接透传。
        other => other,
    }
}
