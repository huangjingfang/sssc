//! SSSC Rust 后端：多集群 Profile 管理 + S3 全生命周期命令。
//!
//! 架构（与 docs/01 §3、docs/02 §4 对应）：
//! - Profile 元数据存 app_config_dir/profiles.json；
//!   密钥存 app_config_dir/secrets.json（0600），重启不丢。
//!   TODO(v0.2): 迁移系统凭据库（keyring：Keychain / Credential Manager / libsecret）。
//! - 每个 Profile 一个 S3 Client（自定义 endpoint / path-style / region），缓存复用。
//! - [`ops`] 模块承载全部 S3 业务逻辑，不依赖 tauri ——
//!   Tauri command 与集成测试（tests/integration.rs）复用同一实现。
//! - 错误统一映射 S3Error{code,message,requestId}，前端呈现 Code/Message/RequestId 三元组。
//! - TODO(v0.2): 传输引擎（MPU 分块 + Range 断点续传 + SQLite 状态库 +
//!   SlowDown 自动降并发）；当前 upload/download 为单请求流式实现。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use aws_sdk_s3::config::{Credentials, Region, SharedCredentialsProvider};
use aws_sdk_s3::Client;
use serde::{Deserialize, Serialize};
pub mod sigv2;
pub mod tls_insecure;
use tauri::{AppHandle, Emitter, Manager, State};

/* ---------------- 领域模型（与 src/types.ts 一一对应） ---------------- */

/// 寻址风格：path（endpoint/bucket/key，私有部署常用）| virtual（bucket.endpoint）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    Path,
    Virtual,
}

impl Default for Style {
    fn default() -> Self {
        Style::Virtual
    }
}

/// 签名版本：v4（默认）| v2（旧网关/nginx 反代 RGW 兼容）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Signature {
    V4,
    V2,
}

impl Default for Signature {
    fn default() -> Self {
        Signature::V4
    }
}

/// 旧数据（字符串 "path"/"v4" 等）兼容；未知值回退默认（v4/virtual）。
fn deserialize_with_fallback<'de, D, T, F>(deserializer: D, fallback: F) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
    F: Fn() -> T,
{
    // 先按字符串读入（兼容旧版 profiles.json），再尝试解析为枚举；失败回退默认值。
    let raw = String::deserialize(deserializer)?;
    let value = serde_json::to_value(raw).map_err(serde::de::Error::custom)?;
    T::deserialize(value).or_else(|_| Ok(fallback()))
}

fn deserialize_style<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Style, D::Error> {
    deserialize_with_fallback(d, Style::default)
}

fn deserialize_signature<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Signature, D::Error> {
    deserialize_with_fallback(d, Signature::default)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    pub region: String,
    pub access_key: String,
    pub secret_masked: String,
    /// 寻址风格："path" | "virtual"（旧数据为字符串，未知值回退 virtual）
    #[serde(default, deserialize_with = "deserialize_style")]
    pub style: Style,
    pub insecure_tls: bool,
    pub color: String,
    pub enabled: bool,
    /// 签名版本：v4（默认）| v2（旧数据为字符串，未知值回退 v4）
    #[serde(default, deserialize_with = "deserialize_signature")]
    pub signature: Signature,
    /// Secret Key 明文仅在保存/测试请求中一次性携带，接口永不回传。
    #[serde(default, skip_serializing)]
    pub secret_key: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub ok: bool,
    pub latency_ms: Option<u128>,
    pub bucket_count: Option<usize>,
    pub error: Option<S3Error>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Bucket {
    pub name: String,
    pub created_at: Option<String>,
    pub region: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct S3Object {
    pub key: String,
    pub size: Option<i64>,
    pub last_modified: Option<String>,
    pub etag: Option<String>,
    pub storage_class: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ListResult {
    pub objects: Vec<S3Object>,
    pub prefixes: Vec<String>,
    pub next_token: Option<String>,
    pub key_count: usize,
}

/// delete_folder 的结果：递归删除的对象数与失败数
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFolderResult {
    pub deleted_count: usize,
    pub error_count: usize,
}

/// 批量删除中单个 key 的失败详情
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFailure {
    pub key: String,
    pub code: String,
    pub message: String,
}

/// 批量删除结果：成功数、失败数与失败详情
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeleteBatchResult {
    pub deleted_count: usize,
    pub error_count: usize,
    pub failures: Vec<DeleteFailure>,
}

/// 批量删除进度事件（delete://progress）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeleteProgress {
    pub batch_id: String,
    pub done: usize,
    pub total: usize,
    /// 阶段：listing=展开文件夹中的对象，deleting=批量删除中
    pub phase: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct S3Error {
    pub code: String,
    pub message: String,
    pub request_id: Option<String>,
    pub http_status: Option<i32>,
}

impl S3Error {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            request_id: None,
            http_status: None,
        }
    }
}

/* ---------------- 可复用 S3 操作（无 tauri 依赖，测试与命令共用） ---------------- */

pub mod ops {
    use super::*;

    /// 上传分块大小（256KB）：分块读文件组装流式 body。
    pub const UPLOAD_BUF_SIZE: usize = 256 * 1024;
    /// 下载进度事件发射阈值（512KB）：避免每个 chunk 都发事件刷屏。
    pub const DOWNLOAD_EMIT_THRESHOLD: u64 = 512 * 1024;
    /// 预签名最大有效期（7 天，S3 SigV4 上限）。
    pub const MAX_PRESIGN_EXPIRES: u64 = 604_800;
    /// 单请求（非 MPU）上传上限：5GB，S3 协议硬限制。
    pub const MAX_SINGLE_UPLOAD: u64 = 5 * 1024 * 1024 * 1024;

    /// 构建集群 Client。自定义 endpoint + path-style + region 是私有部署核心能力（docs/02 §1.5）。
    /// region 留空回退 us-east-1（兼容部署默认值）。
    pub async fn build_client(profile: &Profile, secret: &str) -> Client {
        let creds = Credentials::new(
            profile.access_key.clone(),
            secret.to_string(),
            None,
            None,
            "sssc-static",
        );
        let region = if profile.region.trim().is_empty() {
            "us-east-1".to_string()
        } else {
            profile.region.trim().to_string()
        };
        let mut cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(Region::new(region))
            .credentials_provider(SharedCredentialsProvider::new(creds));
        if !profile.endpoint.is_empty() {
            cfg = cfg.endpoint_url(profile.endpoint.trim_end_matches('/'));
        }
        let sdk_config = cfg.load().await;
        let mut builder = aws_sdk_s3::config::Builder::from(&sdk_config);
        if profile.style == Style::Path {
            builder = builder.force_path_style(true);
        }
        if profile.insecure_tls {
            // 自签证书 / 内网 HTTPS：跳过证书校验（用户显式勾选）
            builder = builder.http_client(crate::tls_insecure::insecure_http_client());
        }
        if profile.signature == Signature::V2 {
            // SigV2 兼容模式：HTTP 层重签（见 sigv2.rs 模块注释）。
            // 关闭默认 flexible checksum：否则 SDK 上传会用 V4 流式分块签名编码
            // body（chunk-signature 帧），V2 网关无法解码导致内容损坏。
            // 注：SigV2 与 insecure_tls 同时启用时，V2 重签连接器在外层、
            // 其内部 default_connector 不带 insecure 语义，此处以 SigV2 优先。
            builder = builder
                .request_checksum_calculation(
                    aws_sdk_s3::config::RequestChecksumCalculation::WhenRequired,
                )
                .response_checksum_validation(
                    aws_sdk_s3::config::ResponseChecksumValidation::WhenRequired,
                )
                .http_client(crate::sigv2::v2_http_client(
                    profile.access_key.clone(),
                    secret.to_string(),
                ));
        }
        Client::from_conf(builder.build())
    }

    /// 连接测试：ListBuckets 探活 + 延迟 + 桶计数。
    /// SignatureDoesNotMatch 时附加排障提示（AK 有效但签名比对不过的常见成因）。
    pub async fn test_connection(profile: &Profile) -> ConnectionStatus {
        let secret = profile.secret_key.clone().unwrap_or_default();
        let client = build_client(profile, &secret).await;
        let start = std::time::Instant::now();
        match client.list_buckets().send().await {
            Ok(out) => ConnectionStatus {
                ok: true,
                latency_ms: Some(start.elapsed().as_millis()),
                bucket_count: Some(out.buckets().len()),
                error: None,
            },
            Err(e) => {
                let mut err = map_s3_err(e);
                // 基于解析出的服务端错误码判断（不再做 message 字符串匹配）
                if err.code == "SignatureDoesNotMatch" {
                    err.message = "签名不匹配。该错误说明 AccessKey 存在且有效，但签名比对失败。常见成因：\
                    ① Secret Key 有误（最常见）；\
                    ② 反向代理（nginx）改写 Host 头——请改用 RGW 直连端口（如 Ceph 默认 7480）重试；\
                    ③ region 与 RGW 配置不符（默认 us-east-1）。"
                        .into();
                }
                ConnectionStatus {
                    ok: false,
                    latency_ms: Some(start.elapsed().as_millis()),
                    bucket_count: None,
                    error: Some(err),
                }
            }
        }
    }

    pub async fn list_buckets(client: &Client) -> Result<Vec<Bucket>, S3Error> {
        let out = client.list_buckets().send().await.map_err(map_s3_err)?;
        Ok(out
            .buckets()
            .iter()
            .map(|b| Bucket {
                name: b.name().unwrap_or_default().to_string(),
                created_at: b.creation_date().map(|d| d.to_string()),
                region: None,
            })
            .collect())
    }

    /// ListObjectsV2 + delimiter=/" 目录"聚合 + encoding-type=url（中文 key 硬坑）。
    pub async fn list_objects(
        client: &Client,
        bucket: &str,
        prefix: &str,
        token: Option<&str>,
    ) -> Result<ListResult, S3Error> {
        let mut req = client
            .list_objects_v2()
            .bucket(bucket)
            .delimiter("/")
            .encoding_type(aws_sdk_s3::types::EncodingType::Url)
            .prefix(prefix);
        if let Some(t) = token {
            req = req.continuation_token(t);
        }
        let out = req.send().await.map_err(map_s3_err)?;
        // encoding-type=url 下服务端返回 URL 编码的 key，SDK 不自动解码，需手动还原
        // （否则中文等非 ASCII 文件名显示为 %XX 编码，docs/02 §1.2）
        let decode = |s: &str| -> String {
            percent_encoding::percent_decode_str(s)
                .decode_utf8()
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| s.to_string())
        };
        Ok(ListResult {
            // 过滤掉目录 marker（key 以 / 结尾的 0 字节占位对象），
            // 它们已经以 CommonPrefix 目录形式展示，不应再作为"文件"出现。
            objects: out
                .contents()
                .iter()
                .filter(|o| !o.key().unwrap_or_default().ends_with('/'))
                .map(|o| S3Object {
                    key: decode(o.key().unwrap_or_default()),
                    size: o.size(),
                    last_modified: o.last_modified().map(|d| d.to_string()), // DateTime Display = RFC3339
                    etag: o.e_tag().map(|s| s.to_string()),
                    storage_class: o.storage_class().map(|s| s.as_str().to_string()),
                })
                .collect(),
            prefixes: out
                .common_prefixes()
                .iter()
                .map(|p| decode(p.prefix().unwrap_or_default()))
                .collect(),
            next_token: out.next_continuation_token().map(|s| s.to_string()),
            key_count: out.key_count().unwrap_or(0) as usize,
        })
    }

    pub async fn create_bucket(client: &Client, bucket: &str) -> Result<(), S3Error> {
        client
            .create_bucket()
            .bucket(bucket)
            .send()
            .await
            .map_err(map_s3_err)?;
        Ok(())
    }

    /// 仅空桶可删；非空返回 BucketNotEmpty（S3Error 原样透出）。
    pub async fn delete_bucket(client: &Client, bucket: &str) -> Result<(), S3Error> {
        client
            .delete_bucket()
            .bucket(bucket)
            .send()
            .await
            .map_err(map_s3_err)?;
        Ok(())
    }

    pub async fn delete_object(client: &Client, bucket: &str, key: &str) -> Result<(), S3Error> {
        client
            .delete_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(map_s3_err)?;
        Ok(())
    }

    /// 已确认存在"读后删"怪癖的桶（进程内缓存；首次 500 后标记，后续删除直接走快路径）
    fn quirky_buckets() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
        static QUIRKY: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
            std::sync::OnceLock::new();
        QUIRKY.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
    }

    /// 该桶是否已确认存在"读后删"怪癖（供批量删除跳过注定失败的 DeleteObjects）
    pub fn bucket_has_delete_quirk(bucket: &str) -> bool {
        quirky_buckets().lock().unwrap().contains(bucket)
    }

    /// 覆盖重写一个空对象：产生未被 GET 读取的新版本，绕开"读后删"500 判定
    async fn overwrite_empty(client: &Client, bucket: &str, key: &str) {
        let _ = client
            .put_object()
            .bucket(bucket)
            .key(key)
            .body(aws_sdk_s3::primitives::ByteStream::from_static(b""))
            .send()
            .await;
    }

    /// 短间隔重试删除（快路径：300ms / 600ms）
    async fn delete_with_short_retry(client: &Client, bucket: &str, key: &str, attempts: u32) -> Result<(), S3Error> {
        let mut last: Option<S3Error> = None;
        for i in 0..attempts {
            if i > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(300u64 * (1 << (i - 1)))).await;
            }
            match delete_object(client, bucket, key).await {
                Ok(()) => return Ok(()),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| S3Error::new("InternalError", "删除失败（重试耗尽）")))
    }

    /// 带服务端怪癖规避的删除：
    /// SeaweedFS 4.50 等网关存在"读后删"怪癖——对象被 GET 读取后 DeleteObject 稳定
    /// 返回 500 InternalError（tests/diag.rs diag_read_then_delete 复现）。
    /// 规避（tests/integration.rs TC8/TC13 验证）：
    ///   · 桶已确认怪癖 → 直接覆盖重写后删除（快路径，不浪费时间在注定失败的首次尝试上）；
    ///   · 首次遇到 500 → 立即覆盖重写再删（不再做无效的长退避），并标记该桶；
    ///   · 剩余重试短间隔兜底。
    pub async fn delete_object_resilient(client: &Client, bucket: &str, key: &str) -> Result<(), S3Error> {
        let is_quirk = |e: &S3Error| e.code == "InternalError" || e.http_status == Some(500);

        // 快路径：该桶已确认怪癖，直接覆盖重写 → 删
        if quirky_buckets().lock().unwrap().contains(bucket) {
            overwrite_empty(client, bucket, key).await;
            return delete_with_short_retry(client, bucket, key, 3).await;
        }

        // 常规路径：先正常删一次
        match delete_object(client, bucket, key).await {
            Ok(()) => return Ok(()),
            Err(e) if !is_quirk(&e) => return Err(e), // NoSuchKey/AccessDenied 等直接返回
            Err(_) => {
                // 首次命中怪癖：立即覆盖重写再删（不做无效长退避），并标记桶
                quirky_buckets().lock().unwrap().insert(bucket.to_string());
                overwrite_empty(client, bucket, key).await;
                delete_with_short_retry(client, bucket, key, 3).await
            }
        }
    }

    /// 批量删除（S3 DeleteObjects，单请求最多 1000 个 key）。
    /// 返回 (成功数, 失败详情列表)；单个 key 失败不影响同批其他 key。
    async fn delete_objects_chunk(
        client: &Client,
        bucket: &str,
        keys: &[String],
    ) -> Result<(usize, Vec<DeleteFailure>), S3Error> {
        if keys.is_empty() {
            return Ok((0, Vec::new()));
        }
        let objs = keys
            .iter()
            .cloned()
            .map(|k| aws_sdk_s3::types::ObjectIdentifier::builder().key(k).build())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| S3Error { code: "InvalidRequest".into(), message: e.to_string(), request_id: None, http_status: None })?;
        let delete = aws_sdk_s3::types::Delete::builder()
            .set_objects(Some(objs))
            .build()
            .map_err(|e| S3Error { code: "InvalidRequest".into(), message: e.to_string(), request_id: None, http_status: None })?;
        let out = client
            .delete_objects()
            .bucket(bucket)
            .delete(delete)
            .send()
            .await
            .map_err(map_s3_err)?;
        let failures = out
            .errors()
            .iter()
            .map(|e| DeleteFailure {
                key: e.key().unwrap_or_default().to_string(),
                code: e.code().unwrap_or("Unknown").to_string(),
                message: e.message().unwrap_or("批量删除失败").to_string(),
            })
            .collect();
        let ok = out.deleted().len();
        Ok((ok, failures))
    }

    /// 分页列举前缀下全部 key（无 delimiter，含文件夹占位 marker 自身）。
    pub async fn list_all_keys_under(client: &Client, bucket: &str, prefix: &str) -> Result<Vec<String>, S3Error> {
        let mut token: Option<String> = None;
        let mut keys = Vec::new();
        loop {
            let mut req = client.list_objects_v2().bucket(bucket).prefix(prefix);
            if let Some(t) = &token {
                req = req.continuation_token(t);
            }
            let out = req.send().await.map_err(map_s3_err)?;
            for o in out.contents() {
                if let Some(k) = o.key() {
                    keys.push(k.to_string());
                }
            }
            token = out.next_continuation_token().map(|s| s.to_string());
            if token.is_none() {
                break;
            }
        }
        Ok(keys)
    }

    /// 统一批量删除：keys 为对象 key，prefixes 为"文件夹"（递归列举）。
    /// 先展开全部待删 key（进度 on_progress(0, total)），再按 1000 个/请求 DeleteObjects，
    /// 每批完成回调 on_progress(已处理数, 总数)。返回逐 key 失败列表。
    pub async fn delete_many(
        client: &Client,
        bucket: &str,
        keys: &[String],
        prefixes: &[String],
        on_progress: &(dyn Fn(usize, usize) + Send + Sync),
    ) -> Result<DeleteBatchResult, S3Error> {
        // 1. 展开文件夹 → 全量 key 列表（文件 key 直接并入）
        let mut all: Vec<String> = Vec::new();
        for k in keys {
            if !all.contains(k) {
                all.push(k.clone());
            }
        }
        for p in prefixes {
            let norm = if p.ends_with('/') { p.clone() } else { format!("{p}/") };
            all.extend(list_all_keys_under(client, bucket, &norm).await?);
        }
        let total = all.len();
        on_progress(0, total);

        // 2. 按 1000 个/请求批量删除；批量报错的 key 逐个 DeleteObject 重试降级
        //    （部分 S3 网关如 SeaweedFS/Ceph 旧版对 DeleteObjects 兼容不全，
        //     批量逐 key 报错但单删成功——重试能救回这些 key，并拿到真实错误码）
        let mut deleted: usize = 0;
        let mut failures: Vec<DeleteFailure> = Vec::new();
        let mut processed: usize = 0;
        for chunk in all.chunks(1000) {
            // 桶已确认怪癖 → DeleteObjects 必然逐 key 500，直接进并发覆盖重写快路径，省掉注定失败的批量请求
            let (ok, batch_failures) = if bucket_has_delete_quirk(bucket) {
                (0, chunk.iter().map(|k| DeleteFailure { key: k.clone(), code: "SkippedBulk".into(), message: String::new() }).collect())
            } else {
                delete_objects_chunk(client, bucket, chunk).await?
            };
            deleted += ok;
            processed += chunk.len();
            on_progress(processed.min(total), total);
            // 降级重试并发执行（8 路）：单删 + 怪癖规避（读后删 500 → 覆盖重写再删）
            use futures_util::stream::{self, StreamExt};
            let mut retry_stream = stream::iter(batch_failures)
                .map(|f| {
                    let bucket = bucket.to_string();
                    async move {
                        (f.key.clone(), delete_object_resilient(client, &bucket, &f.key).await, f)
                    }
                })
                .buffer_unordered(8);
            let mut done_cnt = 0usize;
            while let Some((key, res, f)) = retry_stream.next().await {
                done_cnt += 1;
                match res {
                    Ok(()) => deleted += 1,
                    Err(e) => failures.push(DeleteFailure {
                        key,
                        code: e.code,
                        message: if e.message.is_empty() { f.message } else { e.message },
                    }),
                }
                on_progress((processed - chunk.len() + done_cnt).min(total), total);
            }
            processed = processed.min(total);
            on_progress(processed, total);
        }
        let error_count = failures.len();
        Ok(DeleteBatchResult { deleted_count: deleted, error_count, failures })
    }

    /// 递归删除"文件夹"（前缀）下列出的全部对象（含文件夹占位对象 key 自身）。
    /// SeaweedFS filer 存在真实目录元数据：纯"幽灵目录"（无任何对象）DeleteObjects
    /// 删 0 个 key 等于没删，目录条目会保留。收尾统一"触碰 marker 再删除"
    /// （PUT 前缀占位对象产生实体 → 再删，filer 即清空目录条目；对 AWS 等纯前缀
    /// 语义的实现无副作用——创建后立刻删除）。
    pub async fn delete_folder(
        client: &Client,
        bucket: &str,
        prefix: &str,
    ) -> Result<DeleteFolderResult, S3Error> {
        let norm = if prefix.ends_with('/') { prefix.to_string() } else { format!("{prefix}/") };
        let mut r = delete_many(client, bucket, &[], &[norm.clone()], &|_, _| {}).await?;
        // 若仍有残留（删除后列表短暂不一致或怪癖重试中），再兜一轮
        let remaining = list_all_keys_under(client, bucket, &norm).await.unwrap_or_default();
        if !remaining.is_empty() {
            let r2 = delete_many(client, bucket, &remaining, &[], &|_, _| {}).await?;
            r.deleted_count += r2.deleted_count;
            r.error_count += r2.error_count;
            r.failures.extend(r2.failures);
        }
        // 触碰 marker 并删除：清除幽灵目录条目
        let _ = client
            .put_object()
            .bucket(bucket)
            .key(&norm)
            .body(aws_sdk_s3::primitives::ByteStream::from_static(b""))
            .send()
            .await;
        if let Err(e) = delete_object_resilient(client, bucket, &norm).await {
            eprintln!("[delete_folder] 清除目录条目 {norm} 失败: {}", e.message);
            r.error_count += 1;
            r.failures.push(DeleteFailure { key: norm, code: e.code, message: e.message });
        }
        Ok(DeleteFolderResult { deleted_count: r.deleted_count, error_count: r.error_count })
    }

    /// 创建"文件夹"：S3 无目录概念，惯例是写入一个 key 以 / 结尾的 0 字节对象。
    /// 列表时它会以 CommonPrefix（目录）形式出现。
    pub async fn create_folder(client: &Client, bucket: &str, key: &str) -> Result<(), S3Error> {
        let key = if key.ends_with('/') {
            key.to_string()
        } else {
            format!("{key}/")
        };
        client
            .put_object()
            .bucket(bucket)
            .key(key)
            .body(aws_sdk_s3::primitives::ByteStream::from_static(b""))
            .send()
            .await
            .map_err(map_s3_err)?;
        Ok(())
    }

    /// SigV4 预签名 GET URL（上限 7 天，docs/02 §3）。
    pub async fn presign_get(
        client: &Client,
        bucket: &str,
        key: &str,
        expires_sec: u64,
    ) -> Result<String, S3Error> {
        let expires = expires_sec.min(MAX_PRESIGN_EXPIRES);
        let presigned = client
            .get_object()
            .bucket(bucket)
            .key(key)
            .presigned(
                aws_sdk_s3::presigning::PresigningConfig::builder()
                    .expires_in(std::time::Duration::from_secs(expires))
                    .build()
                    .map_err(|e| S3Error::new("PresignConfig", format!("{e:?}")))?,
            )
            .await
            .map_err(map_s3_err)?;
        Ok(presigned.uri().to_string())
    }

    /// 上传本地文件（单请求流式；MPU 分块 + 断点续传见 v0.2 传输引擎）。
    /// 分块读取文件组装请求 body，每块实时回调 `on_progress(done, total)`。
    pub async fn upload_file(
        client: &Client,
        bucket: &str,
        key: &str,
        path: &std::path::Path,
        progress: std::sync::Arc<dyn Fn(u64, u64) + Send + Sync>,
    ) -> Result<(), S3Error> {
        use futures_util::TryStreamExt;
        use tokio::io::AsyncRead;

        let total = std::fs::metadata(path)
            .map_err(|e| S3Error::new("LocalFile", format!("{e}")))?
            .len();
        // 单请求上传上限 5GB（S3 协议限制；MPU 支持见 v0.2 传输引擎）
        if total > MAX_SINGLE_UPLOAD {
            return Err(S3Error::new(
                "EntityTooLarge",
                format!(
                    "单请求上传上限 5GB，当前文件 {total} 字节（{} GB）。请在后续版本的多分片上传（MPU）中传输该文件。",
                    total / (1024 * 1024 * 1024)
                ),
            ));
        }
        progress(0, total);
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|e| S3Error::new("LocalFile", format!("{e}")))?;
        // 复用同一 buffer：不在每次 poll 中重新分配 256KB
        let mut buf = vec![0u8; UPLOAD_BUF_SIZE];
        let mut state: Option<(u64,)> = Some((0u64,)); // (done,) EOF 后置 None
        let progress2 = progress.clone();
        let stream = futures_util::stream::poll_fn(move |cx| {
            use std::task::Poll;
            let Some((done,)) = state.as_mut() else {
                return Poll::Ready(None);
            };
            let mut rb = tokio::io::ReadBuf::new(&mut buf);
            match std::pin::Pin::new(&mut file).poll_read(cx, &mut rb) {
                Poll::Ready(Ok(())) => {
                    let n = rb.filled().len();
                    if n == 0 {
                        let (d,) = state.take().unwrap();
                        // EOF：最后一条事件必须 done==total（完成前发出）
                        progress2(d, total);
                        return Poll::Ready(None);
                    }
                    *done += n as u64;
                    progress(*done, total);
                    Poll::Ready(Some(Ok(bytes::Bytes::copy_from_slice(rb.filled()))))
                }
                Poll::Ready(Err(e)) => Poll::Ready(Some(Err(e))),
                Poll::Pending => Poll::Pending,
            }
        });
        let body = aws_sdk_s3::primitives::ByteStream::new(aws_smithy_types::body::SdkBody::from_body_1_x(
            http_body_util::StreamBody::new(stream.map_ok(http_body::Frame::data)),
        ));
        client
            .put_object()
            .bucket(bucket)
            .key(key)
            .content_length(total as i64)
            .body(body)
            .send()
            .await
            .map_err(map_s3_err)?;
        Ok(())
    }

    /// 下载对象到本地文件，分块回调进度。
    /// content_length 未知时 total 传 0，流结束后补发最终事件（done=实际字节数）。
    pub async fn download_file(
        client: &Client,
        bucket: &str,
        key: &str,
        path: &std::path::Path,
        progress: std::sync::Arc<dyn Fn(u64, u64) + Send + Sync>,
    ) -> Result<(), S3Error> {
        use tokio::io::AsyncWriteExt;
        let resp = client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await
            .map_err(map_s3_err)?;
        let known_total = resp.content_length().map(|n| n.max(0) as u64);
        let total = known_total.unwrap_or(0);
        progress(0, total);
        let mut file = tokio::fs::File::create(path)
            .await
            .map_err(|e| S3Error::new("LocalFile", format!("{e}")))?;
        let mut stream = resp.body;
        let mut done: u64 = 0;
        let mut last_emit: u64 = 0;
        while let Some(chunk) = stream
            .try_next()
            .await
            .map_err(|e| S3Error::new("Download", format!("{e}")))?
        {
            file.write_all(&chunk)
                .await
                .map_err(|e| S3Error::new("LocalFile", format!("{e}")))?;
            done += chunk.len() as u64;
            // total 未知（0）时按字节量阈值发射，避免每 chunk 刷事件
            if done - last_emit >= DOWNLOAD_EMIT_THRESHOLD {
                progress(done, total);
                last_emit = done;
            }
        }
        file.flush()
            .await
            .map_err(|e| S3Error::new("LocalFile", format!("{e}")))?;
        // 流结束：确保最后一条事件 done==total（total 未知时 total=done=实际字节数），完成前发出
        progress(done, if known_total.is_some() { total.max(done) } else { done });
        Ok(())
    }

    /// 错误映射：解析服务端错误响应，提取真实 Code / Message / x-amz-request-id / HTTP 状态。
    pub fn map_s3_err<E: std::fmt::Debug + aws_smithy_types::error::metadata::ProvideErrorMetadata>(
        e: aws_sdk_s3::error::SdkError<E>,
    ) -> S3Error {
        let display = e.to_string();
        let mut err = match &e {
            aws_sdk_s3::error::SdkError::ServiceError(se) => {
                let meta = se.err().meta();
                let mut err = S3Error {
                    code: meta.code().unwrap_or("ServiceError").to_string(),
                    message: meta
                        .message()
                        .map(|m| m.to_string())
                        .unwrap_or_else(|| display.clone()),
                    request_id: None,
                    http_status: None,
                };
                let resp = se.raw();
                if let Some(rid) = resp.headers().get("x-amz-request-id") {
                    err.request_id = Some(rid.to_string());
                }
                err.http_status = Some(resp.status().as_u16() as i32);
                err
            }
            _ => S3Error::new("SdkError", display),
        };
        if err.message.trim().is_empty() {
            err.message = "（服务端未返回错误详情）".into();
        }
        // 运行日志：便于从 dev 进程 stderr 定位偶发报错（含请求 id 与状态码）
        eprintln!(
            "[S3Err] code={} http={:?} rid={:?} msg={}",
            err.code,
            err.http_status,
            err.request_id.as_deref().unwrap_or("-"),
            err.message.chars().take(200).collect::<String>()
        );
        err
    }
}

/* ---------------- Profile / Secret 存储 ---------------- */

struct Store {
    profiles: Mutex<Vec<Profile>>,
    profiles_path: PathBuf,
    secrets: Mutex<HashMap<String, String>>,
    secrets_path: PathBuf,
    clients: Mutex<HashMap<String, Client>>,
}

fn config_dir(app: &AppHandle) -> PathBuf {
    let dir = app.path().app_config_dir().expect("no config dir");
    fs::create_dir_all(&dir).ok();
    dir
}

/// 以 0600 权限创建/打开后写入（避免"先创建后 chmod"的权限窗口时序问题），
/// 写临时文件 + fs::rename 原子替换（半写文件不会破坏原配置）。
fn write_private(path: &std::path::Path, content: &str) -> Result<(), S3Error> {
    use std::io::Write;
    let tmp = path.with_extension("json.tmp");
    let write = |p: &std::path::Path| -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode_0600()
            .open(p)?;
        f.write_all(content.as_bytes())?;
        f.sync_all().ok();
        #[cfg(unix)]
        {
            // 建库非 0600 时（如已存在的旧文件）也兜底收紧到 0600
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    };
    let write_result = write(&tmp).and_then(|()| fs::rename(&tmp, path));
    match write_result {
        Ok(()) => Ok(()),
        Err(rename_err) => {
            // 兜底：rename 被拒（macOS App Management TCC 对带 com.apple.provenance 属性的
            // 旧文件做 rename 替换会返回 EPERM，尤其 dev 每次重编译签名不同）→
            // 原地截断写入（保 inode 与权限位），并保留 .tmp 内容供人工排查
            let fallback = write(path);
            match fallback {
                Ok(()) => {
                    let _ = fs::remove_file(&tmp);
                    Ok(())
                }
                Err(e2) => {
                    Err(S3Error::new(
                        "WriteConfig",
                        format!("写入 {} 失败: rename: {rename_err}; 原地写入: {e2}", path.display()),
                    ))
                }
            }
        }
    }
}

#[cfg(unix)]
trait OpenOptions0600 {
    fn mode_0600(&mut self) -> &mut Self;
}

#[cfg(unix)]
impl OpenOptions0600 for fs::OpenOptions {
    fn mode_0600(&mut self) -> &mut Self {
        use std::os::unix::fs::OpenOptionsExt;
        self.mode(0o600)
    }
}

#[cfg(not(unix))]
trait OpenOptions0600 {
    fn mode_0600(&mut self) -> &mut Self;
}

#[cfg(not(unix))]
impl OpenOptions0600 for fs::OpenOptions {
    fn mode_0600(&mut self) -> &mut Self {
        self
    }
}

impl Store {
    fn load(app: &AppHandle) -> Self {
        let dir = config_dir(app);
        let profiles_path = dir.join("profiles.json");
        let secrets_path = dir.join("secrets.json");
        let read_json = |path: &std::path::Path| -> Result<String, String> {
            fs::read_to_string(path).map_err(|e| e.to_string())
        };
        let profiles: Vec<Profile> = match read_json(&profiles_path) {
            Ok(s) => match serde_json::from_str(&s) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[sssc] profiles.json 解析失败（{}），忽略并视为空。文件内容未删除。", e);
                    Vec::new()
                }
            },
            Err(_) => Vec::new(), // 文件不存在是首次运行的正常状态
        };
        let secrets: HashMap<String, String> = match read_json(&secrets_path) {
            Ok(s) => match serde_json::from_str(&s) {
                Ok(v) => v,
                Err(e) => {
                    // 不静默丢数据：eprintln 告警 + 坏文件重命名为 secrets.json.bak
                    let bak = secrets_path.with_extension("json.bak");
                    eprintln!(
                        "[sssc] secrets.json 解析失败（{}）！已将坏文件备份为 {}，原位置重置为空。\
                         请检查该文件（可能被手工编辑损坏）后恢复密钥。",
                        e,
                        bak.display()
                    );
                    let _ = fs::rename(&secrets_path, &bak);
                    HashMap::new()
                }
            },
            Err(_) => HashMap::new(),
        };
        Self {
            profiles: Mutex::new(profiles),
            profiles_path,
            secrets: Mutex::new(secrets),
            secrets_path,
            clients: Mutex::new(HashMap::new()),
        }
    }

    /// 把候选状态写入磁盘成功后，才替换内存状态（失败时内存保持不变）。
    fn persist_candidate(
        &self,
        profiles: &Vec<Profile>,
        secrets: &HashMap<String, String>,
    ) -> Result<(), S3Error> {
        let profiles_json = serde_json::to_string_pretty(profiles)
            .map_err(|e| S3Error::new("Serialize", format!("{e}")))?;
        let secrets_json = serde_json::to_string_pretty(secrets)
            .map_err(|e| S3Error::new("Serialize", format!("{e}")))?;
        write_private(&self.profiles_path, &profiles_json)?;
        write_private(&self.secrets_path, &secrets_json)
    }

    async fn client_for(&self, profile_id: &str) -> Result<Client, S3Error> {
        if let Some(c) = self.clients.lock().unwrap().get(profile_id) {
            return Ok(c.clone());
        }
        let profile = {
            let profiles = self.profiles.lock().unwrap();
            match profiles.iter().find(|p| p.id == profile_id) {
                Some(p) => p.clone(),
                None => {
                    return Err(S3Error::new(
                        "ProfileNotFound",
                        format!("profile {profile_id} 不存在"),
                    ))
                }
            }
        };
        let secret = {
            self.secrets
                .lock()
                .unwrap()
                .get(profile_id)
                .cloned()
                .unwrap_or_default()
        };
        if secret.is_empty() {
            return Err(S3Error::new(
                "SecretMissing",
                "该集群未保存密钥（可能由旧版本创建，密钥已随重启丢失）。请点击侧栏「编辑集群」重新录入 Secret Key。",
            ));
        }
        let client = ops::build_client(&profile, &secret).await;
        self.clients
            .lock()
            .unwrap()
            .insert(profile_id.to_string(), client.clone());
        Ok(client)
    }
}

/* ---------------- 传输事件 ---------------- */

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct TransferEvent {
    task_id: String,
    kind: String, // "upload" | "download"
    cluster: String,
    bucket: String,
    key: String,
    local_path: String,
    done: u64,
    total: u64,
    state: String, // "running" | "done" | "failed"
    error: Option<S3Error>,
}

/* ---------------- Tauri Commands ---------------- */

#[tauri::command]
fn list_profiles(store: State<Store>) -> Vec<Profile> {
    store.profiles.lock().unwrap().clone()
}

#[tauri::command]
fn save_profile(store: State<Store>, profile: Profile) -> Result<Profile, S3Error> {
    let mut saved = profile.clone();
    let mut new_secrets = store.secrets.lock().unwrap().clone();
    if let Some(secret) = profile.secret_key.as_deref() {
        if !secret.is_empty() {
            new_secrets.insert(profile.id.clone(), secret.to_string());
            // 多字节 UTF-8 安全掩码：按字符取末 4 位（字节切片遇到中文等会 panic）
            let tail: String = secret.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
            saved.secret_masked = format!("••••{tail}");
        }
    }
    saved.secret_key = None; // 永不回传明文
    // 回滚语义：先构造候选状态并持久化，成功后才改内存（失败时内存不变）
    let mut new_profiles = store.profiles.lock().unwrap().clone();
    new_profiles.retain(|p| p.id != saved.id);
    new_profiles.push(saved.clone());
    store.persist_candidate(&new_profiles, &new_secrets)?;
    *store.profiles.lock().unwrap() = new_profiles;
    *store.secrets.lock().unwrap() = new_secrets;
    store.clients.lock().unwrap().remove(&saved.id);
    Ok(saved)
}

#[tauri::command]
fn delete_profile(store: State<Store>, id: String) -> Result<(), S3Error> {
    // 回滚语义：先持久化删除后的状态，成功才改内存
    let mut new_profiles = store.profiles.lock().unwrap().clone();
    new_profiles.retain(|p| p.id != id);
    let mut new_secrets = store.secrets.lock().unwrap().clone();
    new_secrets.remove(&id);
    store.persist_candidate(&new_profiles, &new_secrets)?;
    *store.profiles.lock().unwrap() = new_profiles;
    *store.secrets.lock().unwrap() = new_secrets;
    store.clients.lock().unwrap().remove(&id);
    Ok(())
}

#[tauri::command]
async fn test_connection(store: State<'_, Store>, profile: Profile) -> Result<ConnectionStatus, S3Error> {
    // 编辑模式未重填密钥（secret_key 为 None）时，取本机已存的密钥完成测试；
    // 两者都没有则按空密钥测试（必然 SignatureDoesNotMatch，前端可见排障提示）。
    let mut profile = profile;
    if profile.secret_key.is_none() {
        if let Some(saved) = store.secrets.lock().unwrap().get(&profile.id).cloned() {
            profile.secret_key = Some(saved);
        }
    }
    Ok(ops::test_connection(&profile).await)
}

#[tauri::command]
async fn list_buckets(store: State<'_, Store>, profile_id: String) -> Result<Vec<Bucket>, S3Error> {
    let client = store.client_for(&profile_id).await?;
    ops::list_buckets(&client).await
}

#[tauri::command]
async fn list_objects(
    store: State<'_, Store>,
    profile_id: String,
    bucket: String,
    prefix: String,
    token: Option<String>,
    // 分页游标（前端"加载更多"回传上次响应的 nextToken）；优先于历史遗留的 token 参数
    continuation_token: Option<String>,
) -> Result<ListResult, S3Error> {
    let client = store.client_for(&profile_id).await?;
    let page_token = continuation_token.or(token);
    ops::list_objects(&client, &bucket, &prefix, page_token.as_deref()).await
}

#[tauri::command]
async fn create_bucket(
    store: State<'_, Store>,
    profile_id: String,
    bucket: String,
) -> Result<(), S3Error> {
    let client = store.client_for(&profile_id).await?;
    ops::create_bucket(&client, &bucket).await
}

#[tauri::command]
async fn delete_bucket(
    store: State<'_, Store>,
    profile_id: String,
    bucket: String,
) -> Result<(), S3Error> {
    let client = store.client_for(&profile_id).await?;
    ops::delete_bucket(&client, &bucket).await
}

#[tauri::command]
async fn delete_object(
    store: State<'_, Store>,
    profile_id: String,
    bucket: String,
    key: String,
) -> Result<(), S3Error> {
    let client = store.client_for(&profile_id).await?;
    // 使用怪癖规避版删除（SeaweedFS 读后删 500：退避重试 + 覆盖重写）
    ops::delete_object_resilient(&client, &bucket, &key).await
}

/// 递归删除"文件夹"（前缀下全部对象，含占位对象本身），批量 DeleteObjects
#[tauri::command]
async fn delete_folder(
    store: State<'_, Store>,
    profile_id: String,
    bucket: String,
    prefix: String,
) -> Result<DeleteFolderResult, S3Error> {
    let client = store.client_for(&profile_id).await?;
    ops::delete_folder(&client, &bucket, &prefix).await
}

/// 一次请求完成整批删除（文件 DeleteObjects 批量 + 文件夹递归展开），
/// 通过 delete://progress 事件实时上报进度（每批 1000 个对象一次事件）。
#[tauri::command]
async fn delete_batch(
    app: AppHandle,
    store: State<'_, Store>,
    profile_id: String,
    bucket: String,
    keys: Vec<String>,
    prefixes: Vec<String>,
    batch_id: String,
) -> Result<DeleteBatchResult, S3Error> {
    let client = store.client_for(&profile_id).await?;
    let emit_progress = |done: usize, total: usize, phase: &str| {
        let _ = app.emit(
            "delete://progress",
            &DeleteProgress { batch_id: batch_id.clone(), done, total, phase: phase.into() },
        );
    };
    emit_progress(0, 0, "listing");
    let keys_ref = &keys;
    let prefixes_ref = &prefixes;
    let r = ops::delete_many(&client, &bucket, keys_ref, prefixes_ref, &|done, total| {
        emit_progress(done, total, "deleting");
    })
    .await;
    // 最终进度事件（100%）
    if let Ok(res) = &r {
        emit_progress(res.deleted_count + res.error_count, res.deleted_count + res.error_count, "done");
    }
    r
}

#[tauri::command]
async fn create_folder(
    store: State<'_, Store>,
    profile_id: String,
    bucket: String,
    key: String,
) -> Result<(), S3Error> {
    let client = store.client_for(&profile_id).await?;
    ops::create_folder(&client, &bucket, &key).await
}

#[tauri::command]
async fn presign_get(
    store: State<'_, Store>,
    profile_id: String,
    bucket: String,
    key: String,
    expires_sec: Option<u64>,
) -> Result<String, S3Error> {
    let expires = expires_sec.unwrap_or(86400).min(ops::MAX_PRESIGN_EXPIRES);
    let client = store.client_for(&profile_id).await?;
    // V2 集群使用查询串式 V2 预签名（V4 SDK presign 不适用）
    let profile = {
        let profiles = store.profiles.lock().unwrap();
        profiles.iter().find(|p| p.id == profile_id).cloned()
    };
    if let Some(p) = profile.filter(|p| p.signature == Signature::V2) {
        let secret = store
            .secrets
            .lock()
            .unwrap()
            .get(&profile_id)
            .cloned()
            .unwrap_or_default();
        let endpoint = p.endpoint.trim_end_matches('/');
        // path-style: endpoint/bucket/key。
        // SigV2 规范：签名使用原始（未编码）path，URL 中的 path 再做编码
        // （sigv2::presign_get_url 内部处理；部分网关对预编码 path 验签失败）。
        let path = format!("/{}/{}", bucket, key);
        let expires_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            + expires;
        return Ok(crate::sigv2::presign_get_url(
            endpoint,
            &path,
            &p.access_key,
            &secret,
            expires_epoch,
        ));
    }
    ops::presign_get(&client, &bucket, &key, expires).await
}

fn emit_transfer(app: &AppHandle, ev: TransferEvent) {
    let _ = app.emit("transfer://event", &ev);
}

/// 生成唯一 task_id：时间戳 + 进程内原子计数器（纯毫秒时间戳并发下会撞车）。
fn new_task_id(prefix: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{prefix}_{ts}_{n}")
}

/// 通用传输流程：发 running 事件 → 执行传输（带进度事件）→ 发 done/failed 事件。
async fn run_transfer<F, Fut>(
    app: AppHandle,
    store: &State<'_, Store>,
    profile_id: &str,
    bucket: &str,
    key: &str,
    local_path: &str,
    kind: &str,
    prefix: &str,
    do_transfer: F,
) -> Result<String, S3Error>
where
    F: FnOnce(Client, std::path::PathBuf, std::sync::Arc<dyn Fn(u64, u64) + Send + Sync>) -> Fut,
    Fut: std::future::Future<Output = Result<(), S3Error>>,
{
    let task_id = new_task_id(prefix);
    let cluster = {
        store
            .profiles
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.id == profile_id)
            .map(|p| p.name.clone())
            .unwrap_or_default()
    };
    let base = TransferEvent {
        task_id: task_id.clone(),
        kind: kind.into(),
        cluster,
        bucket: bucket.to_string(),
        key: key.to_string(),
        local_path: local_path.to_string(),
        done: 0,
        total: 0,
        state: "running".into(),
        error: None,
    };
    emit_transfer(&app, base.clone());
    let client = store.client_for(profile_id).await?;
    let path = std::path::PathBuf::from(local_path);
    let app2 = app.clone();
    let base2 = base.clone();
    let result = do_transfer(
        client,
        path,
        std::sync::Arc::new(move |d, t| {
            let mut ev = base2.clone();
            ev.done = d;
            ev.total = t;
            emit_transfer(&app2, ev);
        }),
    )
    .await;
    match result {
        Ok(()) => {
            let mut ev = base.clone();
            ev.state = "done".into();
            emit_transfer(&app, ev);
            Ok(task_id)
        }
        Err(e) => {
            let mut ev = base;
            ev.state = "failed".into();
            ev.error = Some(e.clone());
            emit_transfer(&app, ev);
            Err(e)
        }
    }
}

#[tauri::command]
async fn upload_file(
    app: AppHandle,
    store: State<'_, Store>,
    profile_id: String,
    bucket: String,
    key: String,
    local_path: String,
) -> Result<String, S3Error> {
    run_transfer(
        app,
        &store,
        &profile_id,
        &bucket,
        &key,
        &local_path,
        "upload",
        "up",
        {
            let bucket = bucket.clone();
            let key = key.clone();
            move |client, path, progress| async move {
                ops::upload_file(&client, &bucket, &key, &path, progress).await
            }
        },
    )
    .await
}

#[tauri::command]
async fn download_file(
    app: AppHandle,
    store: State<'_, Store>,
    profile_id: String,
    bucket: String,
    key: String,
    local_path: String,
) -> Result<String, S3Error> {
    run_transfer(
        app,
        &store,
        &profile_id,
        &bucket,
        &key,
        &local_path,
        "download",
        "down",
        {
            let bucket = bucket.clone();
            let key = key.clone();
            move |client, path, progress| async move {
                ops::download_file(&client, &bucket, &key, &path, progress).await
            }
        },
    )
    .await
}

/* ---------------- 入口 ---------------- */

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(Store::load(app.handle()));
            #[cfg(target_os = "macos")]
            set_macos_dock_icon();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_profiles,
            save_profile,
            delete_profile,
            test_connection,
            list_buckets,
            list_objects,
            create_bucket,
            delete_bucket,
            delete_object,
            delete_folder,
            delete_batch,
            create_folder,
            presign_get,
            upload_file,
            download_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// macOS dev 模式下 tauri 不打包 bundle，Dock 显示默认图标；
/// 这里在启动时用 NSApplication 显式设置应用图标（生产构建由 bundle.icon 覆盖）。
#[cfg(target_os = "macos")]
fn set_macos_dock_icon() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSImage};
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let bytes = include_bytes!("../icons/icon.png");
    let data = objc2_foundation::NSData::with_bytes(bytes);
    if let Some(image) = NSImage::initWithData(mtm.alloc(), &data) {
        let app = NSApplication::sharedApplication(mtm);
        // 仅改写当前进程 Dock 图标，无越界/空指针风险
        unsafe { app.setApplicationIconImage(Some(&image)) };
    }
}
