//! PTY 交互示例:打开终端,发几条命令,读回输出。
//!
//! 运行:
//!   TALON_SANDBOX_SERVER=... TALON_SANDBOX_API_KEY=... cargo run --example pty_loop

use std::time::Duration;

use talon_sandbox::{CreateOpts, Resources, Sandbox};

#[tokio::main]
async fn main() -> talon_sandbox::Result<()> {
    let sb = Sandbox::create(CreateOpts {
        image: Some("node:20-bookworm".into()),
        resources: Resources {
            cpu: 1.0,
            memory: Some("2GiB".into()),
            ..Default::default()
        },
        network: Some("allowlist".into()),
        ttl: Some("1h".into()),
        ..Default::default()
    })
    .await?;
    println!("sandbox: {}", sb.id());

    // 打开 PTY 会话。
    let mut pty = sb.terminal().open().await?;

    // 写入一条命令(注意末尾换行触发执行)。
    pty.write(b"ls -la /\n").await?;
    pty.write(b"echo done\n").await?;

    // 读回输出,直到看到 "done" 或超时。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, pty.recv()).await {
            Ok(Ok(Some(chunk))) => {
                print!("{}", String::from_utf8_lossy(&chunk));
                if chunk.windows(4).any(|w| w == b"done") {
                    break;
                }
            }
            Ok(Ok(None)) => break, // 对端关闭
            Ok(Err(e)) => {
                eprintln!("pty recv error: {e}");
                break;
            }
            Err(_) => break, // 超时
        }
    }

    pty.close().await?;
    sb.kill().await?;
    Ok(())
}
