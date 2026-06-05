//! PTY WebSocket 模块,对齐 Go SDK terminal/terminal.go。
//!
//! [`Terminal`] 是轻量句柄,通过 [`Terminal::open`] 建立 WebSocket 连接并返回
//! [`PtySession`]。`PtySession` 提供 `write` / `resize` / `recv` / `close` 四个
//! 异步方法。recv 采用 Rust-idiomatic 的拉取式(`async fn recv`)而非 Go 的
//! 注册回调(`OnData`),让调用方可以自由选择 loop / select / stream 组合。
//!
//! ## 并发读写设计
//!
//! tungstenite 的 WS 流(`WebSocketStream`)实现 `Stream + Sink`,但同时持有
//! `&mut self` 引用不能既 poll sink 又 poll stream。`split()` 把它拆成独立的
//! `(SplitSink, SplitStream)`,两半各自只需 `&mut self`,可放进不同字段,使
//! `write`/`resize` 和 `recv` 能在 `&mut PtySession` 上自由交替调用,无需额外
//! `Mutex`。这是最简洁可用的形态。

use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::HeaderValue,
        Message,
    },
    MaybeTlsStream, WebSocketStream,
};

use bytes::Bytes;

use crate::{
    client::Client,
    errors::{Error, Result},
};

// ─── 内部类型别名 ──────────────────────────────────────────────────────────────

/// WS 流的完整类型,便于后续引用。
type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

// ─── Terminal ─────────────────────────────────────────────────────────────────

/// PTY 句柄,对应单个 sandbox 的终端入口。
///
/// 通过 `Sandbox::terminal()` 构造,不持有 WS 连接本身。
/// 调用 [`Terminal::open`] 才实际发起握手并返回 [`PtySession`]。
pub struct Terminal {
    sandbox_id: String,
    client: Client,
}

impl Terminal {
    /// 由 `Sandbox::terminal()` 调用。
    pub(crate) fn new(sandbox_id: String, client: Client) -> Self {
        Self { sandbox_id, client }
    }

    /// 建立 PTY WebSocket 连接,返回可读写的 [`PtySession`]。
    ///
    /// 握手请求会在 HTTP Upgrade 阶段附带 `Authorization: Bearer <key>` 头,
    /// 服务端中间件在 upgrade 前校验。
    pub async fn open(&self) -> Result<PtySession> {
        self.open_with(None).await
    }

    /// 建立连接并发送初始 resize 消息(可选)。
    ///
    /// `rows`/`cols` 均为 `None` 时等同于 [`Terminal::open`]。
    pub async fn open_with(
        &self,
        initial_size: Option<(u16, u16)>,
    ) -> Result<PtySession> {
        // 1. 拼 WebSocket URL,e.g. "wss://api.sandbox.talon.net.cn/v1/sandboxes/sb_xxx/pty"
        let url = self
            .client
            .ws_url(&format!("/v1/sandboxes/{}/pty", self.sandbox_id));

        // 2. 构造握手 Request,借助 tungstenite 的 IntoClientRequest 补全
        //    Host / Upgrade / Sec-WebSocket-Key 等标准头,再手动加 Authorization。
        let mut request = url
            .into_client_request()
            .map_err(|e| Error::Network(Box::new(e)))?;

        // 规范 User-Agent,与 HTTP 路径一致,供后端来源追踪。
        request.headers_mut().insert(
            "User-Agent",
            HeaderValue::from_static(self.client.user_agent()),
        );

        if let Some(auth) = self.client.auth_header() {
            let value = HeaderValue::from_str(&auth)
                .map_err(|e| Error::Network(Box::new(e)))?;
            request.headers_mut().insert("Authorization", value);
        }

        // 3. 发起 WS 握手
        let (ws_stream, _resp) = connect_async(request)
            .await
            .map_err(|e| Error::Network(Box::new(e)))?;

        // 4. split 成独立的 sink / stream,使写和读可以独立 &mut 借用
        let (sink, stream) = ws_stream.split();
        let mut sess = PtySession {
            sink,
            stream,
            closed: false,
        };

        // 5. 可选:发送初始 resize
        if let Some((rows, cols)) = initial_size {
            sess.resize(rows, cols).await?;
        }

        Ok(sess)
    }
}

// ─── PtySession ───────────────────────────────────────────────────────────────

/// 活跃的 PTY 双工会话。
///
/// `write` / `resize` 走 `SplitSink`,`recv` 走 `SplitStream`。
/// 由于两半持有独立的 `&mut self` 域,调用方无需额外锁即可交替使用。
///
/// ## 并发注意
///
/// `PtySession` 本身不是 `Sync`,要在两个 task 里并发读写,需要先将其拆开
/// 或套 `Mutex`。最常见的用法是单 task 内交替 `recv` / `write`:
///
/// ```ignore
/// loop {
///     tokio::select! {
///         chunk = pty.recv() => { /* 处理输出 */ }
///         _ = some_input_future => { pty.write(b"data").await?; }
///     }
/// }
/// ```
pub struct PtySession {
    /// 发送半(stdin 帧 + 控制帧)。
    sink: SplitSink<WsStream, Message>,
    /// 接收半(stdout/stderr 帧)。
    stream: SplitStream<WsStream>,
    /// 会话是否已关闭(本地标记,避免重复 close)。
    closed: bool,
}

impl PtySession {
    // ── 发送 ──────────────────────────────────────────────────────────────────

    /// 向 PTY stdin 发送字节数据(二进制帧)。
    ///
    /// 会话已关闭时返回 [`Error::PtyClosed`]。
    pub async fn write(&mut self, data: &[u8]) -> Result<()> {
        if self.closed {
            return Err(Error::PtyClosed);
        }
        self.sink
            .send(Message::Binary(data.to_vec()))
            .await
            .map_err(|e| Error::Network(Box::new(e)))
    }

    /// 发送终端大小调整控制帧(文本 JSON 帧)。
    ///
    /// 格式:`{"type":"resize","rows":<rows>,"cols":<cols>}`
    pub async fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        if self.closed {
            return Err(Error::PtyClosed);
        }
        // 手动拼 JSON,避免为一个结构体引入额外 serde 派生
        let json = format!(
            r#"{{"type":"resize","rows":{rows},"cols":{cols}}}"#
        );
        self.sink
            .send(Message::Text(json))
            .await
            .map_err(|e| Error::Network(Box::new(e)))
    }

    // ── 接收 ──────────────────────────────────────────────────────────────────

    /// 从 PTY 接收下一个 stdout/stderr 数据块。
    ///
    /// - 返回 `Ok(Some(bytes))`:收到一帧二进制数据。
    /// - 返回 `Ok(None)`:对端正常关闭连接。
    /// - 返回 `Err(...)`:连接出错。
    ///
    /// 文本帧(服务端目前只发 resize,理论上不会有)会被跳过;Ping/Pong
    /// 由 tungstenite 自动处理。调用方只需 loop recv 即可。
    pub async fn recv(&mut self) -> Result<Option<Bytes>> {
        loop {
            match self.stream.next().await {
                // 流正常结束(对端发了 Close 帧或底层连接断开)
                None => return Ok(None),

                Some(Ok(msg)) => match msg {
                    // 二进制帧 = PTY stdout/stderr,直接返回
                    Message::Binary(data) => {
                        return Ok(Some(Bytes::from(data)));
                    }
                    // Close 帧:对端主动关闭
                    Message::Close(_) => {
                        self.closed = true;
                        return Ok(None);
                    }
                    // 文本帧(服务端当前不主动推)、Ping/Pong:跳过继续等
                    _ => continue,
                },

                Some(Err(e)) => {
                    return Err(Error::Network(Box::new(e)));
                }
            }
        }
    }

    // ── 关闭 ──────────────────────────────────────────────────────────────────

    /// 优雅关闭 PTY 会话:发送 WS Close 帧并标记会话已关闭。
    ///
    /// 幂等——重复调用不会返回错误。
    pub async fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        // 发送 Close 帧;服务端收到后会回 Close 并关闭连接
        self.sink
            .send(Message::Close(None))
            .await
            .map_err(|e| Error::Network(Box::new(e)))
    }
}
