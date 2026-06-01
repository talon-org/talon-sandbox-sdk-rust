//! 可用镜像列表,对齐后端 GET /v1/images。
//!
//! [`list_images`] 是顶层函数,对应 Go SDK 中 `Images()` 的调用位置。
//! 调用方通过返回的 [`ImageInfo`] 列表在创建 sandbox 时填选镜像 ID。

use serde::Deserialize;

use crate::client::Client;
use crate::config::default_client;
use crate::errors::Result;

/// 镜像信息,对齐后端 dto.go `ImageDTO`。
///
/// `is_default` 为 `true` 的条目是当前系统默认镜像;
/// 创建 sandbox 时若不指定 image,服务端会使用它。
#[derive(Debug, Clone, Deserialize)]
pub struct ImageInfo {
    /// 镜像唯一 ID(如 `"img_abc123"`)。创建 sandbox 时传入此值。
    pub id: String,
    /// 镜像名称(如 `"node:20-bookworm"`)。
    pub name: String,
    /// 镜像 OCI URL(如 `"docker.io/library/node:20-bookworm"`)。
    pub url: String,
    /// SHA256 校验和。
    pub sha256: String,
    /// 操作系统(通常为 `"linux"`)。
    pub os: String,
    /// CPU 架构(通常为 `"amd64"`)。
    pub arch: String,
    /// 来源:`"builtin"` 内置 | `"admin"` 管理员上传。
    pub source: String,
    /// 是否为系统默认镜像。
    pub is_default: bool,
    /// 镜像描述(可选)。
    #[serde(default)]
    pub description: String,
    /// 创建时间(Unix 秒)。
    pub created_at: i64,
}

/// `GET /v1/images` 的响应体,对齐 dto.go `ImageListResponse`。
#[derive(Debug, Deserialize)]
struct ImageListResponse {
    images: Vec<ImageInfo>,
}

/// 列出所有可用镜像,使用全局默认 client。
///
/// 对应端点:`GET /v1/images`。
/// 返回结果可作为 [`CreateOpts::image`] 的可选来源。
///
/// # 示例
///
/// ```no_run
/// use talon_sandbox::list_images;
///
/// # async fn demo() -> talon_sandbox::Result<()> {
/// let images = list_images().await?;
/// for img in &images {
///     println!("{} ({}){}", img.name, img.id, if img.is_default { " [default]" } else { "" });
/// }
/// # Ok(())
/// # }
/// ```
pub async fn list_images() -> Result<Vec<ImageInfo>> {
    list_images_with(default_client()).await
}

/// 列出所有可用镜像,使用显式 client(多租户 / 多 server 场景)。
pub async fn list_images_with(client: Client) -> Result<Vec<ImageInfo>> {
    let resp: ImageListResponse = client.get("/v1/images").await?;
    Ok(resp.images)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证 ImageInfo 反序列化字段对齐 dto.go ImageDTO。
    #[test]
    fn image_info_deserialize() {
        let json = r#"{
            "id": "img_abc123",
            "name": "node:20-bookworm",
            "url": "docker.io/library/node:20-bookworm",
            "sha256": "deadbeef",
            "os": "linux",
            "arch": "amd64",
            "source": "builtin",
            "is_default": true,
            "description": "Node.js 20 on Debian Bookworm",
            "created_at": 1700000000
        }"#;
        let img: ImageInfo = serde_json::from_str(json).unwrap();
        assert_eq!(img.id, "img_abc123");
        assert_eq!(img.name, "node:20-bookworm");
        assert!(img.is_default);
        assert_eq!(img.created_at, 1700000000);
    }

    /// description 可选,默认空字符串。
    #[test]
    fn image_info_no_description() {
        let json = r#"{
            "id": "img_xyz",
            "name": "debian:bookworm",
            "url": "docker.io/library/debian:bookworm",
            "sha256": "cafe1234",
            "os": "linux",
            "arch": "amd64",
            "source": "builtin",
            "is_default": false,
            "created_at": 1700000001
        }"#;
        let img: ImageInfo = serde_json::from_str(json).unwrap();
        assert_eq!(img.description, "");
    }

    /// 列表响应反序列化验证。
    #[test]
    fn image_list_response_deserialize() {
        let json = r#"{"images": [
            {"id":"img_1","name":"n1","url":"u1","sha256":"s1","os":"linux","arch":"amd64","source":"builtin","is_default":true,"created_at":1},
            {"id":"img_2","name":"n2","url":"u2","sha256":"s2","os":"linux","arch":"amd64","source":"admin","is_default":false,"created_at":2}
        ]}"#;
        let resp: ImageListResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.images.len(), 2);
        assert_eq!(resp.images[0].id, "img_1");
        assert_eq!(resp.images[1].source, "admin");
    }
}
