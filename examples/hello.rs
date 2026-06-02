//! 最小示例:创建 sandbox → 跑一条命令 → 暴露端口 → 清理。
//!
//! 运行:
//!   TALON_SANDBOX_SERVER=https://api.example.com \
//!   TALON_SANDBOX_API_KEY=ask_xxx \
//!   cargo run --example hello

use talon_sandbox::{CreateOpts, Resources, Sandbox};

#[tokio::main]
async fn main() -> talon_sandbox::Result<()> {
    // server / api_key 默认从 TALON_SANDBOX_SERVER / TALON_SANDBOX_API_KEY 读取。
    let sb = Sandbox::create(CreateOpts {
        image: Some("talon-alpine".into()),
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

    let result = sb.run("node -e 'console.log(\"hello from sandbox\")'").await?;
    print!("{}", result.combined);

    match sb.expose(3000, Default::default()).await {
        Ok(url) => println!("Preview URL: {url}"),
        Err(e) => eprintln!("expose: {e}"),
    }

    sb.kill().await?;
    Ok(())
}
