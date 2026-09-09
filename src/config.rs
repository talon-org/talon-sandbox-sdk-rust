//! 全局配置,对齐其它语言 SDK(Go Configure / TS configure)。
//!
//! 通过 [`configure`] 设置一个进程级默认 [`Client`],之后 `Sandbox::create` 等
//! 工厂方法不传 client 时即用它。显式传入的 client 始终优先于全局默认。

use std::sync::RwLock;

use crate::client::Client;

/// 全局配置参数。
pub struct Config {
    server: String,
    api_key: Option<String>,
}

impl Config {
    /// 用指定 server 创建配置。
    pub fn new(server: impl Into<String>) -> Self {
        Config {
            server: server.into(),
            api_key: None,
        }
    }

    /// 设置 API key。
    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    fn into_client(self) -> Client {
        let mut b = Client::builder().server(self.server);
        if let Some(k) = self.api_key {
            b = b.api_key(k);
        }
        b.build()
    }
}

static DEFAULT_CLIENT: RwLock<Option<Client>> = RwLock::new(None);

/// 设置进程级默认 client。后续不显式传 client 的工厂方法都会用它。
pub fn configure(config: Config) {
    let client = config.into_client();
    *DEFAULT_CLIENT.write().expect("config lock poisoned") = Some(client);
}

/// 取得默认 client:已 [`configure`] 则返回其克隆,否则用 env / 默认值惰性构造。
/// crate 内部工厂方法在 `client: None` 时调用。
pub(crate) fn default_client() -> Client {
    if let Some(c) = DEFAULT_CLIENT
        .read()
        .expect("config lock poisoned")
        .as_ref()
    {
        return c.clone();
    }
    // 未配置:用 env(TALON_SANDBOX_SERVER / _API_KEY)或内置默认构造。
    Client::builder().build()
}
