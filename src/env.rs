//! 沙箱环境变量操作。
//!
//! [`Env`] 提供读取、设置、批量查询和删除 sandbox 环境变量的接口。
//! 由 `Sandbox::env()` 构造,不建议直接实例化。
//!
//! # 热更新语义
//!
//! 以下操作(set/unset)只更新持久化存储的值,**不重启已运行的进程**。
//! 下次通过 `Start`/`spawn` 启动的新进程才会读取到更新后的值。
//! 如需让运行中的进程生效,需要在 sandbox 内手动 `export` 或重启进程。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

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

/// `GET /v1/sandboxes/{id}/env`、`PUT /v1/sandboxes/{id}/env/{key}`、
/// `DELETE /v1/sandboxes/{id}/env/{key}` 的响应体——全量 env map。
#[derive(Debug, Deserialize)]
struct EnvMapResponse {
    env: HashMap<String, String>,
}

/// `PUT /v1/sandboxes/{id}/env/{key}` 的请求体。
/// key 在 path 里,body 只携带 value。
#[derive(Debug, Serialize)]
struct EnvSetBody<'a> {
    value: &'a str,
}

impl Env {
    /// 构造环境变量句柄。由 `Sandbox::env()` 调用,通常不需要直接使用。
    pub(crate) fn new(sandbox_id: String, client: Client) -> Env {
        Env { sandbox_id, client }
    }

    /// 读取 sandbox 内的单个环境变量值。
    ///
    /// 契约:`GET /v1/sandboxes/{id}/env/{key}` → `{"value": "..."}`。
    /// key 不存在时服务端返回空字符串(非错误),因此返回 `Ok(String)`;
    /// 空字符串表示该变量未设置。
    pub async fn get(&self, key: &str) -> Result<String> {
        let path = format!("/v1/sandboxes/{}/env/{}", self.sandbox_id, key);
        let resp: EnvGetResponse = self.client.get(&path).await?;
        Ok(resp.value)
    }

    /// 获取 sandbox 内所有环境变量的全量快照。
    ///
    /// 契约:`GET /v1/sandboxes/{id}/env` → `{"env": {k: v, ...}}`。
    /// 返回当前持久化的全部 key-value 对;运行中进程实际读到的值可能不同
    /// (见模块级热更新语义说明)。
    pub async fn all(&self) -> Result<HashMap<String, String>> {
        let path = format!("/v1/sandboxes/{}/env", self.sandbox_id);
        let resp: EnvMapResponse = self.client.get(&path).await?;
        Ok(resp.env)
    }

    /// 设置(或更新)sandbox 内的单个环境变量。
    ///
    /// 契约:`PUT /v1/sandboxes/{id}/env/{key}`,body `{"value": "..."}` →
    /// `{"env": {k: v, ...}}`(更新后的全量 map)。
    /// key 在 path 里,body 只携带 value。
    ///
    /// 只更新持久化值,不重启已运行进程;下次 Start/spawn 的进程读到新值。
    pub async fn set(&self, key: &str, value: &str) -> Result<HashMap<String, String>> {
        let path = format!("/v1/sandboxes/{}/env/{}", self.sandbox_id, key);
        let body = EnvSetBody { value };
        let resp: EnvMapResponse = self.client.put(&path, &body).await?;
        Ok(resp.env)
    }

    /// 删除 sandbox 内的单个环境变量。
    ///
    /// 契约:`DELETE /v1/sandboxes/{id}/env/{key}` →
    /// `{"env": {k: v, ...}}`(删除后的全量 map)。
    ///
    /// 只更新持久化值,不重启已运行进程;下次 Start/spawn 的进程读到新值。
    pub async fn unset(&self, key: &str) -> Result<HashMap<String, String>> {
        let path = format!("/v1/sandboxes/{}/env/{}", self.sandbox_id, key);
        let resp: EnvMapResponse = self.client.delete_json(&path).await?;
        Ok(resp.env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── EnvGetResponse 反序列化 ────────────────────────────────────────────────

    /// 正常 value 字段反序列化。
    #[test]
    fn env_get_response_deserialize() {
        let json = r#"{"value": "hello"}"#;
        let resp: EnvGetResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.value, "hello");
    }

    /// key 不存在时服务端返回空字符串(或省略字段),默认为空串。
    #[test]
    fn env_get_response_missing_value_defaults_empty() {
        let json = r#"{}"#;
        let resp: EnvGetResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.value, "");
    }

    // ─── EnvMapResponse 反序列化(all / set / unset 共用) ───────────────────────

    /// all/set/unset 响应体正确反序列化全量 map。
    #[test]
    fn env_map_response_deserialize() {
        let json = r#"{"env": {"FOO": "bar", "BAZ": "qux"}}"#;
        let resp: EnvMapResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.env.get("FOO").map(|s| s.as_str()), Some("bar"));
        assert_eq!(resp.env.get("BAZ").map(|s| s.as_str()), Some("qux"));
        assert_eq!(resp.env.len(), 2);
    }

    /// 空 env map 正常解析。
    #[test]
    fn env_map_response_empty() {
        let json = r#"{"env": {}}"#;
        let resp: EnvMapResponse = serde_json::from_str(json).unwrap();
        assert!(resp.env.is_empty());
    }

    // ─── EnvSetBody 序列化(PUT body 只含 value) ─────────────────────────────────

    /// set 请求体序列化:只包含 value,不包含 key。
    #[test]
    fn env_set_body_serialize_value_only() {
        let body = EnvSetBody { value: "my_secret" };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["value"], "my_secret");
        // key 不应出现在 body 里(契约:key 在 path)
        assert!(json.get("key").is_none());
    }

    /// set 请求体空 value。
    #[test]
    fn env_set_body_empty_value() {
        let body = EnvSetBody { value: "" };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["value"], "");
    }
}
