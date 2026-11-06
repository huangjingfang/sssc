//! AWS Signature V2 实现（S3 旧版签名）。
//!
//! 动机：大量研发私有环境通过 nginx 反代挂 Ceph RGW，nginx 默认
//! `proxy_set_header Host $proxy_host` 会改写 Host，而 SigV4 把 Host 签入
//! 签名导致 SignatureDoesNotMatch；SigV2 不签 Host，对这类代理天然免疫
//! （旧版 s3browser 等客户端正是因此可正常直连）。
//!
//! 实现方式：SDK（aws-sdk-s3）仅支持 V4，因此通过 `http_client_fn`
//! 包装默认连接器——在请求发出前把 Authorization 重签为 V2。
//! 参考：AWS S3 REST 认证（已归档的 SigV2 规范）。

use aws_smithy_runtime_api::client::http::{
    http_client_fn, SharedHttpConnector, HttpConnector, HttpConnectorFuture,
};
use aws_smithy_types::body::SdkBody;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// 参与 CanonicalizedResource 的 V2 子资源（AWS 规范列表）。
const SUB_RESOURCES: &[&str] = &[
    "acl", "cors", "defaultObjectAcl", "location", "logging", "partNumber", "policy",
    "requestPayment", "torrent", "versioning", "versionId", "versions", "website",
    "uploads", "uploadId", "tagging", "restore", "storageClass", "lifecycle",
    "delete", "response-content-type", "response-content-language", "response-expires",
    "response-cache-control", "response-content-disposition", "response-content-encoding",
];

/// 计算 V2 StringToSign。
///
/// GET\nContent-MD5\nContent-Type\nDate\nCanonicalizedAmzHeaders + CanonicalizedResource
pub fn string_to_sign(
    method: &str,
    path: &str,
    query: &str,
    content_md5: &str,
    content_type: &str,
    date: &str,
    amz_headers: &[(String, String)],
) -> String {
    // CanonicalizedAmzHeaders：x-amz-* 小写排序，去值首尾空白，name:value\n
    let mut amz: Vec<(String, String)> = amz_headers
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.trim().to_string()))
        .filter(|(k, _)| k.starts_with("x-amz-"))
        .collect();
    amz.sort();
    let mut sts = String::new();
    sts.push_str(method);
    sts.push('\n');
    sts.push_str(content_md5);
    sts.push('\n');
    sts.push_str(content_type);
    sts.push('\n');
    sts.push_str(date);
    sts.push('\n');
    for (k, v) in &amz {
        sts.push_str(k);
        sts.push(':');
        sts.push_str(v);
        sts.push('\n');
    }
    // CanonicalizedResource：path + 排序后的子资源 query
    let mut sub: Vec<(String, String)> = query
        .split('&')
        .filter(|s| !s.is_empty())
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            SUB_RESOURCES
                .contains(&k)
                .then(|| (k.to_string(), v.to_string()))
        })
        .collect();
    sub.sort();
    sts.push_str(path);
    if !sub.is_empty() {
        sts.push('?');
        let pairs: Vec<String> = sub
            .iter()
            .map(|(k, v)| if v.is_empty() { k.clone() } else { format!("{k}={v}") })
            .collect();
        sts.push_str(&pairs.join("&"));
    }
    sts
}

/// HMAC-SHA1 + Base64 计算 V2 签名。
pub fn sign(secret: &str, sts: &str) -> String {
    let mut mac = HmacSha1::new_from_slice(secret.as_bytes()).expect("hmac accepts any key length");
    mac.update(sts.as_bytes());
    B64.encode(mac.finalize().into_bytes())
}

/// V2 查询串式预签名（GET 下载链接）。
///
/// `path` 为**原始（未 URL 编码）**请求路径，如 `/bucket/dir/中文.txt`：
/// - 签名（StringToSign）使用原始 path —— SigV2 规范即如此，
///   先编码再签名会导致部分网关（对解码后 path 验签）SignatureDoesNotMatch；
/// - URL 中 path 按 `/` 分段逐段百分号编码。
pub fn presign_get_url(
    endpoint: &str,
    path: &str,
    access_key: &str,
    secret: &str,
    expires_epoch: u64,
) -> String {
    // 规范：对原始 path 签名
    let sts = format!("GET\n\n\n{expires_epoch}\n{path}");
    let sig = sign(secret, &sts);
    // URL：对原始 path 逐段编码
    let path_enc: String = path
        .split('/')
        .map(url_encode)
        .collect::<Vec<_>>()
        .join("/");
    format!(
        "{endpoint}{}?AWSAccessKeyId={}&Expires={}&Signature={}",
        path_enc,
        url_encode(access_key),
        expires_epoch,
        url_encode(&sig)
    )
}

/// 最小 URL 编码（query 值安全）。
pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 包装默认连接器：请求发出前将 Authorization 重签为 SigV2。
#[derive(Debug)]
pub struct V2Connector {
    inner: SharedHttpConnector,
    access_key: std::sync::Arc<String>,
    secret: std::sync::Arc<String>,
}

impl HttpConnector for V2Connector {
    fn call(&self, request: aws_smithy_runtime_api::http::Request<SdkBody>) -> HttpConnectorFuture {
        let req = resign_v2(request, &self.access_key, &self.secret);
        self.inner.call(req)
    }
}

/// 构造 SigV2 http client（挂到 config Builder 的 `.http_client()`）。
pub fn v2_http_client(
    access_key: String,
    secret: String,
) -> aws_sdk_s3::config::SharedHttpClient {
    let ak = std::sync::Arc::new(access_key);
    let sk = std::sync::Arc::new(secret);
    http_client_fn(move |settings, components| {
        let inner = aws_smithy_http_client::default_connector(
            settings,
            components.sleep_impl(),
        )
        .expect("默认 connector 不可用（TLS feature 未启用？）");
        SharedHttpConnector::new(V2Connector {
            inner,
            access_key: ak.clone(),
            secret: sk.clone(),
        })
    })
}

/// 将 SDK 生成的 V4 请求重签为 V2。
fn resign_v2(
    mut request: aws_smithy_runtime_api::http::Request<SdkBody>,
    access_key: &str,
    secret: &str,
) -> aws_smithy_runtime_api::http::Request<SdkBody> {
    let date = httpdate::fmt_http_date(std::time::SystemTime::now());

    {
        let headers = request.headers_mut();
        // V4 专属头移除（x-amz-date 会进入 V2 的 canonical amz 头导致不一致）
        headers.remove("x-amz-date");
        headers.remove("x-amz-content-sha256");
        headers.remove("authorization");
        headers.insert("date", date.clone());
    }

    let method = request.method().to_string();
    let path = request.uri_mut().path().to_string();
    let query = request.uri_mut().query().unwrap_or("").to_string();
    let content_md5 = request.headers().get("content-md5").unwrap_or("").to_string();
    let content_type = request.headers().get("content-type").unwrap_or("").to_string();
    let amz_headers: Vec<(String, String)> = request
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let sts = string_to_sign(
        &method,
        &path,
        &query,
        &content_md5,
        &content_type,
        &date,
        &amz_headers,
    );
    let sig = sign(secret, &sts);
    let auth = format!("AWS {access_key}:{sig}");
    request.headers_mut().insert("authorization", auth.clone());
    request
}

/* ---------------- 单元测试（AWS 官方文档 V2 测试向量） ---------------- */

#[cfg(test)]
mod tests {
    use super::*;

    /// AWS S3 SigV2 官方示例：
    /// GET /johnsmith/photos/puppy.jpg，2007-03-27T19:36:42+0000
    #[test]
    fn aws_doc_vector_get_object() {
        let sts = string_to_sign(
            "GET",
            "/johnsmith/photos/puppy.jpg",
            "",
            "",
            "",
            "Tue, 27 Mar 2007 19:36:42 +0000",
            &[],
        );
        assert_eq!(
            sts,
            "GET\n\n\nTue, 27 Mar 2007 19:36:42 +0000\n/johnsmith/photos/puppy.jpg"
        );
        let sig = sign("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", &sts);
        assert_eq!(sig, "bWq2s1WEIj+Ydj0vQ697zp+IXMU=");
    }

    /// AWS 官方示例：PUT Object + Content-Type
    #[test]
    fn aws_doc_vector_put_object() {
        let sts = string_to_sign(
            "PUT",
            "/johnsmith/photos/puppy.jpg",
            "",
            "",
            "image/jpeg",
            "Tue, 27 Mar 2007 21:15:45 +0000",
            &[],
        );
        let sig = sign("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", &sts);
        assert_eq!(sig, "MyyxeRY7whkBe+bq8fHCL/2kKUg=");
    }

    /// AWS 官方示例：ListBucket（query 参数不是子资源，不参与签名路径）
    #[test]
    fn aws_doc_vector_list_bucket() {
        let sts = string_to_sign(
            "GET",
            "/johnsmith/",
            "prefix=photos&max-keys=50&marker=puppy",
            "",
            "",
            "Tue, 27 Mar 2007 19:42:41 +0000",
            &[],
        );
        assert!(sts.ends_with("/johnsmith/"), "非子资源 query 不得进入签名: {sts}");
        let sig = sign("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", &sts);
        assert_eq!(sig, "htDYFYduRNen8P9ZfE/s9SuKy0U=");
    }

    /// 子资源（如 uploadId）参与 CanonicalizedResource
    #[test]
    fn subresource_in_signature() {
        let sts = string_to_sign(
            "DELETE",
            "/bucket/key",
            "uploadId=abc&x=1",
            "",
            "",
            "Thu, 05 Nov 2026 08:00:00 +0000",
            &[],
        );
        assert!(sts.ends_with("/bucket/key?uploadId=abc"), "实际: {sts}");
    }

    /// x-amz 头参与 CanonicalizedAmzHeaders（小写、排序）
    #[test]
    fn amz_headers_canonicalized() {
        let sts = string_to_sign(
            "GET",
            "/b/k",
            "",
            "",
            "",
            "Thu, 05 Nov 2026 08:00:00 +0000",
            &[
                ("X-Amz-Meta-B".into(), " two ".into()),
                ("X-Amz-Meta-A".into(), "one".into()),
                ("Ignore-Me".into(), "no".into()),
            ],
        );
        assert!(sts.contains("x-amz-meta-a:one\nx-amz-meta-b:two\n/b/k"), "实际: {sts}");
    }

    /// 预签名：对原始（未编码）path 签名，URL 中再编码（SigV2 规范）。
    /// 签名必须与手工对原始 path 计算 StringToSign 的结果一致。
    #[test]
    fn presign_signs_raw_path_and_encodes_url() {
        let raw = "/hdd-demo/it-dir/中文 测试.txt";
        let exp = 1_800_000_000u64;
        let url = presign_get_url("http://gw:8333", raw, "AK", "SK", exp);
        // URL 中路径已编码，原始字符不出现
        assert!(!url.contains("中文"), "URL 应编码非 ASCII: {url}");
        assert!(url.contains("%E4%B8%AD%E6%96%87"), "URL 应含百分号编码: {url}");
        // 手工按规范对原始 path 计算 StringToSign
        let sts = format!("GET\n\n\n{exp}\n{raw}");
        let expect_sig = url_encode(&sign("SK", &sts));
        assert!(url.ends_with(&format!("&Signature={expect_sig}")), "签名应基于原始 path: {url}");
        // ASCII path 行为不变（向量自洽）
        let url2 = presign_get_url("http://gw:8333", "/b/k.txt", "AK", "SK", exp);
        let sts2 = format!("GET\n\n\n{exp}\n/b/k.txt");
        assert!(url2.ends_with(&format!("&Signature={}", url_encode(&sign("SK", &sts2)))));
    }
}
