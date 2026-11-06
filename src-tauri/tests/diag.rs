//! 临时诊断：SeaweedFS 流式 vs 缓冲上传（定位 500 InternalError）。
//!
//! 凭据全部来自环境变量（不硬编码任何真实值）：
//!   SSSC_ENDPOINT / SSSC_AK / SSSC_SK（必需，缺失则跳过测试）
//!   SSSC_REGION（可选，默认 us-east-1）
//!   SSSC_IT_WORK_BUCKET（可选，默认 hdd-demo）
use sssc_lib::{ops, Profile};

/// 读取目标集群凭据；任一缺失则打印跳过原因并返回 None（测试早退）。
fn creds(signature: sssc_lib::Signature) -> Option<(Profile, String)> {
    let (endpoint, ak, sk) = match (
        std::env::var("SSSC_ENDPOINT"),
        std::env::var("SSSC_AK"),
        std::env::var("SSSC_SK"),
    ) {
        (Ok(e), Ok(a), Ok(s)) if !e.is_empty() && !a.is_empty() && !s.is_empty() => (e, a, s),
        _ => {
            eprintln!(
                "SKIP: 需要环境变量 SSSC_ENDPOINT / SSSC_AK / SSSC_SK（缺少则本测试跳过，不使用任何默认真实值）"
            );
            return None;
        }
    };
    Some((
        Profile {
            id: "d".into(),
            name: "d".into(),
            endpoint,
            region: std::env::var("SSSC_REGION").unwrap_or_else(|_| "us-east-1".into()),
            access_key: ak,
            secret_masked: String::new(),
            style: sssc_lib::Style::Path,
            insecure_tls: false,
            color: "blue".into(),
            enabled: true,
            signature,
            secret_key: Some(sk.clone()),
        },
        sk,
    ))
}

fn work_bucket() -> String {
    std::env::var("SSSC_IT_WORK_BUCKET").unwrap_or_else(|_| "hdd-demo".into())
}

fn ts() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[tokio::test]
async fn diag_upload_variants() {
    let Some((p, sk)) = creds(sssc_lib::Signature::V4) else { return };
    let client = ops::build_client(&p, &sk).await;
    let bucket = format!("sssc-diag-{}", ts()); // 每次独立桶，避免依赖残留
    ops::create_bucket(&client, &bucket).await.ok();
    let data = vec![7u8; 2 * 1024 * 1024];

    // 变体1：缓冲（ByteStream::from(Vec)）
    let r1 = client.put_object().bucket(&bucket).key("diag-buffered.bin")
        .body(aws_sdk_s3::primitives::ByteStream::from(data.clone()))
        .send().await;
    println!("buffered 2MB: {}", match &r1 { Ok(_) => "OK".into(), Err(e) => format!("{e:?}") });

    // 变体2：流式 from_path（同尺寸）
    let path = std::env::temp_dir().join("sssc-diag-2m.bin");
    std::fs::write(&path, &data).unwrap();
    let r2 = client.put_object().bucket(&bucket).key("diag-stream.bin")
        .content_length(data.len() as i64)
        .body(aws_sdk_s3::primitives::ByteStream::from_path(&path).await.unwrap())
        .send().await;
    println!("streamed 2MB: {}", match &r2 { Ok(_) => "OK".into(), Err(e) => format!("{e:?}") });

    // 变体3：流式小文件
    let path1k = std::env::temp_dir().join("sssc-diag-1k.bin");
    std::fs::write(&path1k, vec![7u8; 1024]).unwrap();
    let r3 = client.put_object().bucket(&bucket).key("diag-stream-1k.bin")
        .content_length(1024)
        .body(aws_sdk_s3::primitives::ByteStream::from_path(&path1k).await.unwrap())
        .send().await;
    println!("streamed 1KB: {}", match &r3 { Ok(_) => "OK".into(), Err(e) => format!("{e:?}") });

    let _ = ops::delete_object(&client, &bucket, "diag-buffered.bin").await;
    let _ = ops::delete_object(&client, &bucket, "diag-stream.bin").await;
    let _ = ops::delete_object(&client, &bucket, "diag-stream-1k.bin").await;
    let _ = ops::delete_bucket(&client, &bucket).await;
}

#[tokio::test]
async fn diag_existing_bucket() {
    let Some((p, sk)) = creds(sssc_lib::Signature::V4) else { return };
    let client = ops::build_client(&p, &sk).await;
    let bucket = work_bucket();
    let r = client.put_object().bucket(&bucket).key("diag-existing.bin")
        .body(aws_sdk_s3::primitives::ByteStream::from(vec![7u8; 1024]))
        .send().await;
    println!("existing bucket 1KB: {}", if r.is_ok() { "OK" } else { "FAIL" });
    if r.is_ok() {
        let _ = client.delete_object().bucket(&bucket).key("diag-existing.bin").send().await;
    }
}

#[tokio::test]
async fn diag_create_then_upload_delay() {
    let Some((p, sk)) = creds(sssc_lib::Signature::V4) else { return };
    let client = ops::build_client(&p, &sk).await;
    let bucket = format!("sssc-diag-{}", ts());
    ops::create_bucket(&client, &bucket).await.unwrap();
    for (delay, label) in [(0u64, "0s"), (2, "2s"), (5, "5s")] {
        tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        let key = format!("diag-{label}.bin");
        let r = client.put_object().bucket(&bucket).key(&key)
            .body(aws_sdk_s3::primitives::ByteStream::from(vec![7u8; 1024]))
            .send().await;
        println!("create→upload after {label}: {}", if r.is_ok() { "OK" } else { "FAIL" });
        if r.is_ok() { break; }
    }
    // 清理尽力而为
    let _ = client.delete_bucket().bucket(&bucket).send().await;
}

#[tokio::test]
async fn diag_delete_variants() {
    let Some((p, sk)) = creds(sssc_lib::Signature::V4) else { return };
    let client = ops::build_client(&p, &sk).await;
    let bucket = work_bucket();
    // 扁平 key
    let r1 = client.put_object().bucket(&bucket).key("diag-flat.bin")
        .body(aws_sdk_s3::primitives::ByteStream::from(vec![1u8; 16])).send().await;
    println!("put flat: {}", if r1.is_ok() { "OK" } else { "FAIL" });
    let d1 = client.delete_object().bucket(&bucket).key("diag-flat.bin").send().await;
    println!("delete flat: {}", if d1.is_ok() { "OK" } else { "FAIL" });
    // 目录 key
    let r2 = client.put_object().bucket(&bucket).key("diag-dir/x.bin")
        .body(aws_sdk_s3::primitives::ByteStream::from(vec![1u8; 16])).send().await;
    println!("put dir: {}", if r2.is_ok() { "OK" } else { "FAIL" });
    let d2 = client.delete_object().bucket(&bucket).key("diag-dir/x.bin").send().await;
    println!("delete dir: {}", if d2.is_ok() { "OK" } else { "FAIL" });
    // 深目录
    let r3 = client.put_object().bucket(&bucket).key("diag-a/diag-b/y.bin")
        .body(aws_sdk_s3::primitives::ByteStream::from(vec![1u8; 16])).send().await;
    println!("put deep dir: {}", if r3.is_ok() { "OK" } else { "FAIL" });
    let d3 = client.delete_object().bucket(&bucket).key("diag-a/diag-b/y.bin").send().await;
    println!("delete deep dir: {}", if d3.is_ok() { "OK" } else { "FAIL" });
}

#[tokio::test]
async fn diag_delete_retry() {
    let Some((p, sk)) = creds(sssc_lib::Signature::V4) else { return };
    let client = ops::build_client(&p, &sk).await;
    let bucket = work_bucket();
    let key = format!("diag-retry-{}.bin", ts());
    client.put_object().bucket(&bucket).key(&key)
        .body(aws_sdk_s3::primitives::ByteStream::from(vec![1u8; 16])).send().await.unwrap();
    for i in 1..=5 {
        let d = client.delete_object().bucket(&bucket).key(&key).send().await;
        let ok = d.is_ok();
        // 检查对象是否仍存在
        let listed = ops::list_objects(&client, &bucket, "", None).await.unwrap()
            .objects.iter().any(|o| o.key == key);
        println!("delete try#{i}: {} / still-listed={}", if ok { "200" } else { "500" }, listed);
        if ok || !listed { break; }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

#[tokio::test]
async fn diag_delete_size_threshold() {
    let Some((p, sk)) = creds(sssc_lib::Signature::V4) else { return };
    let client = ops::build_client(&p, &sk).await;
    let bucket = work_bucket();
    for size in [1024usize, 64 * 1024, 256 * 1024, 512 * 1024, 1024 * 1024, 2 * 1024 * 1024] {
        let key = format!("diag-size-{}.bin", size);
        client.put_object().bucket(&bucket).key(&key)
            .body(aws_sdk_s3::primitives::ByteStream::from(vec![1u8; size])).send().await.unwrap();
        let d = client.delete_object().bucket(&bucket).key(&key).send().await;
        println!("size {size:>7}: delete {}", if d.is_ok() { "OK" } else { "500-FAIL" });
    }
}

#[tokio::test]
async fn diag_streamed_delete() {
    let Some((p, sk)) = creds(sssc_lib::Signature::V4) else { return };
    let client = ops::build_client(&p, &sk).await;
    let bucket = work_bucket();
    let ts = ts();
    let path = std::env::temp_dir().join(format!("sssc-diag-st-{}.bin", ts));
    std::fs::write(&path, vec![7u8; 2 * 1024 * 1024]).unwrap();
    ops::upload_file(&client, &bucket, &format!("it-dir/st-{}.bin", ts), &path, std::sync::Arc::new(|_, _| {}) as _).await.unwrap();
    for i in 1..=3 {
        let key = format!("it-dir/st-{}.bin", ts);
        let d = client.delete_object().bucket(&bucket).key(&key).send().await;
        println!("streamed+dir delete try#{i}: {}", if d.is_ok() { "200-OK" } else { "500-FAIL" });
        if d.is_ok() { break; }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn diag_read_then_delete() {
    let Some((p, sk)) = creds(sssc_lib::Signature::V4) else { return };
    let client = ops::build_client(&p, &sk).await;
    let bucket = work_bucket();
    let ts = ts();
    let key = format!("it-dir/rt-{}.bin", ts);
    let path = std::env::temp_dir().join(format!("sssc-diag-rt-{}.bin", ts));
    std::fs::write(&path, vec![7u8; 2 * 1024 * 1024]).unwrap();
    ops::upload_file(&client, &bucket, &key, &path, std::sync::Arc::new(|_, _| {}) as _).await.unwrap();
    // 预签名 GET（匿名 HTTP 读）
    let url = ops::presign_get(&client, &bucket, &key, 300).await.unwrap();
    if let Some(hp) = url.strip_prefix("http://").and_then(|r| r.split('/').next().map(|s| s.to_string())) {
        let _ = reqwest_lite(&url, &hp).await;
    }
    // SDK 下载
    let _ = ops::download_file(&client, &bucket, &key, &std::env::temp_dir().join(format!("sssc-rt-dl-{}.bin", ts)), std::sync::Arc::new(|_, _| {}) as _).await;
    // 立即删
    for i in 1..=5 {
        let d = client.delete_object().bucket(&bucket).key(&key).send().await;
        println!("read-then-delete try#{i}: {}", if d.is_ok() { "200-OK" } else { "500-FAIL" });
        if d.is_ok() { break; }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let _ = std::fs::remove_file(&path);
}

async fn reqwest_lite(url: &str, hostport: &str) -> Vec<u8> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let path = url.splitn(4, '/').nth(3).unwrap_or("");
    let mut s = tokio::net::TcpStream::connect(hostport).await.unwrap();
    s.write_all(format!("GET /{path} HTTP/1.1\r\nHost: {hostport}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    let mut b = Vec::new(); s.read_to_end(&mut b).await.unwrap(); b
}

#[tokio::test]
async fn zz_cleanup_hdd_demo_root() {
    let Some((p, sk)) = creds(sssc_lib::Signature::V4) else { return };
    let client = ops::build_client(&p, &sk).await;
    let bucket = work_bucket();
    let list = ops::list_objects(&client, &bucket, "", None).await.unwrap();
    let fresh = std::env::temp_dir().join("sssc-clean.bin");
    std::fs::write(&fresh, b"x").unwrap();
    for o in list.objects {
        if o.key.starts_with("diag-") || o.key.starts_with("sssc-") {
            let _ = ops::upload_file(&client, &bucket, &o.key, &fresh, std::sync::Arc::new(|_, _| {}) as _).await;
            let ok = ops::delete_object(&client, &bucket, &o.key).await.is_ok();
            println!("cleanup {}: {}", o.key, if ok { "✓" } else { "FAIL" });
        } else {
            println!("kept: {}", o.key);
        }
    }
}

/// V2 签名端到端（凭据来自 SSSC_* 环境变量）
#[tokio::test]
async fn diag_sigv2_seaweed() {
    let Some((p, _sk)) = creds(sssc_lib::Signature::V2) else { return };
    let st = ops::test_connection(&p).await;
    println!("SigV2 → {}: ok={} err={:?}", p.endpoint, st.ok, st.error.as_ref().map(|e| e.code.clone()));
}

/// V2 签名端到端：多 endpoint 场景（如 nginx 80 / rgw 直连 7480）。
/// endpoint 用 SSSC_CEPH_ENDPOINTS（逗号分隔）+ SSSC_CEPH_AK / SSSC_CEPH_SK 提供：
///   SSSC_CEPH_ENDPOINTS="http://10.x.x.x,http://10.x.x.x:7480" \
///   SSSC_CEPH_AK=xxx SSSC_CEPH_SK=xxx cargo test --test diag diag_sigv2_ceph -- --nocapture
#[tokio::test]
#[ignore]
async fn diag_sigv2_ceph() {
    let endpoints = std::env::var("SSSC_CEPH_ENDPOINTS").expect("需要 SSSC_CEPH_ENDPOINTS");
    let ak = std::env::var("SSSC_CEPH_AK").expect("需要 SSSC_CEPH_AK");
    let sk = std::env::var("SSSC_CEPH_SK").expect("需要 SSSC_CEPH_SK");
    for (i, endpoint) in endpoints.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).enumerate() {
        let p = Profile {
            id: "d".into(), name: "d".into(),
            endpoint: endpoint.into(),
            region: "us-east-1".into(),
            access_key: ak.clone(),
            secret_masked: String::new(),
            style: sssc_lib::Style::Path, insecure_tls: false, color: "blue".into(), enabled: true,
            signature: sssc_lib::Signature::V2,
            secret_key: Some(sk.clone()),
        };
        let st = ops::test_connection(&p).await;
        println!("SigV2 → endpoint#{i}({endpoint}): ok={} buckets={:?} err={:?}",
            st.ok, st.bucket_count, st.error.as_ref().map(|e| e.code.clone()));
    }
}

/// V2 签名全操作验证（真实集群：list/upload/download/presign/delete）
#[tokio::test]
async fn diag_sigv2_full_ops() {
    let Some((p, secret)) = creds(sssc_lib::Signature::V2) else { return };
    let client = ops::build_client(&p, &secret).await;
    let bucket = work_bucket();
    let ts = ts();
    let key = format!("it-dir/v2-{}.bin", ts);

    let local = std::env::temp_dir().join(format!("v2-{}.bin", ts));
    std::fs::write(&local, vec![9u8; 64 * 1024]).unwrap();
    ops::upload_file(&client, &bucket, &key, &local, std::sync::Arc::new(|_, _| {}) as _).await.expect("V2 上传失败");
    println!("V2 upload ✓");

    let listed = ops::list_objects(&client, &bucket, "it-dir/", None).await.unwrap();
    assert!(listed.objects.iter().any(|o| o.key == key));
    println!("V2 list ✓");

    let dl = std::env::temp_dir().join(format!("v2-dl-{}.bin", ts));
    ops::download_file(&client, &bucket, &key, &dl, std::sync::Arc::new(|_, _| {}) as _).await.expect("V2 下载失败");
    assert_eq!(std::fs::read(&dl).unwrap(), vec![9u8; 64 * 1024]);
    println!("V2 download ✓（逐字节一致）");

    // V2 预签名（查询串式；注意：此处仅验证 URL 生成，不做网络验证以兼容 https endpoint）
    let path = format!("/{}/{}", bucket, key);
    let exp = ts as u64 / 1000 + 300;
    let url = sssc_lib::sigv2::presign_get_url(p.endpoint.trim_end_matches('/'), &path, &p.access_key, &secret, exp);
    assert!(url.starts_with(p.endpoint.trim_end_matches('/')), "预签名 URL 应指向目标 endpoint: {url}");
    println!("V2 presign ✓（URL 已生成：{url}）");

    let _ = ops::delete_object(&client, &bucket, &key).await;
    let _ = std::fs::remove_file(&local); let _ = std::fs::remove_file(&dl);
    println!("V2 全操作验证通过");
}

/// 导出真实 ListObjectsV2 JSON 供前端渲染复现
#[tokio::test]
async fn dump_real_list() {
    let Some((p, sk)) = creds(sssc_lib::Signature::V4) else { return };
    let client = ops::build_client(&p, &sk).await;
    let bucket = work_bucket();
    let list = ops::list_objects(&client, &bucket, "", None).await.unwrap();
    std::fs::write("/tmp/sssc-real-list.json", serde_json::to_string_pretty(&list).unwrap()).unwrap();
    let buckets = ops::list_buckets(&client).await.unwrap();
    std::fs::write("/tmp/sssc-real-buckets.json", serde_json::to_string_pretty(&buckets).unwrap()).unwrap();
    println!("dumped");
}

/// 中文文件名端到端：上传 → 列举显示 → 下载，确定编码发生在哪一层
#[tokio::test]
async fn diag_chinese_key() {
    let Some((p, sk)) = creds(sssc_lib::Signature::V4) else { return };
    let client = ops::build_client(&p, &sk).await;
    let bucket = work_bucket();
    let key = "it-dir/中文测试-测试文件.txt";
    let local = std::env::temp_dir().join("sssc-中文测试.txt");
    std::fs::write(&local, b"chinese content").unwrap();

    // 1. 上传（key 含中文）
    let up = ops::upload_file(&client, &bucket, key, &local, std::sync::Arc::new(|_, _| {}) as _).await;
    println!("上传: {}", if up.is_ok() { "OK" } else { "FAIL" });

    // 2a. 带 encoding-type=url 列举（应用当前行为）
    let l1 = ops::list_objects(&client, &bucket, "it-dir/", None).await.unwrap();
    let found1 = l1.objects.iter().find(|o| o.key.contains("中文") || o.key.contains("%"));
    println!("encoding-type=url 列举到的 key: {:?}", found1.map(|o| &o.key));

    // 2b. 不带 encoding-type 列举
    let raw = client.list_objects_v2().bucket(&bucket).delimiter("/").prefix("it-dir/").send().await.unwrap();
    let found2 = raw.contents().iter().find(|o| o.key().unwrap_or("").contains("中文") || o.key().unwrap_or("").contains("%"));
    println!("无 encoding-type 列举到的 key: {:?}", found2.map(|o| o.key().unwrap_or("")));

    // 3. 下载（key 用中文原文）
    let dl = std::env::temp_dir().join("sssc-dl-中文.txt");
    let down = ops::download_file(&client, &bucket, key, &dl, std::sync::Arc::new(|_, _| {}) as _).await;
    println!("下载(中文key): {}", if down.is_ok() { "OK" } else { "FAIL" });

    // 清理
    let _ = ops::delete_object(&client, &bucket, key).await;
    let _ = std::fs::remove_file(&local); let _ = std::fs::remove_file(&dl);
}

/// 上传进度回调验证：2MB 文件按 256KB 分块，应产生多次单调递增回调
#[tokio::test]
async fn diag_upload_progress() {
    let Some((p, sk)) = creds(sssc_lib::Signature::V4) else { return };
    let client = ops::build_client(&p, &sk).await;
    let bucket = work_bucket();
    let ts = ts();
    let key = format!("it-dir/prog-{}.bin", ts);
    let local = std::env::temp_dir().join(format!("sssc-prog-{}.bin", ts));
    let total = 2u64 * 1024 * 1024;
    std::fs::write(&local, vec![1u8; total as usize]).unwrap();

    let marks = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(u64, u64)>::new()));
    let m2 = marks.clone();
    ops::upload_file(&client, &bucket, &key, &local, std::sync::Arc::new(move |d, t| {
        m2.lock().unwrap().push((d, t));
    })).await.expect("上传失败");

    let marks = marks.lock().unwrap().clone();
    println!("进度回调次数: {}", marks.len());
    println!("序列: {:?}", marks);
    assert!(marks.len() >= 8, "进度回调不足（期望 ≥8 次 256KB 分块）");
    assert!(marks.windows(2).all(|w| w[0].0 <= w[1].0), "进度必须非递减");
    assert_eq!(*marks.last().unwrap(), (total, total), "最终进度必须是 100%");
    let _ = ops::delete_object(&client, &bucket, &key).await;
    let _ = std::fs::remove_file(&local);
    println!("进度验证 ✓");
}
