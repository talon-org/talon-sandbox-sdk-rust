//! Sandbox handle 及工厂函数,对齐 Go SDK sandbox.go。
//!
//! [`Sandbox`] 是一个活跃 sandbox 的 live handle,所有方法都发 API 调用,可并发使用。
//! 工厂函数 [`Sandbox::create`] / [`Sandbox::get`] / [`Sandbox::list`] 对齐 Go 包级函数
//! `Create` / `Get` / `List`,但 Rust 没有可变参数 Option,改为「无 client 版(用全局默认)
//! + with_client 版(显式 client)」双轨。

use serde_json::Value;

use crate::client::Client;
use crate::config::default_client;
use crate::errors::Result;
use crate::expose;
use crate::parse::{parse_duration, parse_size};
use crate::types::{
    CreateOpts, ExposeOpts, ExposedPort, ListOpts, SandboxInfo, SandboxListDto, SandboxState,
};

// 子资源 handle,由其它模块定义,这里按跨模块约定 import。
use crate::browser::Browser;
use crate::env::Env;
use crate::fs::Fs;
use crate::terminal::Terminal;

// ─── 网络策略别名映射 ──────────────────────────────────────────────────────────

/// 将友好名称(v2 风格)映射到 API 规范值。对齐 Go `networkAliases`。
fn normalize_network(s: &str) -> &str {
    match s {
        "allowlist" => "restricted-egress",
        "open" => "full-egress",
        "sealed" => "offline",
        "deny" => "offline",
        other => other,
    }
}

// ─── Sandbox ──────────────────────────────────────────────────────────────────

/// 活跃 sandbox 的 handle。所有方法均发 API 调用,可安全并发使用。
///
/// 通过 [`Sandbox::create`] / [`Sandbox::get`] / [`Sandbox::list`] 工厂函数获取实例。
pub struct Sandbox {
    /// sandbox 元信息缓存(由 [`Sandbox::refresh`] 更新)。
    pub(crate) info: SandboxInfo,
    /// 底层 HTTP 客户端(可 Clone,内部共享连接池)。
    pub(crate) client: Client,
}

// ─── 访问器 ───────────────────────────────────────────────────────────────────

impl Sandbox {
    /// 返回 sandbox 唯一标识符(如 `"sbx_abc123"`)。
    pub fn id(&self) -> &str {
        &self.info.id
    }

    /// 返回最近已知的 sandbox 状态(由上次 API 调用或 [`Sandbox::refresh`] 缓存)。
    pub fn state(&self) -> SandboxState {
        self.info.state
    }

    /// 返回完整的 sandbox 元信息快照(缓存值,非实时)。
    pub fn info(&self) -> &SandboxInfo {
        &self.info
    }
}

// ─── 工厂函数 ─────────────────────────────────────────────────────────────────

impl Sandbox {
    /// 创建新 sandbox,使用全局默认 client。
    ///
    /// 等待 sandbox 进入 `running` 状态后返回。
    ///
    /// # 示例
    ///
    /// ```no_run
    /// use talon_sandbox::{Sandbox, CreateOpts, Resources};
    ///
    /// # async fn demo() -> talon_sandbox::Result<()> {
    /// let sb = Sandbox::create(CreateOpts {
    ///     image: Some("node:20-bookworm".into()),
    ///     resources: Resources { cpu: 2.0, memory: Some("4GiB".into()), ..Default::default() },
    ///     network: Some("allowlist".into()),
    ///     ..Default::default()
    /// }).await?;
    /// # let _ = sb;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create(opts: CreateOpts) -> Result<Sandbox> {
        Self::create_with(default_client(), opts).await
    }

    /// 创建新 sandbox,使用显式 client(多租户 / 多 server 场景)。
    pub async fn create_with(client: Client, opts: CreateOpts) -> Result<Sandbox> {
        let body = build_create_body(opts)?;
        // POST /v1/sandboxes?wait=running —— 服务端等 sandbox 进入 running 再响应
        let info: SandboxInfo = client.post("/v1/sandboxes?wait=running", &body).await?;
        Ok(Sandbox { info, client })
    }

    /// 通过 ID 取得已有 sandbox,使用全局默认 client。
    pub async fn get(id: &str) -> Result<Sandbox> {
        Self::get_with(default_client(), id).await
    }

    /// 通过 ID 取得已有 sandbox,使用显式 client。
    pub async fn get_with(client: Client, id: &str) -> Result<Sandbox> {
        let path = format!("/v1/sandboxes/{id}");
        let info: SandboxInfo = client.get(&path).await?;
        Ok(Sandbox { info, client })
    }

    /// 列出当前租户下的所有 sandbox,使用全局默认 client。
    ///
    /// 客户端侧按 `opts.labels` 过滤(与 Go `matchesLabels` 对齐)。
    pub async fn list(opts: ListOpts) -> Result<Vec<Sandbox>> {
        Self::list_with(default_client(), opts).await
    }

    /// 列出当前租户下的所有 sandbox,使用显式 client。
    ///
    /// 当 `opts.labels` 非空时,将每对 `(key, value)` 编码为 `label=key:value` query 参数
    /// 发送给服务端(AND 语义,服务端只返回匹配全部 label 的 sandbox)。
    /// 响应回来后仍保留客户端侧 [`matches_labels`] 二次校验,兼容不支持服务端 label 过滤
    /// 的老版本后端。
    pub async fn list_with(client: Client, opts: ListOpts) -> Result<Vec<Sandbox>> {
        // 将 labels HashMap 转换为 reqwest query 参数列表,格式 label=key:value。
        // 预先分配容量避免多次 realloc。
        let label_params: Vec<(&str, String)> = opts
            .labels
            .iter()
            .map(|(k, v)| ("label", format!("{k}:{v}")))
            .collect();

        // 把 Vec<(&str, String)> 转为 Vec<(&str, &str)> 以符合 get_with_query 签名。
        let params: Vec<(&str, &str)> = label_params
            .iter()
            .map(|(name, val)| (*name, val.as_str()))
            .collect();

        // labels 非空时附带 query 参数;为空时走普通 get(URL 更干净)。
        let dto: SandboxListDto = if params.is_empty() {
            client.get("/v1/sandboxes").await?
        } else {
            client.get_with_query("/v1/sandboxes", &params).await?
        };

        // 客户端侧二次过滤:老后端兜底 + 双保险。
        let sandboxes = dto
            .sandboxes
            .into_iter()
            .filter(|info| matches_labels(&info.labels, &opts.labels))
            .map(|info| Sandbox {
                info,
                client: client.clone(),
            })
            .collect();
        Ok(sandboxes)
    }
}

// ─── 生命周期方法 ─────────────────────────────────────────────────────────────

impl Sandbox {
    /// 暂停 sandbox 内所有进程。对齐 Go `Pause`。
    pub async fn pause(&self) -> Result<()> {
        let path = format!("/v1/sandboxes/{}/pause", self.info.id);
        // pause/resume 无请求体,传空 JSON 对象满足 Content-Type 要求。
        self.client
            .post_no_content(&path, &serde_json::json!({}))
            .await
    }

    /// 恢复已暂停的 sandbox。对齐 Go `Resume`。
    pub async fn resume(&self) -> Result<()> {
        let path = format!("/v1/sandboxes/{}/resume", self.info.id);
        self.client
            .post_no_content(&path, &serde_json::json!({}))
            .await
    }

    /// 启动已停止的 sandbox,使其重新进入 running 状态。
    ///
    /// 对应端点:`POST /v1/sandboxes/{id}/start`,返回 204。
    /// 与 pause/resume 不同:stop/start 是更重量级的停止——进程被终止,
    /// 而 pause/resume 只是冻结进程(cgroup freeze)。
    pub async fn start(&self) -> Result<()> {
        let path = format!("/v1/sandboxes/{}/start", self.info.id);
        self.client
            .post_no_content(&path, &serde_json::json!({}))
            .await
    }

    /// 停止 sandbox(running→stopped),终止所有进程但保留文件系统。
    ///
    /// 对应端点:`POST /v1/sandboxes/{id}/stop`,返回 204。
    /// 与 `pause` 不同:stop 是真正终止进程,可通过 [`Sandbox::start`] 重新启动。
    pub async fn stop(&self) -> Result<()> {
        let path = format!("/v1/sandboxes/{}/stop", self.info.id);
        self.client
            .post_no_content(&path, &serde_json::json!({}))
            .await
    }

    /// 永久销毁 sandbox(DELETE)。对齐 Go `Kill`。
    pub async fn kill(&self) -> Result<()> {
        let path = format!("/v1/sandboxes/{}", self.info.id);
        self.client.delete(&path).await
    }

    /// 从 API 拉取最新状态,更新 `self.info` 缓存并返回新快照。对齐 Go `Refresh`。
    ///
    /// 注意:需要 `&mut self` 以写入 info 缓存。
    pub async fn refresh(&mut self) -> Result<SandboxInfo> {
        let path = format!("/v1/sandboxes/{}", self.info.id);
        let info: SandboxInfo = self.client.get(&path).await?;
        self.info = info.clone();
        Ok(info)
    }
}

// ─── 子资源 handle ────────────────────────────────────────────────────────────

impl Sandbox {
    /// 返回文件系统操作 handle。对齐 Go `FS()`。
    pub fn fs(&self) -> Fs {
        Fs::new(self.info.id.clone(), self.client.clone())
    }

    /// 返回环境变量操作 handle。对齐 Go `Env()`。
    pub fn env(&self) -> Env {
        Env::new(self.info.id.clone(), self.client.clone())
    }

    /// 返回 PTY 终端 handle。对齐 Go `Terminal()`。
    pub fn terminal(&self) -> Terminal {
        Terminal::new(self.info.id.clone(), self.client.clone())
    }

    /// 返回 headless 浏览器会话 handle(Spec 34)。
    ///
    /// 通过返回的 [`Browser`] 可以启动/查询/停止 sandbox 内的 Chromium 进程。
    pub fn browser(&self) -> Browser {
        Browser::new(self.info.id.clone(), self.client.clone())
    }
}

// ─── 端口暴露(转调 expose 模块) ──────────────────────────────────────────────

impl Sandbox {
    /// 暴露 sandbox 内部端口,返回公开访问 URL。对齐 Go Expose 逻辑。
    pub async fn expose(&self, port: u16, opts: ExposeOpts) -> Result<String> {
        expose::expose(&self.client, &self.info.id, port, &opts).await
    }

    /// 取消暴露指定端口。
    pub async fn unexpose(&self, port: u16) -> Result<()> {
        expose::unexpose(&self.client, &self.info.id, port).await
    }

    /// 返回当前所有已暴露端口信息。
    pub async fn exposed(&self) -> Result<Vec<ExposedPort>> {
        expose::exposed(&self.client, &self.info.id).await
    }
}

// ─── 辅助:构建 create body ────────────────────────────────────────────────────

/// 将 [`CreateOpts`] 转换成 API wire 格式(serde_json::Value)。对齐 Go `buildCreateBody`。
///
/// - `image` → `image_id`
/// - `network` 经 [`normalize_network`] 映射 → `network_policy`
/// - `resources.cpu` * 1000 → `cpu_millis`(i64)
/// - `resources.memory` 经 [`parse_size`] → `memory_bytes`
/// - `resources.disk`   经 [`parse_size`] → `disk_bytes`
/// - `timeout` 经 [`parse_duration`] → `idle_timeout_seconds`
/// - `ttl`     经 [`parse_duration`] → `ttl_seconds`
pub(crate) fn build_create_body(opts: CreateOpts) -> Result<Value> {
    let mut body = serde_json::Map::new();

    // 镜像 ID
    if let Some(img) = opts.image {
        if !img.is_empty() {
            body.insert("image_id".into(), Value::String(img));
        }
    }

    // 网络策略(友好名称→规范值)
    if let Some(net) = opts.network {
        let canonical = normalize_network(&net).to_string();
        if !canonical.is_empty() {
            body.insert("network_policy".into(), Value::String(canonical));
        }
    }

    // per-sandbox 出站白名单(restricted-egress 下生效;非空覆盖全局)
    if !opts.network_allowed_hosts.is_empty() {
        let hosts: Vec<Value> = opts
            .network_allowed_hosts
            .into_iter()
            .map(Value::String)
            .collect();
        body.insert("network_allowed_hosts".into(), Value::Array(hosts));
    }

    // 环境变量(HashMap<String,String>)
    if !opts.env.is_empty() {
        // 服务端接受 {"KEY": "val"} map 格式(与 Go Env map 对齐)
        let env_map: serde_json::Map<_, _> = opts
            .env
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        body.insert("env".into(), Value::Object(env_map));
    }

    // 标签
    if !opts.labels.is_empty() {
        let labels_map: serde_json::Map<_, _> = opts
            .labels
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        body.insert("labels".into(), Value::Object(labels_map));
    }

    // CPU:f64 核数 × 1000 → cpu_millis i64
    let r = opts.resources;
    if r.cpu != 0.0 {
        let cpu_millis = (r.cpu * 1000.0) as i64;
        body.insert("cpu_millis".into(), Value::Number(cpu_millis.into()));
    }

    // 内存
    if let Some(mem) = r.memory {
        if !mem.is_empty() {
            let bytes = parse_size(&mem)?;
            body.insert("memory_bytes".into(), Value::Number(bytes.into()));
        }
    }

    // 磁盘
    if let Some(disk) = r.disk {
        if !disk.is_empty() {
            let bytes = parse_size(&disk)?;
            body.insert("disk_bytes".into(), Value::Number(bytes.into()));
        }
    }

    // idle 超时
    if let Some(timeout) = opts.timeout {
        if !timeout.is_empty() {
            let secs = parse_duration(&timeout)?;
            body.insert("idle_timeout_seconds".into(), Value::Number(secs.into()));
        }
    }

    // 硬性 TTL
    if let Some(ttl) = opts.ttl {
        if !ttl.is_empty() {
            let secs = parse_duration(&ttl)?;
            body.insert("ttl_seconds".into(), Value::Number(secs.into()));
        }
    }

    Ok(Value::Object(body))
}

// ─── 辅助:labels 过滤 ────────────────────────────────────────────────────────

/// 检查 sandbox 的 labels 是否包含所有 want 中的 k=v 对。对齐 Go `matchesLabels`。
fn matches_labels(
    have: &std::collections::HashMap<String, String>,
    want: &std::collections::HashMap<String, String>,
) -> bool {
    want.iter().all(|(k, v)| have.get(k) == Some(v))
}

// ─── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // ── matches_labels ──────────────────────────────────────────────────────

    /// want 为空时始终匹配(列出全部)。
    #[test]
    fn matches_labels_empty_want() {
        let have: HashMap<String, String> = [("env".into(), "prod".into())].into();
        assert!(matches_labels(&have, &HashMap::new()));
    }

    /// have 包含 want 的全部 k=v → 匹配。
    #[test]
    fn matches_labels_subset_match() {
        let have: HashMap<String, String> = [
            ("env".into(), "prod".into()),
            ("team".into(), "infra".into()),
        ]
        .into();
        let want: HashMap<String, String> = [("env".into(), "prod".into())].into();
        assert!(matches_labels(&have, &want));
    }

    /// want 中有一个 key 对应值不同 → 不匹配。
    #[test]
    fn matches_labels_value_mismatch() {
        let have: HashMap<String, String> = [("env".into(), "prod".into())].into();
        let want: HashMap<String, String> = [("env".into(), "staging".into())].into();
        assert!(!matches_labels(&have, &want));
    }

    /// want 中有 key 在 have 里不存在 → 不匹配。
    #[test]
    fn matches_labels_missing_key() {
        let have: HashMap<String, String> = [("env".into(), "prod".into())].into();
        let want: HashMap<String, String> = [
            ("env".into(), "prod".into()),
            ("region".into(), "cn".into()),
        ]
        .into();
        assert!(!matches_labels(&have, &want));
    }

    // ── label_params 编码 ───────────────────────────────────────────────────

    /// 验证服务端 query 参数格式:label=key:value,冒号分隔(value 可含等号)。
    #[test]
    fn label_params_format() {
        let labels: HashMap<String, String> = [("env".into(), "prod".into())].into();
        let params: Vec<(&str, String)> = labels
            .iter()
            .map(|(k, v)| ("label", format!("{k}:{v}")))
            .collect();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].0, "label");
        assert_eq!(params[0].1, "env:prod");
    }

    /// value 中含等号时冒号分隔不会歧义。
    #[test]
    fn label_params_value_with_equals() {
        let labels: HashMap<String, String> = [("filter".into(), "x=1".into())].into();
        let params: Vec<(&str, String)> = labels
            .iter()
            .map(|(k, v)| ("label", format!("{k}:{v}")))
            .collect();
        assert_eq!(params[0].1, "filter:x=1");
    }

    /// labels 为空时不产生任何 query 参数。
    #[test]
    fn label_params_empty() {
        let labels: HashMap<String, String> = HashMap::new();
        let params: Vec<(&str, String)> = labels
            .iter()
            .map(|(k, v)| ("label", format!("{k}:{v}")))
            .collect();
        assert!(params.is_empty());
    }
}
