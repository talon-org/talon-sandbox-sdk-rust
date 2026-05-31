//! Vibe coding 完整工作流:创建 sandbox → 写入(AI 生成的)代码 → 装依赖 →
//! 起 dev server → 暴露端口拿 preview URL → 清理。
//!
//! 这正是「云端 vibe coding 项目接入 sandbox」的典型用法:
//!   - 打开项目 = Sandbox::create(带 timeout/ttl 自动回收兜底)
//!   - AI 改代码  = fs().write() 增量写文件,容器不重建
//!   - 起预览    = spawn dev server + expose 端口
//!   - 离开项目  = sb.kill()(或靠 idle timeout / TTL 自动回收)
//!
//! 运行:
//!   TALON_SANDBOX_SERVER=... TALON_SANDBOX_API_KEY=... cargo run --example vibe_coding

use std::time::Duration;

use talon_sandbox::{CreateOpts, Resources, Sandbox};

#[tokio::main]
async fn main() -> talon_sandbox::Result<()> {
    let mut labels = std::collections::HashMap::new();
    labels.insert("project".to_string(), "vibe-coding".to_string());

    let mut env = std::collections::HashMap::new();
    env.insert("NODE_ENV".to_string(), "development".to_string());

    let sb = Sandbox::create(CreateOpts {
        image: Some("node:20-bookworm".into()),
        resources: Resources {
            cpu: 2.0,
            memory: Some("4GiB".into()),
            disk: Some("10GiB".into()),
        },
        network: Some("allowlist".into()),
        // per-sandbox 出站白名单:只放行 npm registry,装依赖够用又不开全网。
        network_allowed_hosts: vec!["registry.npmjs.org".into()],
        env,
        timeout: Some("30m".into()), // idle 自动暂停兜底
        ttl: Some("6h".into()),      // 硬性销毁兜底
        labels,
    })
    .await?;
    println!("Sandbox created: {}", sb.id());

    // 写入 AI 生成的代码(增量写文件,不重建容器)。
    let app_code = br#"
const express = require('express')
const app = express()
app.get('/', (req, res) => res.send('hello from talon sandbox!'))
app.listen(3000, () => console.log('listening on :3000'))
"#;
    sb.fs().write("/workspace/index.js", app_code.to_vec()).await?;
    sb.fs()
        .write_text(
            "/workspace/package.json",
            r#"{"name":"demo","version":"1.0.0","dependencies":{"express":"^4"}}"#,
        )
        .await?;

    // 同步装依赖。
    let result = sb.run("cd /workspace && npm install").await?;
    println!("npm install exit: {}", result.exit_code);

    // 起 dev server(spawn 异步,立即返回句柄)。
    let proc = sb.spawn("node /workspace/index.js").await?;
    tokio::time::sleep(Duration::from_secs(1)).await;

    // 暴露 3000 端口拿公开 preview URL。
    match sb.expose(3000, Default::default()).await {
        Ok(url) => println!("Preview URL: {url}"),
        Err(e) => println!("expose not available: {e}"),
    }

    // 跑一会儿再清理。
    tokio::time::sleep(Duration::from_secs(5)).await;
    let _ = proc.kill().await;
    sb.kill().await?;
    println!("cleaned up");
    Ok(())
}
