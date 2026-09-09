//! 进程执行模块,对齐 Go SDK run_spawn.go。
//!
//! 提供两种执行模式:
//! - [`Sandbox::run`] / [`Sandbox::run_with`]:同步执行,阻塞至进程退出,返回 [`ProcessResult`]。
//! - [`Sandbox::spawn`]:异步启动,立即返回 [`SpawnedProcess`] handle,由调用方驱动。
//!
//! 服务端没有 GET /processes/{id} 单进程端点,每次轮询要拉 LIST 再过滤,
//! 与 Go SDK 保持一致(默认 500ms 间隔)。

use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use tokio::time::sleep;

use crate::errors::Result;
use crate::sandbox::Sandbox;
use crate::types::{ProcessListDto, ProcessResult};

// ─── 轮询间隔(可测试时覆盖) ──────────────────────────────────────────────────

/// 进程状态轮询间隔,默认 500ms,与其它语言 SDK 保持一致。
///
/// 仅在测试中需要加速时修改;生产代码请保持默认。
pub static POLL_INTERVAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(500);

fn poll_interval() -> Duration {
    Duration::from_millis(POLL_INTERVAL.load(std::sync::atomic::Ordering::Relaxed))
}

// ─── RunOpts / SpawnOpts ──────────────────────────────────────────────────────

/// 同步 [`Sandbox::run`] 的可选配置。
#[derive(Debug, Clone, Default)]
pub struct RunOpts {
    /// sandbox 内的工作目录,空 = 不设(使用镜像默认)。
    pub cwd: Option<String>,
    /// 追加环境变量,格式 `"KEY=value"`。
    pub env: Vec<String>,
}

/// 异步 [`Sandbox::spawn`] 的可选配置。
#[derive(Debug, Clone, Default)]
pub struct SpawnOpts {
    /// sandbox 内的工作目录。
    pub cwd: Option<String>,
    /// 追加环境变量,格式 `"KEY=value"`。
    pub env: Vec<String>,
    /// 进程声明对外暴露的容器端口(如 `[5173]`)。runc adapter 据此建立
    /// 容器→host 的 DNAT 映射,预览反向代理的端口准入 + 路由都依赖它
    /// (见服务端 `expose_ports`/`host_ports`)。空 = 不声明。
    pub expose_ports: Vec<i32>,
}

// ─── 内部 wire DTO ────────────────────────────────────────────────────────────

/// 进程创建响应中的最小字段(只需 id)。
#[derive(Debug, Deserialize)]
struct ProcessDto {
    id: String,
    // 服务端可能返回更多字段,忽略即可
}

// ─── Run(同步) ─────────────────────────────────────────────────────────────────

impl Sandbox {
    /// 在 sandbox 内同步执行命令,等待退出后返回 [`ProcessResult`]。
    ///
    /// 命令经 `/bin/sh -c` 解释,支持管道、重定向等 shell 语法。
    /// 对齐 Go `Run(ctx, command)`。
    pub async fn run(&self, command: &str) -> Result<ProcessResult> {
        self.run_with(command, RunOpts::default()).await
    }

    /// [`Sandbox::run`] 的带参数版本。
    pub async fn run_with(&self, command: &str, opts: RunOpts) -> Result<ProcessResult> {
        // 构造请求体:command 数组 + 可选 cwd/env
        let mut body = serde_json::json!({
            "command": ["/bin/sh", "-c", command],
        });
        if let Some(cwd) = &opts.cwd {
            body["cwd"] = serde_json::Value::String(cwd.clone());
        }
        if !opts.env.is_empty() {
            body["env"] = serde_json::json!(opts.env);
        }

        let path = format!("/v1/sandboxes/{}/processes", self.info.id);
        let proc: ProcessDto = self.client.post(&path, &body).await?;

        self.wait_process(&proc.id).await
    }

    /// 轮询 LIST 端点直到目标进程退出,再拉 combined 日志。
    /// 对齐 Go `waitProcess`。
    pub(crate) async fn wait_process(&self, proc_id: &str) -> Result<ProcessResult> {
        loop {
            let list_path = format!("/v1/sandboxes/{}/processes", self.info.id);
            let list: ProcessListDto = self.client.get(&list_path).await?;

            // 在列表里找目标进程
            if let Some(found) = list.processes.iter().find(|p| p.id == proc_id) {
                match found.state.as_str() {
                    "exited" | "killed" | "failed" => {
                        let exit_code = found.exit_code;
                        let logs = self.fetch_process_logs(proc_id).await;
                        // 若 logs 拉取失败但已有退出状态,仍返回 ProcessResult,
                        // 只有 logs 完全拉不到且 combined 为空时才传播 log 错误。
                        // 对齐 Go 的注释:"Don't lose the log fetch error"。
                        let (combined, log_err) = match logs {
                            Ok(bytes) => (String::from_utf8_lossy(&bytes).into_owned(), None),
                            Err(e) => (String::new(), Some(e)),
                        };
                        if combined.is_empty() {
                            if let Some(e) = log_err {
                                return Err(e);
                            }
                        }
                        return Ok(ProcessResult {
                            exit_code,
                            combined,
                        });
                    }
                    _ => {} // 仍在运行,继续轮询
                }
            } else {
                // 进程已从列表消失(在 spawn 和 poll 之间退出),拉残余日志,exit_code=-1
                let combined = self
                    .fetch_process_logs(proc_id)
                    .await
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default();
                return Ok(ProcessResult {
                    exit_code: -1,
                    combined,
                });
            }

            // 等待下一个轮询周期
            sleep(poll_interval()).await;
        }
    }

    /// GET /v1/sandboxes/{id}/processes/{pid}/logs,返回原始字节。
    /// 对齐 Go `fetchProcessLogs`。
    pub(crate) async fn fetch_process_logs(&self, proc_id: &str) -> Result<Vec<u8>> {
        let path = format!("/v1/sandboxes/{}/processes/{}/logs", self.info.id, proc_id);
        self.client.get_bytes(&path).await
    }
}

// ─── Spawn(异步) ──────────────────────────────────────────────────────────────

/// 异步启动进程的 handle。对齐 Go `Process`。
///
/// # 回调简化决策
///
/// Go SDK 提供 `OnStdout(fn([]byte))` 和 `OnExit(fn(int))` 回调,但服务端无实时日志流端点
/// (日志只能在进程退出后通过 GET .../logs 拿到),回调实际上没有实时语义价值。
/// Rust 版简化为:`wait` 阻塞至进程退出并返回退出码,`exit_code` 在 wait 完成后
/// 取最终退出码。实时日志流(SSE/WebSocket)留待后续迭代。
pub struct SpawnedProcess {
    /// 平台分配的进程 ID(如 `"proc_abc123"`)。
    id: String,
    /// 所属 sandbox ID。
    sandbox_id: String,
    /// 底层 HTTP 客户端。
    client: crate::client::Client,
    /// wait() 填入的退出码;None = 尚未退出。
    exit_code: Mutex<Option<i32>>,
}

impl SpawnedProcess {
    /// 返回平台进程 ID。
    pub fn id(&self) -> &str {
        &self.id
    }

    /// 返回退出码(仅 [`wait`][SpawnedProcess::wait] 完成后有效,否则返回 `None`)。
    pub fn exit_code(&self) -> Option<i32> {
        *self.exit_code.lock().expect("exit_code lock poisoned")
    }

    /// 向进程发送 kill 信号(DELETE)。对齐 Go `Kill`。
    pub async fn kill(&self) -> Result<()> {
        let path = format!("/v1/sandboxes/{}/processes/{}", self.sandbox_id, self.id);
        self.client.delete(&path).await
    }

    /// 阻塞至进程退出,返回退出码。对齐 Go `Wait`。
    ///
    /// 多次调用安全:第二次调用会直接返回已缓存的退出码而无需再轮询。
    pub async fn wait(&self) -> Result<i32> {
        // 如果已有缓存的退出码,直接返回
        if let Some(code) = self.exit_code() {
            return Ok(code);
        }

        loop {
            let list_path = format!("/v1/sandboxes/{}/processes", self.sandbox_id);
            let list: ProcessListDto = self.client.get(&list_path).await?;

            if let Some(found) = list.processes.iter().find(|p| p.id == self.id) {
                match found.state.as_str() {
                    "exited" | "killed" | "failed" => {
                        let code = found.exit_code;
                        *self.exit_code.lock().expect("exit_code lock poisoned") = Some(code);
                        return Ok(code);
                    }
                    _ => {}
                }
            } else {
                // 进程已消失,记为 -1
                *self.exit_code.lock().expect("exit_code lock poisoned") = Some(-1);
                return Ok(-1);
            }

            sleep(poll_interval()).await;
        }
    }
}

impl Sandbox {
    /// 异步启动长期运行进程,立即返回 [`SpawnedProcess`] handle。
    ///
    /// 命令按空白分割(不经 shell 解释);如需 shell 语法请用 `spawn("sh -c '...'", ...)`
    /// 或直接用 [`Sandbox::run`]。对齐 Go `Spawn`。
    pub async fn spawn(&self, command: &str) -> Result<SpawnedProcess> {
        self.spawn_with(command, SpawnOpts::default()).await
    }

    /// [`Sandbox::spawn`] 的带参数版本。
    pub async fn spawn_with(&self, command: &str, opts: SpawnOpts) -> Result<SpawnedProcess> {
        // 命令按空白分割(不经 shell),对齐 Go strings.Fields
        let args: Vec<&str> = command.split_whitespace().collect();

        let mut body = serde_json::json!({ "command": args });
        if let Some(cwd) = &opts.cwd {
            body["cwd"] = serde_json::Value::String(cwd.clone());
        }
        if !opts.env.is_empty() {
            body["env"] = serde_json::json!(opts.env);
        }
        if !opts.expose_ports.is_empty() {
            body["expose_ports"] = serde_json::json!(opts.expose_ports);
        }

        let path = format!("/v1/sandboxes/{}/processes", self.info.id);
        let proc: ProcessDto = self.client.post(&path, &body).await?;

        Ok(SpawnedProcess {
            id: proc.id,
            sandbox_id: self.info.id.clone(),
            client: self.client.clone(),
            exit_code: Mutex::new(None),
        })
    }
}
