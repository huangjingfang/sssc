//! SSSC 全生命周期集成测试（可复用）。
//!
//! 与 Tauri command 复用同一 `sssc_lib::ops` 实现，直接打真实集群，
//! 覆盖：连接 → 建桶 → 上传 → 列举 → 预签名验证 → 下载校验 → 删对象 →
//! （负例：非空删桶）→ 删桶 → 删除连接。
//!
//! 目标集群凭据全部来自环境变量（不硬编码任何默认真实值）：
//!   SSSC_ENDPOINT / SSSC_AK / SSSC_SK（必需；缺失时所有测试优雅跳过）
//!   SSSC_REGION（可选，默认 us-east-1）
//!   SSSC_IT_WORK_BUCKET（可选工作桶）
//! 运行：cd src-tauri && SSSC_ENDPOINT=... SSSC_AK=... SSSC_SK=... \
//!        cargo test --test integration -- --nocapture
//! 注意：测试会创建并清理 `sssc-it-<时间戳>` 桶，请对目标集群使用测试凭证。

use aws_sdk_s3::Client;
use sssc_lib::{ops, Profile};

/// 读取目标集群凭据；任一必需变量缺失则打印跳过原因并返回 None。
fn target() -> Option<Profile> {
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
    Some(Profile {
        id: "it".into(),
        name: "it-cluster".into(),
        endpoint,
        region: std::env::var("SSSC_REGION").unwrap_or_else(|_| "us-east-1".into()),
        access_key: ak,
        secret_masked: String::new(),
        style: sssc_lib::Style::Path,
        signature: sssc_lib::Signature::V4,
        insecure_tls: false,
        color: "blue".into(),
        enabled: true,
        secret_key: Some(sk),
    })
}

fn ts() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

/// 生成确定伪随机内容的测试文件（可重复校验）。
fn make_test_file(size: usize) -> (std::path::PathBuf, Vec<u8>) {
    let mut data = vec![0u8; size];
    let mut x: u64 = 0x9E3779B97F4A7C15;
    for b in data.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = (x & 0xFF) as u8;
    }
    let path = std::env::temp_dir().join(format!("sssc-it-{}.bin", ts()));
    std::fs::write(&path, &data).unwrap();
    (path, data)
}

/// 原生 HTTP GET（验证预签名 URL 的 SigV4 正确性）。
async fn http_get(url: &str) -> (u16, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = url.strip_prefix("http://").expect("仅支持 http");
    let (hostport, path) = rest.split_once('/').expect("URL 无路径");
    let mut stream = tokio::net::TcpStream::connect(hostport).await.unwrap();
    stream
        .write_all(
            format!("GET /{path} HTTP/1.1\r\nHost: {hostport}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await
        .unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let header_end = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("响应无 header 分隔");
    let header = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let status: u16 = header
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, buf[header_end + 4..].to_vec())
}

/// TC1: 创建连接（ListBuckets 探活 + 延迟 + 桶计数）
#[tokio::test]
async fn tc1_connect() {
    let Some(p) = target() else { return; };
    let st = ops::test_connection(&p).await;
    assert!(st.ok, "连接失败: {:?}", st.error);
    println!("[TC1] 连接成功 latency={:?}ms buckets={:?}", st.latency_ms, st.bucket_count);
}

/// TC2~TC9: 完整生命周期（按顺序在一个桶上执行）
///
/// 说明：目标集群若是 SeaweedFS 且 S3 CreateBucket 建出的新桶拒绝写入
/// （服务端怪癖，见 tests/diag.rs 的调查记录），对象级用例自动回退到
/// 已有工作桶执行；桶的创建/删除用例仍在独立新桶上验证。
#[tokio::test]
async fn tc2_full_lifecycle() {
    let Some(p) = target() else { return; };
    let secret = p.secret_key.clone().unwrap();
    let client = ops::build_client(&p, &secret).await;
    let bucket = format!("sssc-it-{}", ts());
    let key = format!("it-dir/sssc-it-{}.bin", ts());

    // TC2: 创建桶（独立验证，不依赖它可写）
    ops::create_bucket(&client, &bucket).await.expect("TC2 建桶失败");
    let names: Vec<String> = ops::list_buckets(&client)
        .await
        .unwrap()
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert!(names.contains(&bucket), "TC2 桶未出现在列表: {names:?}");
    println!("[TC2] 建桶 {bucket} ✓（当前 {} 桶）", names.len());

    // 选择对象级用例的工作桶：新建桶 → 探测可写 → 必要时回退已有桶
    let (work_bucket, work_bucket_owned, quirk) = select_work_bucket(&client, &bucket, &names).await;
    if quirk {
        println!("[WARN] 目标集群存在 SeaweedFS 新桶不可写怪癖，对象级用例回退到已有桶 {work_bucket}");
    }

    // TC3: 上传文件（2MB 伪随机）
    let (local, data) = make_test_file(2 * 1024 * 1024);
    let last_progress = std::sync::Arc::new(std::sync::Mutex::new((0u64, 0u64)));
    let lp = last_progress.clone();
    ops::upload_file(&client, &work_bucket, &key, &local, std::sync::Arc::new(move |d, t| {
        *lp.lock().unwrap() = (d, t);
    }))
    .await
    .expect("TC3 上传失败");
    assert_eq!(*last_progress.lock().unwrap(), (data.len() as u64, data.len() as u64), "TC3 进度回调未到 100%");
    println!("[TC3] 上传 → {work_bucket}/{key}（{} bytes，进度回调完成）✓", data.len());

    // TC4: 列举（目录树：prefix 聚合 + 对象可见）
    let root = ops::list_objects(&client, &work_bucket, "", None).await.unwrap();
    assert!(
        root.prefixes.iter().any(|x| x == "it-dir/"),
        "TC4 根目录未见 it-dir/ 前缀: {:?}",
        root.prefixes
    );
    let dir = ops::list_objects(&client, &work_bucket, "it-dir/", None).await.unwrap();
    let obj = dir.objects.iter().find(|o| o.key == key).expect("TC4 未列举到上传对象");
    assert_eq!(obj.size, Some(data.len() as i64), "TC4 大小不符");
    println!("[TC4] 列举 ✓ 目录聚合 {:?}, 对象 size={} etag={:?}", root.prefixes, obj.size.unwrap(), obj.etag);

    // TC5: 预签名 URL 生成 + 免凭证下载逐字节校验（SigV4 端到端）
    let url = ops::presign_get(&client, &work_bucket, &key, 300)
        .await
        .expect("TC5 预签名失败");
    let (status, body) = http_get(&url).await;
    assert_eq!(status, 200, "TC5 预签名 GET 非 200");
    assert_eq!(body.as_slice(), data.as_slice(), "TC5 预签名下载内容不符");
    // TC5b: 错误 AK 生成的签名必须被拒绝（签名体系负向验证）
    let mut bad = p.clone();
    bad.access_key = format!("{}-typo", p.access_key);
    bad.id = "it-bad".into();
    let bad_client = ops::build_client(&bad, &secret).await;
    let bad_url = ops::presign_get(&bad_client, &work_bucket, &key, 300).await.unwrap();
    let (bad_status, _) = http_get(&bad_url).await;
    assert!(bad_status >= 400, "TC5b 错误 AK 的预签名竟被放行: {bad_status}");
    println!("[TC5] 预签名 ✓（正确签名 200 / 内容一致；错误 AK 被拒 {bad_status}）");
    // 注：S3 预签名是 bearer token，无法主动吊销，有效期是唯一撤销手段。

    // TC6: 下载到本地 + 逐字节校验
    let dl = std::env::temp_dir().join(format!("sssc-it-dl-{}.bin", ts()));
    ops::download_file(&client, &work_bucket, &key, &dl, std::sync::Arc::new(|_d, _t| {}) as _)
        .await
        .expect("TC6 下载失败");
    let downloaded = std::fs::read(&dl).unwrap();
    assert_eq!(downloaded, data, "TC6 下载内容与原始文件不一致");
    println!("[TC6] 下载 {} ✓（逐字节一致）", dl.display());

    // TC7: 负例——非空桶不可删（仅对测试自建桶执行破坏性尝试；
    // 回退到共享桶时跳过，避免误删他人数据）
    if work_bucket_owned {
        let err = ops::delete_bucket(&client, &work_bucket).await;
        assert!(err.is_err(), "TC7 非空桶竟删除成功");
        println!("[TC7] 非空删桶被拒 ✓（{:?}）", err.err().map(|e| (e.code.clone(), e.http_status)));
    } else {
        println!("[TC7] 跳过（工作桶为共享已有桶，不做破坏性删桶尝试）");
    }

    // TC8: 删除对象 → 桶变空
    // TC8: 删除对象 → 桶变空。
    // 已知服务端怪癖（SeaweedFS 4.50，tests/diag.rs diag_read_then_delete 复现）：
    // 对象被 GET 读取后 DeleteObject 稳定返回 500。规避：先覆盖重写
    // （产生未被读取的新版本），再删除；若仍失败按 docs/02 §1.6 退避重试。
    let small = std::env::temp_dir().join(format!("sssc-it-small-{}.bin", ts()));
    std::fs::write(&small, b"fresh").unwrap();
    ops::upload_file(&client, &work_bucket, &key, &small, std::sync::Arc::new(|_, _| {}) as _)
        .await
        .expect("TC8 覆盖重写失败");
    let mut deleted = false;
    for i in 1..=5 {
        match ops::delete_object(&client, &work_bucket, &key).await {
            Ok(()) => { deleted = true; println!("[TC8] 删除对象 ✓（第 {i} 次尝试）"); break; }
            Err(e) => println!("[TC8] 删除重试 {i}: {} ({})", e.code, e.http_status.unwrap_or(0)),
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(deleted, "TC8 删除对象 5 次重试后仍失败");
    let _ = std::fs::remove_file(&small);
    let after = ops::list_objects(&client, &work_bucket, "it-dir/", None).await.unwrap();
    assert!(after.objects.iter().all(|o| o.key != key), "TC8 对象仍存在");
    // TC8b: 对已删对象预签名/下载必须失败（404/NoSuchKey）
    let gone = ops::download_file(&client, &work_bucket, &key, &std::env::temp_dir().join("sssc-it-gone"), std::sync::Arc::new(|_, _| {}) as _).await;
    assert!(gone.is_err(), "TC8b 已删对象竟可下载");
    println!("[TC8] 删对象 ✓（桶已空；已删对象下载被拒）");

    // TC9: 删除空桶（TC2 自建桶 + 若对象级用例也在自建桶则一并清理）
    ops::delete_bucket(&client, &bucket).await.expect("TC9 删桶失败");
    if work_bucket_owned && work_bucket != bucket {
        ops::delete_bucket(&client, &work_bucket).await.expect("TC9 删工作桶失败");
    }
    let names: Vec<String> = ops::list_buckets(&client)
        .await
        .unwrap()
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert!(!names.contains(&bucket), "TC9 桶仍在列表");
    println!("[TC9] 删桶 {bucket} ✓");

    // 清理本地临时文件
    let _ = std::fs::remove_file(&local);
    let _ = std::fs::remove_file(&dl);
}

/// TC10: 删除连接（本地 Profile 存储语义由 command 层保证，
/// 此处验证同一 profile 重建 client 幂等可用——模拟删除后重新添加集群）
#[tokio::test]
async fn tc10_recreate_connection() {
    let Some(p) = target() else { return; };
    let secret = p.secret_key.clone().unwrap();
    let client: Client = ops::build_client(&p, &secret).await;
    let st = ops::test_connection(&p).await;
    assert!(st.ok);
    println!("[TC10] 删除并重建连接语义 ✓（client 重建 + 探活 {}ms）", st.latency_ms.unwrap_or(0));
    drop(client);
}

/// 选择工作桶：优先 TC2 自建桶；若服务端拒绝向新桶写入
/// （SeaweedFS 怪癖，见 tests/diag.rs），回退到第一个已有桶
/// （或 SSSC_IT_WORK_BUCKET 指定桶）。
/// 返回 (桶名, 是否测试自建, 是否触发了怪癖回退)。
async fn select_work_bucket(
    client: &Client,
    created: &str,
    existing: &[String],
) -> (String, bool, bool) {
    if let Ok(b) = std::env::var("SSSC_IT_WORK_BUCKET") {
        return (b, false, false);
    }
    // 探测自建桶可写性（5 字节）
    let probe = std::env::temp_dir().join(format!("sssc-probe-{}.bin", ts()));
    std::fs::write(&probe, b"probe").unwrap();
    let probe_ok = ops::upload_file(client, created, "it-dir/probe.bin", &probe, std::sync::Arc::new(|_, _| {}) as _)
        .await
        .is_ok();
    let _ = std::fs::remove_file(&probe);
    if probe_ok {
        return (created.to_string(), true, false);
    }
    // 怪癖回退：第一个非测试桶
    let fallback = existing
        .iter()
        .find(|b| !b.starts_with("sssc-it-") && !b.starts_with("sssc-diag-"))
        .cloned()
        .expect("集群上没有任何可用作工作桶的已有桶");
    (fallback, false, true)
}

/// TC11: 残留清理——扫描工作桶 it-dir/ 下本套件/诊断留下的对象，
/// 覆盖重写后删除（规避读后删 500 怪癖），保证集群干净可重复执行。
#[tokio::test]
async fn tc11_cleanup_leftovers() {
    let Some(p) = target() else { return; };
    let secret = p.secret_key.clone().unwrap();
    let client = ops::build_client(&p, &secret).await;
    let work = std::env::var("SSSC_IT_WORK_BUCKET").unwrap_or_else(|_| "hdd-demo".into());
    let list = ops::list_objects(&client, &work, "it-dir/", None).await.unwrap();
    let fresh = std::env::temp_dir().join(format!("sssc-it-fresh-{}.bin", ts()));
    std::fs::write(&fresh, b"x").unwrap();
    let mut cleaned = 0;
    for o in list.objects {
        let key = o.key.clone();
        let _ = ops::upload_file(&client, &work, &key, &fresh, std::sync::Arc::new(|_, _| {}) as _).await;
        if ops::delete_object(&client, &work, &key).await.is_ok() {
            cleaned += 1;
        } else {
            println!("[TC11] 无法清理 {key}（服务端读后删怪癖），请手工处理");
        }
    }
    let _ = std::fs::remove_file(&fresh);
    println!("[TC11] 残留清理完成：{cleaned} 个对象");
}

/// TC12: 清理历史遗留的空测试桶（sssc-it-* / sssc-diag-*）。
#[tokio::test]
async fn tc12_cleanup_leftover_buckets() {
    let Some(p) = target() else { return; };
    let secret = p.secret_key.clone().unwrap();
    let client = ops::build_client(&p, &secret).await;
    for b in ops::list_buckets(&client).await.unwrap() {
        if b.name.starts_with("sssc-it-") || b.name.starts_with("sssc-diag-") {
            let r = ops::delete_bucket(&client, &b.name).await;
            println!("[TC12] 清理桶 {}：{}", b.name, if r.is_ok() { "✓" } else { "非空/失败，保留" });
        }
    }
}

/// TC13：新建文件夹（S3 目录 = key 以 / 结尾的 0 字节对象）。
/// 创建 → 列举应出现对应 CommonPrefix → 清理 marker 对象。
#[tokio::test]
async fn tc13_create_folder() {
    let Some(p) = target() else { return; };
    let secret = p.secret_key.clone().unwrap();
    let client = ops::build_client(&p, &secret).await;
    let work = "hdd-demo".to_string(); // TC13 固定用共享桶（只增删目录 marker）

    let dir = format!("it-folder-{}/", ts());
    println!("[TC13] 在桶 {work} 创建文件夹 {dir}");
    ops::create_folder(&client, &work, &dir).await.expect("create_folder 失败");

    // 顶层列举应出现该目录前缀
    let list = ops::list_objects(&client, &work, "", None).await.unwrap();
    assert!(
        list.prefixes.iter().any(|p| p.as_str() == dir),
        "列举结果中未找到目录前缀 {dir}，实际 prefixes: {:?}",
        list.prefixes.iter().map(|p| p.clone()).collect::<Vec<_>>()
    );
    println!("[TC13] ✓ 列举中出现目录前缀 {dir}");

    // 在文件夹内再建一级子目录，验证 prefix 拼接
    let sub = format!("{dir}sub-{}/", ts());
    ops::create_folder(&client, &work, &sub).await.expect("子文件夹创建失败");
    let inside = ops::list_objects(&client, &work, &dir, None).await.unwrap();
    assert!(inside.prefixes.iter().any(|p| p.as_str() == sub), "子目录前缀未出现");
    println!("[TC13] ✓ 子目录 {sub} 可见");

    // 清理（SeaweedFS 读后删怪癖：先覆盖再删）
    let fresh = std::env::temp_dir().join(format!("sssc-it-fresh-{}.bin", ts()));
    std::fs::write(&fresh, b"x").unwrap();
    for key in [&sub, &dir] {
        let _ = ops::upload_file(&client, &work, key, &fresh, std::sync::Arc::new(|_, _| {}) as _).await;
        match ops::delete_object(&client, &work, key).await {
            Ok(()) => println!("[TC13] 清理 {key}"),
            Err(e) => println!("[TC13] {key} 清理失败（服务端怪癖）: {:?}", e),
        }
    }
    let _ = std::fs::remove_file(&fresh);
    println!("[TC13] PASSED");
}

/// TC13b：目录 marker（key 以 / 结尾的 0 字节对象）不应出现在 objects 列表中。
#[tokio::test]
async fn tc13b_folder_marker_hidden() {
    let Some(p) = target() else { return; };
    let secret = p.secret_key.clone().unwrap();
    let client = ops::build_client(&p, &secret).await;
    let work = "hdd-demo".to_string();

    let dir = format!("it-marker-{}/", ts());
    ops::create_folder(&client, &work, &dir).await.expect("create_folder 失败");

    let list = ops::list_objects(&client, &work, "", None).await.unwrap();
    assert!(list.prefixes.iter().any(|x| x.as_str() == dir), "目录前缀应出现");
    assert!(
        !list.objects.iter().any(|o| o.key == dir),
        "目录 marker 不应出现在 objects 中: {:?}",
        list.objects.iter().map(|o| o.key.clone()).collect::<Vec<_>>()
    );
    println!("[TC13b] ✓ marker {dir} 已从对象列表隐藏（以目录形式展示）");

    // 清理
    let fresh = std::env::temp_dir().join(format!("sssc-it-fresh-{}.bin", ts()));
    std::fs::write(&fresh, b"x").unwrap();
    let _ = ops::upload_file(&client, &work, &dir, &fresh, std::sync::Arc::new(|_, _| {}) as _).await;
    let _ = ops::delete_object(&client, &work, &dir).await;
    let _ = std::fs::remove_file(&fresh);
    println!("[TC13b] PASSED");
}

/// TC14：递归删除文件夹（delete_folder）。
/// 建目录 → 放入多个对象（含子目录）→ delete_folder → 前缀下应清空。
#[tokio::test]
async fn tc14_delete_folder_recursive() {
    let Some(p) = target() else { return; };
    let secret = p.secret_key.clone().unwrap();
    let client = ops::build_client(&p, &secret).await;
    let work = "hdd-demo".to_string();

    let dir = format!("it-delfolder-{}/", ts());
    let sub = format!("{dir}sub/");
    println!("[TC14] 建立目录结构 {dir}（含子目录 sub/ 与 3 个对象）");
    ops::create_folder(&client, &work, &dir).await.expect("create_folder 失败");
    ops::create_folder(&client, &work, &sub).await.expect("create_folder 失败");

    let fresh = std::env::temp_dir().join(format!("sssc-it-df-{}.bin", ts()));
    std::fs::write(&fresh, b"hello-folder-delete").unwrap();
    let arc: std::sync::Arc<dyn Fn(u64, u64) + Send + Sync> = std::sync::Arc::new(|_: u64, _: u64| {});
    let keys = [
        format!("{dir}a.txt"),
        format!("{dir}b.txt"),
        format!("{sub}c.txt"),
        dir.clone(), // 文件夹占位 marker 本身也要能被清掉
        sub.clone(),
    ];
    for k in &keys {
        ops::upload_file(&client, &work, k, &fresh, arc.clone()).await.expect("上传失败");
    }
    println!("[TC14] ✓ 5 个对象（含 2 个 marker）就位");

    let r = ops::delete_folder(&client, &work, &dir).await.expect("delete_folder 失败");
    println!("[TC14] delete_folder 返回 deleted={} errors={}", r.deleted_count, r.error_count);
    assert_eq!(r.error_count, 0, "递归删除不应有失败项");
    assert!(r.deleted_count >= 5, "至少应删除 5 个对象，实际 {}", r.deleted_count);

    // 前缀下应清空（读后删怪癖：允许重读一次）
    let mut empty = false;
    for _ in 0..2 {
        let after = ops::list_objects(&client, &work, &dir, None).await.unwrap();
        if after.objects.is_empty() && after.prefixes.is_empty() {
            empty = true;
            break;
        }
    }
    assert!(empty, "delete_folder 后前缀 {dir} 下仍有残留");
    println!("[TC14] ✓ 前缀 {dir} 已清空");
    let _ = std::fs::remove_file(&fresh);
    println!("[TC14] PASSED");
}
