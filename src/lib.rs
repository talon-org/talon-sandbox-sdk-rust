//! # talon-sandbox
//!
//! Rust SDK for [Talon Sandbox](https://talon-sandbox.dev) —— 为 AI agent 提供的
//! 隔离容器环境。与 TypeScript / Go / Python / .NET SDK 行为对齐。
//!
//! ## 快速开始
//!
//! ```no_run
//! use talon_sandbox::{Sandbox, CreateOpts, Resources};
//!
//! #[tokio::main]
//! async fn main() -> talon_sandbox::Result<()> {
//!     // server / api_key 默认从 TALON_SANDBOX_SERVER / TALON_SANDBOX_API_KEY 读取。
//!     let sb = Sandbox::create(CreateOpts {
//!         image: Some("talon-alpine".into()),
//!         resources: Resources { cpu: 2.0, memory: Some("4GiB".into()), ..Default::default() },
//!         network: Some("allowlist".into()),
//!         timeout: Some("30m".into()),
//!         ttl: Some("6h".into()),
//!         ..Default::default()
//!     })
//!     .await?;
//!
//!     // 同步执行命令
//!     let result = sb.run("npm install").await?;
//!     println!("exit={} out={}", result.exit_code, result.combined);
//!
//!     sb.kill().await?;
//!     Ok(())
//! }
//! ```
//!
//! ## 全局配置
//!
//! 不想每次传 client,可用 [`configure`] 设置进程级默认:
//!
//! ```no_run
//! talon_sandbox::configure(
//!     talon_sandbox::Config::new("https://api.example.com").api_key("ask_..."),
//! );
//! ```

mod agent;
mod browser;
mod client;
mod config;
mod env;
mod errors;
mod expose;
mod fs;
mod images;
mod parse;
mod process;
mod sandbox;
mod terminal;
mod types;

// ─── 公开 API ───────────────────────────────────────────────────────────────

pub use agent::{AgentRunOpts, AgentRunResponse, AgentRunStep};
pub use browser::{Browser, BrowserSession};
pub use client::{Client, ClientBuilder};
pub use config::{configure, Config};
pub use env::Env;
pub use errors::{Error, Result};
pub use fs::Fs;
pub use images::{list_images, list_images_with, ImageInfo};
pub use parse::{parse_duration, parse_size};
pub use process::{SpawnOpts, SpawnedProcess};
pub use sandbox::Sandbox;
pub use terminal::{PtySession, Terminal};
pub use types::{
    CreateOpts, ExposeOpts, ExposedPort, FsEntry, ListOpts, ProcessInfo, ProcessResult, Resources,
    SandboxInfo, SandboxState,
};
