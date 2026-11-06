/**
 * 后端适配层：Tauri 环境走 invoke；纯浏览器（vite dev / 原型演示）走 mock。
 * 所有 UI 只依赖此模块，不直接 import @tauri-apps/api。
 */
import type {
  Bucket,
  ConnectionStatus,
  ListResult,
  Profile,
  S3Error,
  S3Object,
} from "../types";

const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<T>(cmd, args);
  }
  return mockInvoke<T>(cmd, args);
}

/* ---------------- Mock（浏览器模式） ---------------- */

const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

let mockProfiles: Profile[] = [
  {
    id: "p1", name: "prod-minio", endpoint: "https://s3.example.com:9000",
    region: "us-east-1", accessKey: "AKIA_prod", secretMasked: "••••3f9a",
    style: "path", insecureTls: true, color: "red", enabled: true,
  },
  {
    id: "p2", name: "dev-ceph", endpoint: "https://rgw.dev.internal",
    region: "us-east-1", accessKey: "AKIA_dev", secretMasked: "••••77bc",
    style: "path", insecureTls: false, color: "green", enabled: true,
  },
];

const pad = (n: number) => String(n).padStart(2, "0");
const mockTime = (day: number, h = 10, m = 22) =>
  `2026-11-${pad(day)}T${pad(h)}:${pad(m)}:00Z`;

async function mockInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  await delay(220);
  switch (cmd) {
    case "list_profiles":
      return mockProfiles as T;
    case "save_profile":
      mockProfiles = [...mockProfiles, args!.profile as Profile];
      return args!.profile as T;
    case "delete_profile":
      mockProfiles = mockProfiles.filter((p) => p.id !== args!.id);
      return undefined as T;
    case "test_connection":
      return { ok: true, latencyMs: 12, bucketCount: 27, error: null } as T;
    case "list_buckets":
      return [
        { name: "artifacts", createdAt: mockTime(1), region: "us-east-1" },
        { name: "logs", createdAt: mockTime(2), region: "us-east-1" },
        { name: "backup", createdAt: mockTime(3), region: "us-east-1" },
      ] as T;
    case "list_objects": {
      const prefix = (args!.prefix as string) ?? "";
      if (prefix === "") {
        return {
          objects: [],
          prefixes: [{ prefix: "artifacts/" }, { prefix: "logs/" }, { prefix: "backup/" }],
          nextToken: null, keyCount: 3,
        } as T;
      }
      if (args!.continuationToken) {
        // 第二页（mock 分页演示）
        const objs: S3Object[] = [
          { key: prefix + "build-20261103.log", size: 1_988_112, lastModified: mockTime(3, 21, 44), etag: "\"aabbccdd11223344\"", storageClass: "STANDARD" },
          { key: prefix + "changelog.md", size: 18_442, lastModified: mockTime(2, 9, 15), etag: "\"eeff001122334455\"", storageClass: "STANDARD" },
        ];
        return { objects: objs, prefixes: [], nextToken: null, keyCount: 2 } as T;
      }
      const objs: S3Object[] = [
        { key: prefix + "sssc-server-v2.4.1-darwin-arm64.tar.gz", size: 134_348_800, lastModified: mockTime(4, 18, 3), etag: "\"c4ca4238a0b9238\"", storageClass: "STANDARD" },
        { key: prefix + "sssc-server-v2.4.1-linux-amd64.tar.gz", size: 137_530_880, lastModified: mockTime(4, 18, 5), etag: "\"9c1b3e77a02f11d\"", storageClass: "STANDARD" },
        { key: prefix + "build-20261104.log", size: 2_202_009, lastModified: mockTime(4, 3, 0), etag: "\"77a02f19b3c4d5\"", storageClass: "STANDARD_IA" },
      ];
      return { objects: objs, prefixes: [], nextToken: "mock-token-1", keyCount: 3 } as T;
    }
    case "presign_get":
      return `https://s3.example.com:9000/${args!.bucket}/${args!.key}?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Expires=${args!.expiresSec}&X-Amz-Signature=mock` as T;
    case "delete_object":
      return undefined as T;
    default:
      throw { code: "NotImplemented", message: `mock 不支持 ${cmd}`, requestId: null, httpStatus: null } as S3Error;
  }
}

/* ---------------- 对外 API ---------------- */

export const api = {
  isTauri,
  listProfiles: () => invoke<Profile[]>("list_profiles"),
  saveProfile: (profile: Profile) => invoke<Profile>("save_profile", { profile }),
  deleteProfile: (id: string) => invoke<void>("delete_profile", { id }),
  testConnection: (profile: Profile) => invoke<ConnectionStatus>("test_connection", { profile }),
  listBuckets: (profileId: string) => invoke<Bucket[]>("list_buckets", { profileId }),
  /**
   * ListObjectsV2。token 为旧版后端遗留参数（保持原 invoke 参数不变）；
   * continuationToken 为可选分页游标：后端新增分页支持时返回 ListResult.nextToken，
   * 前端把它作为 continuationToken 回传即可翻页（未知参数后端兼容忽略）。
   */
  listObjects: (profileId: string, bucket: string, prefix: string, token: string | null, continuationToken?: string) =>
    invoke<ListResult>("list_objects", continuationToken
      ? { profileId, bucket, prefix, token, continuationToken }
      : { profileId, bucket, prefix, token }),
  presignGet: (profileId: string, bucket: string, key: string, expiresSec: number) =>
    invoke<string>("presign_get", { profileId, bucket, key, expiresSec }),
  deleteObject: (profileId: string, bucket: string, key: string) =>
    invoke<void>("delete_object", { profileId, bucket, key }),
  /** 递归删除"文件夹"（前缀下全部对象），批量 DeleteObjects */
  deleteFolder: (profileId: string, bucket: string, prefix: string) =>
    invoke<{ deletedCount: number; errorCount: number }>("delete_folder", { profileId, bucket, prefix }),
  /**
   * 批量删除（后端一次完成：文件走 DeleteObjects 每批 1000，文件夹递归展开后同批删）。
   * keys = 对象 key；prefixes = 文件夹（以 / 结尾）。进度经 onBatchProgress 事件上报。
   */
  deleteBatch: (profileId: string, bucket: string, keys: string[], prefixes: string[], batchId: string) =>
    invoke<{ deletedCount: number; errorCount: number; failures: { key: string; code: string; message: string }[] }>("delete_batch", {
      profileId, bucket, keys, prefixes, batchId,
    }),
  createBucket: (profileId: string, bucket: string) =>
    invoke<void>("create_bucket", { profileId, bucket }),
  createFolder: (profileId: string, bucket: string, key: string) =>
    invoke<void>("create_folder", { profileId, bucket, key }),
  deleteBucket: (profileId: string, bucket: string) =>
    invoke<void>("delete_bucket", { profileId, bucket }),
  uploadFile: (profileId: string, bucket: string, key: string, localPath: string) =>
    invoke<string>("upload_file", { profileId, bucket, key, localPath }),
  downloadFile: (profileId: string, bucket: string, key: string, localPath: string) =>
    invoke<string>("download_file", { profileId, bucket, key, localPath }),
};

/** 文件选择/保存对话框（Tauri 用系统对话框；浏览器回退 prompt） */
export async function pickFiles(): Promise<string[] | null> {
  if (isTauri) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const r = await open({ multiple: true });
    if (!r) return null;
    return Array.isArray(r) ? r : [r];
  }
  const p = prompt("浏览器 mock 模式：输入本地文件绝对路径（多个用逗号分隔）");
  return p ? p.split(",").map((s) => s.trim()).filter(Boolean) : null;
}

export async function pickSavePath(defaultName: string): Promise<string | null> {
  if (isTauri) {
    const { save } = await import("@tauri-apps/plugin-dialog");
    return await save({ defaultPath: defaultName });
  }
  return prompt("浏览器 mock 模式：输入保存路径", defaultName);
}

/** 传输任务事件（Rust 端 upload_file/download_file 发出） */
export interface TransferEvent {
  taskId: string;
  kind: "upload" | "download";
  cluster: string;
  bucket: string;
  key: string;
  localPath: string;
  done: number;
  total: number;
  state: "running" | "done" | "failed";
  error: { code: string; message: string } | null;
}

export async function onTransferEvent(cb: (ev: TransferEvent) => void): Promise<() => void> {
  if (!isTauri) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  const un = await listen<TransferEvent>("transfer://event", (e) => cb(e.payload));
  return un;
}

export interface BatchProgress { batchId: string; done: number; total: number; phase: "listing" | "deleting" | "done"; }

/** 批量删除进度事件（后端每完成一批 DeleteObjects 上报一次） */
export async function onBatchProgress(cb: (ev: BatchProgress) => void): Promise<() => void> {
  if (!isTauri) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  const un = await listen<BatchProgress>("delete://progress", (e) => cb(e.payload));
  return un;
}

export function formatSize(n: number | null): string {
  if (n == null) return "—";
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024, i = 0;
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i] ?? "KB"}`;
}

export function formatTime(iso: string | null): string {
  if (!iso) return "";
  // 兼容后端可能返回的 epoch 秒字符串（旧格式）
  const d = /^\d{9,}$/.test(iso) ? new Date(Number(iso) * 1000) : new Date(iso);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/**
 * 安全解码：encoding-type=url 时后端返回的 key/prefix 是百分号编码，
 * 解码失败（非法序列等）回退原文。仅用于展示；导航仍使用原始值。
 */
export function safeDecode(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}
