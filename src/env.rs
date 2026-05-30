//! 沙箱环境变量操作,对齐 Go SDK env/env.go。
//!
//! [`Env`] 提供在 sandbox 内读取和设置环境变量的接口。
//! 由 `Sandbox::env()` 构造,不建议直接实例化。

use std::collections::HashMap;

use serde::Deserialize;

use crate::client::Client;
use crate::errors::Result;

/// 沙箱环境变量句柄。
///
/// 所有操作都路由到指定 sandbox 的 env 端点。
/// 可安全 Clone(内部 `Client` 共享连接池)。
#[derive(Clone)]
pub struct Env {
    sandbox_id: String,
    client: Client,
}

/// `GET /v1/sandboxes/{id}/env/{key}` 的响应体。
#[derive(Debug, Deserialize)]
struct EnvGetResponse {
    #[serde(default)]
    value: String,
}

impl Env {
    /// 构造环境变量句柄。由 `Sandbox::env()` 调用,通常不需要直接使用。
    pub(crate) fn new(sandbox_id: String, client: Client) -> Env {
        Env { sandbox_id, client }
    }

    /// 读取 sandbox 内的单个环境变量值。
    ///
    /// 对应端点:`GET /v1/sandboxes/{id}/env/{key}`。
    /// 对齐 Go `Env.Get`:key 不存在时服务端通常返回空字符串(非错误),
    /// 因此返回 `Ok(String)` 而非 `Option`;空字符串表示未设置。
    pub async fn get(&self, key: &str) -> Result<String> {
        let path = format!("/v1/sandboxes/{}/env/{}", self.sandbox_id, key);
        let resp: EnvGetResponse = self.client.get(&path).await?;
        Ok(resp.value)
    }

    /// 在 sandbox 内设置(或更新)一个环境变量。
    ///
    /// 对应端点:`POST /v1/sandboxes/{id}/env`,body `{"key": ..., "value": ...}`。
    /// 对齐 Go `Env.Set`。
    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        let path = format!("/v1/sandboxes/{}/env", self.sandbox_id);
        let body = HashMap::from([("key", key), ("value", value)]);
        self.client.post_no_content(&path, &body).await
    }
}
