//! Sandbox 内 AI agent 运行(Spec 38)。
//!
//! [`Sandbox::agent_run`] 向后端提交一个自然语言目标,由 browser-harness 驱动
//! headless Chromium 完成多步骤任务,同步阻塞直到结束(最长 5 分钟)。
//!
//! # 注意
//!
//! - LLM API key 不在请求体里传递——应通过 Spec 27 secrets 注入到 sandbox env。
//! - `max_steps` 服务端硬上限 100;传更大值会被钳到 100。
//! - `status == "completed"` 不代表任务成功,仅表示进程 `exit_code == 0`。
//!   是否达成目标看 `result` 字段(LLM 自我评估)。

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::errors::Result;
use crate::sandbox::Sandbox;

/// agent_run 端点的超时:服务端最多跑 5 分钟,留 30s 余量。
const AGENT_RUN_TIMEOUT: Duration = Duration::from_secs(330);

// ─── 请求 ─────────────────────────────────────────────────────────────────────

/// `agent_run` 的可选配置参数。
#[derive(Debug, Clone, Default)]
pub struct AgentRunOpts {
    /// 最大步骤数,默认 20,服务端硬上限 100。
    pub max_steps: Option<i32>,
    /// 模型 hint 字符串,如 `"anthropic:claude-sonnet-4-6"`。
    /// 仅作提示,browser-harness 内部决定是否生效。
    pub llm_model: Option<String>,
}

/// 发送给服务端的 wire body,对齐 dto.go `AgentRunRequest`。
#[derive(Serialize)]
struct AgentRunRequest<'a> {
    goal: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_steps: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    llm_model: Option<&'a str>,
}

// ─── 响应 ─────────────────────────────────────────────────────────────────────

/// Agent 运行的单步骤记录,对齐 dto.go `AgentRunStep`。
#[derive(Debug, Clone, Deserialize)]
pub struct AgentRunStep {
    /// 步骤编号(从 1 开始)。
    pub step: i32,
    /// 动作类型,如 `"Page.navigate"` / `"Input.click"` / `"result"`。
    pub action: String,
    /// LLM 对本步骤的解释(可选)。
    #[serde(default)]
    pub thought: String,
    /// action 特定附加字段(松散 map)。
    #[serde(default)]
    pub details: HashMap<String, serde_json::Value>,
}

/// `agent_run` 的同步响应,对齐 dto.go `AgentRunResponse`。
#[derive(Debug, Clone, Deserialize)]
pub struct AgentRunResponse {
    /// 运行 ID(如 `"run_abc123"`)。
    pub run_id: String,
    /// 运行状态:`"completed"` | `"failed"` | `"timeout"`。
    pub status: String,
    /// 总耗时(毫秒)。
    pub duration_ms: i64,
    /// 执行步骤列表。
    pub steps: Vec<AgentRunStep>,
    /// browser-harness 最后一步的 result 字段(LLM 自我评估);失败时可能为空。
    #[serde(default)]
    pub result: String,
    /// 进程退出码。0 = 正常退出。
    pub exit_code: i32,
    /// 失败时的 stderr;成功路径上通常为空。
    #[serde(default)]
    pub stderr: String,
}

// ─── Sandbox 方法 ─────────────────────────────────────────────────────────────

impl Sandbox {
    /// 在 sandbox 内同步运行 AI agent,完成自然语言描述的目标。
    ///
    /// 对应端点:`POST /v1/sandboxes/{id}/agent/run`。
    /// 同步阻塞,最长约 5 分钟。
    ///
    /// LLM API key 须提前通过 secrets 注入到 sandbox 环境变量,不在此参数传递。
    ///
    /// # 参数
    ///
    /// - `goal`: 自然语言目标描述,如 `"搜索 Rust tokio 最新版本并截图"`。
    /// - `opts`: 可选配置([`AgentRunOpts`])。
    ///
    /// # 示例
    ///
    /// ```no_run
    /// use talon_sandbox::{Sandbox, AgentRunOpts};
    ///
    /// # async fn demo() -> talon_sandbox::Result<()> {
    /// let sb = Sandbox::get("sbx_xxx").await?;
    /// let resp = sb.agent_run("截图 https://example.com", AgentRunOpts {
    ///     max_steps: Some(10),
    ///     ..Default::default()
    /// }).await?;
    /// println!("status={} steps={}", resp.status, resp.steps.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn agent_run(&self, goal: &str, opts: AgentRunOpts) -> Result<AgentRunResponse> {
        let path = format!("/v1/sandboxes/{}/agent/run", self.info.id);
        let body = AgentRunRequest {
            goal,
            max_steps: opts.max_steps,
            llm_model: opts.llm_model.as_deref(),
        };
        self.client.post_with_timeout(&path, &body, AGENT_RUN_TIMEOUT).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证 AgentRunResponse 反序列化对齐 dto.go AgentRunResponse。
    #[test]
    fn agent_run_response_deserialize() {
        let json = r#"{
            "run_id": "run_abc123",
            "status": "completed",
            "duration_ms": 12345,
            "steps": [
                {
                    "step": 1,
                    "action": "Page.navigate",
                    "thought": "打开目标页面",
                    "details": {"url": "https://example.com"}
                },
                {
                    "step": 2,
                    "action": "result",
                    "thought": "",
                    "details": {}
                }
            ],
            "result": "任务完成,已截图",
            "exit_code": 0,
            "stderr": ""
        }"#;
        let resp: AgentRunResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.run_id, "run_abc123");
        assert_eq!(resp.status, "completed");
        assert_eq!(resp.duration_ms, 12345);
        assert_eq!(resp.steps.len(), 2);
        assert_eq!(resp.steps[0].action, "Page.navigate");
        assert_eq!(resp.exit_code, 0);
        assert_eq!(resp.result, "任务完成,已截图");
    }

    /// 失败场景:stderr 非空,result 可选为空。
    #[test]
    fn agent_run_response_failed() {
        let json = r#"{
            "run_id": "run_err",
            "status": "failed",
            "duration_ms": 3000,
            "steps": [],
            "exit_code": 1,
            "stderr": "browser harness crashed"
        }"#;
        let resp: AgentRunResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "failed");
        assert_eq!(resp.exit_code, 1);
        assert_eq!(resp.stderr, "browser harness crashed");
        assert_eq!(resp.result, "");
    }

    /// AgentRunRequest wire 序列化:空 opts 不产生多余字段。
    #[test]
    fn agent_run_request_serialize_minimal() {
        let req = AgentRunRequest {
            goal: "hello",
            max_steps: None,
            llm_model: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["goal"], "hello");
        // 空 opts 字段不出现在 JSON 里
        assert!(json.get("max_steps").is_none());
        assert!(json.get("llm_model").is_none());
    }

    /// AgentRunRequest wire 序列化:opts 全填时正确输出。
    #[test]
    fn agent_run_request_serialize_full() {
        let req = AgentRunRequest {
            goal: "截图",
            max_steps: Some(10),
            llm_model: Some("anthropic:claude-sonnet-4-6"),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["max_steps"], 10);
        assert_eq!(json["llm_model"], "anthropic:claude-sonnet-4-6");
    }
}
