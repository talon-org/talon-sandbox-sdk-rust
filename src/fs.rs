//! 沙箱文件系统操作,对齐 Go SDK fs/fs.go。
//!
//! [`Fs`] 提供在 sandbox 内读写文件、列目录、删除的接口。
//! 由 `Sandbox::fs()` 构造,不建议直接实例化。

use crate::client::Client;
use crate::errors::Result;
use crate::types::{FsEntry, FsListDto};

/// 沙箱文件系统句柄。
///
/// 所有操作都路由到指定 sandbox 的文件系统端点。
/// 可安全 Clone(内部 `Client` 共享连接池)。
#[derive(Clone)]
pub struct Fs {
    sandbox_id: String,
    client: Client,
}

impl Fs {
    /// 构造文件系统句柄。由 `Sandbox::fs()` 调用,通常不需要直接使用。
    pub(crate) fn new(sandbox_id: String, client: Client) -> Fs {
        Fs { sandbox_id, client }
    }

    /// 读取文件内容,返回原始字节。
    ///
    /// 对应端点:`GET /v1/sandboxes/{id}/fs/{path}`。
    pub async fn read(&self, path: &str) -> Result<Vec<u8>> {
        let url_path = format!("/v1/sandboxes/{}/fs/{}", self.sandbox_id, clean_path(path));
        self.client.get_bytes(&url_path).await
    }

    /// 读取文件内容,以 UTF-8 字符串返回。
    ///
    /// 底层调用 [`Fs::read`];若文件内容含非 UTF-8 字节则返回错误。
    /// 对齐 Go 侧直接做 `string(bytes)` 的语义(Go 不会报 UTF-8 错误,
    /// 但 Rust 严格校验;如需宽松模式请用 `read` + `String::from_utf8_lossy`)。
    pub async fn read_text(&self, path: &str) -> Result<String> {
        let bytes = self.read(path).await?;
        String::from_utf8(bytes).map_err(|e| {
            crate::errors::Error::Parse(format!("文件内容不是合法 UTF-8: {e}"))
        })
    }

    /// 将字节写入文件,自动创建父目录。
    ///
    /// 对应端点:`PUT /v1/sandboxes/{id}/fs/{path}`(Content-Type: application/octet-stream)。
    pub async fn write(&self, path: &str, data: Vec<u8>) -> Result<()> {
        let url_path = format!("/v1/sandboxes/{}/fs/{}", self.sandbox_id, clean_path(path));
        self.client.put_bytes(&url_path, data).await
    }

    /// 将文本写入文件(UTF-8 编码)。
    ///
    /// 底层调用 [`Fs::write`],将 `&str` 编码为 UTF-8 字节后发送。
    pub async fn write_text(&self, path: &str, text: &str) -> Result<()> {
        self.write(path, text.as_bytes().to_vec()).await
    }

    /// 列出目录条目。
    ///
    /// 对应端点:`GET /v1/sandboxes/{id}/fs-list/{path}`。
    /// 注意端点是 `fs-list` 而非 `fs`。
    pub async fn list(&self, path: &str) -> Result<Vec<FsEntry>> {
        let url_path = format!("/v1/sandboxes/{}/fs-list/{}", self.sandbox_id, clean_path(path));
        let dto: FsListDto = self.client.get(&url_path).await?;
        Ok(dto.entries)
    }

    /// 删除文件或目录。
    ///
    /// 对应端点:`DELETE /v1/sandboxes/{id}/fs/{path}`。
    pub async fn remove(&self, path: &str) -> Result<()> {
        let url_path = format!("/v1/sandboxes/{}/fs/{}", self.sandbox_id, clean_path(path));
        self.client.delete(&url_path).await
    }
}

/// 规范化文件路径:去掉开头的 `/`,并对每个路径段做 percent-encoding。
///
/// 对齐 Go `cleanPath`:
/// ```go
/// p = strings.TrimPrefix(p, "/")
/// parts := strings.Split(p, "/")
/// for i, part := range parts { parts[i] = url.PathEscape(part) }
/// return strings.Join(parts, "/")
/// ```
/// `percent_encode` 对应 Go `url.PathEscape`,
/// 只编码在路径段里非法的字符(保留 `~`, `-`, `.`, `_` 等)。
fn clean_path(p: &str) -> String {
    let p = p.trim_start_matches('/');
    p.split('/')
        .map(percent_encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// 对单个路径段做 percent-encoding,对齐 Go `url.PathEscape`。
///
/// 编码策略:只有字母、数字及 `-._~` 不需要转义,其余全部 `%XX` 编码。
/// 这与 RFC 3986 unreserved characters 一致,和 Go `url.PathEscape` 行为相同。
fn percent_encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            // unreserved characters: ALPHA / DIGIT / "-" / "." / "_" / "~"
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            b => {
                // 格式化为 %XX
                out.push('%');
                out.push(char::from_digit((b >> 4) as u32, 16).unwrap().to_ascii_uppercase());
                out.push(char::from_digit((b & 0xf) as u32, 16).unwrap().to_ascii_uppercase());
            }
        }
    }
    out
}
