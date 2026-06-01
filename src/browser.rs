//! 沙箱内 headless Chromium 浏览器会话管理(Spec 34)。
//!
//! [`Browser`] 提供启动/查询/停止 CDP 浏览器的接口。
//! 由 `Sandbox::browser()` 构造,不建议直接实例化。
//!
//! # 典型用法
//!
//! ```no_run
//! use talon_sandbox::Sandbox;
//!
//! # async fn demo() -> talon_sandbox::Result<()> {
//! let sb = Sandbox::get("sbx_xxx").await?;
//! let br = sb.browser();
//!
//! // 启动浏览器,获取 CDP WebSocket URL
//! let sess = br.start().await?;
//! println!("CDP WS URL: {}", sess.cdp_ws_url);
//!
//! // 停止浏览器
//! br.stop().await?;
//! # Ok(())
//! # }
//! ```

use serde::Deserialize;

use crate::client::Client;
use crate::errors::Result;

/// 运行中的 headless Chromium 会话描述。
///
/// 调用方使用 [`BrowserSession::cdp_ws_url`] 通过 Chrome DevTools Protocol 连接浏览器。
/// `host_port` 是排障字段,日常调用方忽略它即可。
#[derive(Debug, Clone, Deserialize)]
pub struct BrowserSession {
    /// 所属 sandbox ID。
    pub sandbox_id: String,
    /// 关联进程 ID(来自 processes 表)。
    pub process_id: String,
    /// 容器内 CDP 端口,固定为 9222。
    pub cdp_port: i32,
    /// CDP DevTools 路径,如 `/devtools/browser/abc-def`。
    pub cdp_path: String,
    /// 客户端直连的 CDP WebSocket URL,形如:
    /// `wss://api.example.com/v1/sandboxes/{id}/preview/9222/devtools/browser/{uuid}`。
    /// 走 sandbox-api 反向代理,已含鉴权,直接传给 CDP 客户端即可。
    pub cdp_ws_url: String,
    /// Worker 侧 host 端口,排障用;日常调用方不需要关心。
    #[serde(default)]
    pub host_port: i32,
}

/// 沙箱内 headless Chromium 会话句柄。
///
/// 通过 [`Sandbox::browser`] 获取实例。可安全 Clone。
#[derive(Clone)]
pub struct Browser {
    sandbox_id: String,
    client: Client,
}

impl Browser {
    /// 构造浏览器句柄。由 `Sandbox::browser()` 调用,通常不需要直接使用。
    pub(crate) fn new(sandbox_id: String, client: Client) -> Browser {
        Browser { sandbox_id, client }
    }

    /// 在 sandbox 内启动 headless Chromium,返回 CDP 会话信息。
    ///
    /// 对应端点:`POST /v1/sandboxes/{id}/browser`。
    /// 若浏览器已在运行,服务端直接返回现有会话(幂等)。
    ///
    /// # 示例
    ///
    /// ```no_run
    /// use talon_sandbox::Sandbox;
    ///
    /// # async fn demo() -> talon_sandbox::Result<()> {
    /// let sb = Sandbox::get("sbx_xxx").await?;
    /// let sess = sb.browser().start().await?;
    /// println!("CDP: {}", sess.cdp_ws_url);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start(&self) -> Result<BrowserSession> {
        let path = format!("/v1/sandboxes/{}/browser", self.sandbox_id);
        // POST 不需要 body,传空 JSON 对象满足 Content-Type 要求
        self.client.post(&path, &serde_json::json!({})).await
    }

    /// 查询当前浏览器会话状态。
    ///
    /// 对应端点:`GET /v1/sandboxes/{id}/browser`。
    /// 若浏览器未启动,服务端返回 404,SDK 映射为 [`Error::NotFound`]。
    ///
    /// # 示例
    ///
    /// ```no_run
    /// use talon_sandbox::Sandbox;
    ///
    /// # async fn demo() -> talon_sandbox::Result<()> {
    /// let sb = Sandbox::get("sbx_xxx").await?;
    /// let sess = sb.browser().get().await?;
    /// println!("state cdp_ws_url={}", sess.cdp_ws_url);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get(&self) -> Result<BrowserSession> {
        let path = format!("/v1/sandboxes/{}/browser", self.sandbox_id);
        self.client.get(&path).await
    }

    /// 停止并销毁当前浏览器会话。
    ///
    /// 对应端点:`DELETE /v1/sandboxes/{id}/browser`。
    /// 若浏览器未启动,服务端返回 404,SDK 同样映射为 [`Error::NotFound`]。
    ///
    /// # 示例
    ///
    /// ```no_run
    /// use talon_sandbox::Sandbox;
    ///
    /// # async fn demo() -> talon_sandbox::Result<()> {
    /// let sb = Sandbox::get("sbx_xxx").await?;
    /// sb.browser().stop().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stop(&self) -> Result<()> {
        let path = format!("/v1/sandboxes/{}/browser", self.sandbox_id);
        self.client.delete(&path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证 BrowserSession 反序列化字段对齐 dto.go BrowserDTO。
    #[test]
    fn browser_session_deserialize() {
        let json = r#"{
            "sandbox_id": "sbx_abc",
            "process_id": "proc_xyz",
            "cdp_port": 9222,
            "cdp_path": "/devtools/browser/abc-def-123",
            "cdp_ws_url": "wss://api.example.com/v1/sandboxes/sbx_abc/preview/9222/devtools/browser/abc-def-123",
            "host_port": 32100
        }"#;
        let sess: BrowserSession = serde_json::from_str(json).unwrap();
        assert_eq!(sess.sandbox_id, "sbx_abc");
        assert_eq!(sess.process_id, "proc_xyz");
        assert_eq!(sess.cdp_port, 9222);
        assert_eq!(sess.cdp_path, "/devtools/browser/abc-def-123");
        assert!(sess.cdp_ws_url.starts_with("wss://"));
        assert_eq!(sess.host_port, 32100);
    }

    /// host_port 可选(服务端可能省略)。
    #[test]
    fn browser_session_no_host_port() {
        let json = r#"{
            "sandbox_id": "sbx_abc",
            "process_id": "proc_xyz",
            "cdp_port": 9222,
            "cdp_path": "/devtools/browser/abc-def-123",
            "cdp_ws_url": "wss://api.example.com/v1/sandboxes/sbx_abc/preview/9222/devtools/browser/abc-def-123"
        }"#;
        let sess: BrowserSession = serde_json::from_str(json).unwrap();
        // host_port 默认 0
        assert_eq!(sess.host_port, 0);
    }
}
