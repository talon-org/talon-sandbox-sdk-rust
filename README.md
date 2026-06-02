# talon-sandbox-sdk-rust

Rust SDK for [Talon Sandbox](https://talon-sandbox.dev) — isolated container environments for AI agents.

行为与 [TypeScript](https://github.com/talon-org/talon-sandbox-sdk-typescript) /
[Go](https://github.com/talon-org/talon-sandbox-sdk-go) /
[Python](https://github.com/talon-org/talon-sandbox-sdk-python) /
[.NET](https://github.com/talon-org/talon-sandbox-sdk-dotnet) SDK 对齐。

## Install

```toml
[dependencies]
talon-sandbox = "0.1"
tokio = { version = "1", features = ["full"] }
```

## Quick start

```rust
use talon_sandbox::{Sandbox, CreateOpts, Resources};

#[tokio::main]
async fn main() -> talon_sandbox::Result<()> {
    // server / api_key 默认从 TALON_SANDBOX_SERVER / TALON_SANDBOX_API_KEY 读取。
    let sb = Sandbox::create(CreateOpts {
        image:     Some("talon-alpine".into()),
        resources: Resources { cpu: 2.0, memory: Some("4GiB".into()), ..Default::default() },
        network:   Some("allowlist".into()),
        timeout:   Some("30m".into()),
        ttl:       Some("6h".into()),
        ..Default::default()
    })
    .await?;

    // 同步执行命令
    let result = sb.run("npm install").await?;
    println!("exit={} out={}", result.exit_code, result.combined);

    // 异步起一个长进程
    let proc = sb.spawn("npm run dev").await?;

    // 交互式终端
    let mut pty = sb.terminal().open().await?;
    pty.write(b"ls /\n").await?;
    while let Some(chunk) = pty.recv().await? {
        print!("{}", String::from_utf8_lossy(&chunk));
    }

    let _ = proc.kill().await;
    sb.kill().await?;
    Ok(())
}
```

## 配置

显式构造 client:

```rust
use talon_sandbox::{Client, Sandbox, CreateOpts};

let client = Client::builder()
    .server("https://api.example.com")
    .api_key("ask_...")
    .build();

let sb = Sandbox::create_with(client, CreateOpts::default()).await?;
```

或设置进程级全局默认:

```rust
talon_sandbox::configure(
    talon_sandbox::Config::new("https://api.example.com").api_key("ask_..."),
);
let sb = Sandbox::create(CreateOpts::default()).await?; // 用全局默认
```

环境变量:`TALON_SANDBOX_SERVER`、`TALON_SANDBOX_API_KEY`。

## 能力

| 能力 | API |
|---|---|
| 创建 / 获取 / 列表 | `Sandbox::create` / `get` / `list` |
| 生命周期 | `pause` / `resume` / `kill` / `refresh` |
| 执行命令 | `run`(同步)/ `spawn`(异步)|
| 文件系统 | `sb.fs().read / write / list / remove` |
| 环境变量 | `sb.env().get / set` |
| 端口暴露 | `sb.expose(port, opts)` → preview URL |
| 交互终端 | `sb.terminal().open()` → `PtySession` |

## 示例

`examples/` 下:

- [`hello`](examples/hello.rs) — 创建 → 跑命令 → 清理
- [`pty_loop`](examples/pty_loop.rs) — PTY 交互
- [`vibe_coding`](examples/vibe_coding.rs) — 完整工作流:写代码 → 装依赖 → 起 dev server → 暴露端口

```sh
TALON_SANDBOX_SERVER=... TALON_SANDBOX_API_KEY=... cargo run --example vibe_coding
```

## 错误处理

所有方法返回 `Result<T, talon_sandbox::Error>`。`Error` 按 HTTP 语义分变体:

```rust
match sb.run("...").await {
    Ok(r)  => { /* ... */ }
    Err(talon_sandbox::Error::NotFound { .. })  => { /* sandbox 不存在 */ }
    Err(talon_sandbox::Error::Quota { .. })     => { /* 配额超限 */ }
    Err(talon_sandbox::Error::RateLimit { retry_after, .. }) => { /* 限流 */ }
    Err(e) => eprintln!("{e}"),
}
```

## License

Proprietary — see [LICENSE](LICENSE).
