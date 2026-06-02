//! 请求 / 响应类型,对齐其它语言 SDK(Go types.go)。
//!
//! 约定:面向用户的输入类型(`CreateOpts`/`Resources`)用人类可读字段
//! (CPU 浮点核数、Memory 字符串 "4GiB");wire 上的响应类型(`SandboxInfo` 等)
//! 用 serde 映射服务端 v1 风格的 snake_case JSON。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 创建 sandbox 的配置。字段全部可选,空值走服务端默认。
#[derive(Debug, Clone, Default)]
pub struct CreateOpts {
    /// 容器镜像引用,如 "talon-alpine"。空 = 系统默认镜像。
    pub image: Option<String>,
    /// 计算资源分配。
    pub resources: Resources,
    /// 网络策略:"allowlist" | "open" | "sealed"。空 = 服务端默认。
    pub network: Option<String>,
    /// per-sandbox 出站白名单(host / IP / CIDR),仅在 network 解析为 "allowlist"
    /// (restricted-egress)时生效。非空 → 覆盖 worker 启动期全局白名单;空 → 回退全局。
    /// 让调用方无需运维改 worker env 即可放行自己的域名(如 git 服务)。
    pub network_allowed_hosts: Vec<String>,
    /// 启动环境变量。
    pub env: HashMap<String, String>,
    /// idle 自动暂停超时,如 "30m"。空 = 不启用。
    pub timeout: Option<String>,
    /// 硬性 TTL(到点销毁),如 "6h"。空 = 不启用。
    pub ttl: Option<String>,
    /// 任意键值元数据标签。
    pub labels: HashMap<String, String>,
}

/// 计算资源,用人类可读字符串描述。
#[derive(Debug, Clone, Default)]
pub struct Resources {
    /// CPU 核数(整数或浮点)。2.0 = 2 核,0.5 = 半核。0 = 服务端默认。
    pub cpu: f64,
    /// 内存字符串:"4GiB"、"512MiB" 等。空 = 服务端默认。
    pub memory: Option<String>,
    /// 磁盘字符串:"10GiB" 等。空 = 服务端默认。
    pub disk: Option<String>,
}

/// 列表过滤条件。
#[derive(Debug, Clone, Default)]
pub struct ListOpts {
    /// 仅返回匹配全部指定 label 的 sandbox。
    pub labels: HashMap<String, String>,
}

/// 端口暴露的可选配置。
#[derive(Debug, Clone, Default)]
pub struct ExposeOpts {
    /// 是否申请签名 preview URL(Spec 48)。
    pub sign: bool,
    /// 签名 URL 有效期,如 "1h"。仅 sign=true 时有意义。
    pub ttl: Option<String>,
    /// 自定义子域名前缀(默认随机)。
    pub subdomain: Option<String>,
}

/// sandbox 状态枚举,对齐服务端 state 取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SandboxState {
    Created,
    Running,
    Paused,
    Stopped,
    Destroyed,
    Killed,
    Lost,
    /// 未知 / 未来新增状态(向前兼容,避免反序列化失败)。
    #[serde(other)]
    Unknown,
}

/// sandbox 读模型(API 返回)。serde 映射服务端 v1 风格 JSON。
#[derive(Debug, Clone, Deserialize)]
pub struct SandboxInfo {
    pub id: String,
    pub state: SandboxState,
    #[serde(rename = "image_id", default)]
    pub image: String,
    #[serde(default)]
    pub cpu_millis: i64,
    #[serde(default)]
    pub memory_bytes: i64,
    #[serde(default)]
    pub idle_timeout_seconds: i64,
    #[serde(default)]
    pub ttl_seconds: i64,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub network_policy: String,
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

/// 同步 Run 调用的结果。
#[derive(Debug, Clone)]
pub struct ProcessResult {
    /// 进程退出码。
    pub exit_code: i32,
    /// stdout+stderr 合并输出(来自进程日志端点)。
    pub combined: String,
}

/// sandbox 内运行的进程信息。
#[derive(Debug, Clone, Deserialize)]
pub struct ProcessInfo {
    pub id: String,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub pid: i32,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub exit_code: i32,
}

/// 当前暴露的端口。
#[derive(Debug, Clone, Deserialize)]
pub struct ExposedPort {
    pub port: i32,
    pub url: String,
    #[serde(default)]
    pub signed: bool,
    #[serde(default)]
    pub source: String, // "explicit" | "dynamic"
}

/// 文件系统条目(文件或目录)。
#[derive(Debug, Clone, Deserialize)]
pub struct FsEntry {
    pub name: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default)]
    pub mod_time: i64,
    #[serde(default)]
    pub is_dir: bool,
}

// ─── 内部 wire DTO(仅 crate 内反序列化用) ────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct ProcessListDto {
    #[serde(default)]
    pub processes: Vec<ProcessInfo>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct FsListDto {
    #[serde(default)]
    pub entries: Vec<FsEntry>,
    /// 服务端返回的总条数(分页用);当前 SDK 只取 entries,保留字段对齐 wire。
    #[serde(default)]
    #[allow(dead_code)]
    pub total: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SandboxListDto {
    #[serde(default)]
    pub sandboxes: Vec<SandboxInfo>,
}

/// 服务端错误响应体 `{"error": "..."}`。
#[derive(Debug, Deserialize)]
pub(crate) struct ErrorBody {
    #[serde(default)]
    pub error: String,
}
