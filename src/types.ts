/** 前后端共享领域模型（与 src-tauri/src/lib.rs 中 serde 结构一一对应） */

export type AddressingStyle = "path" | "virtual";

export interface Profile {
  id: string;
  name: string;
  endpoint: string;
  region: string;
  accessKey: string;
  /** 仅展示脱敏形式；真实密钥存后端（v0.2 迁移系统凭据库） */
  secretMasked: string;
  /** 仅在保存请求中一次性携带，后端永不回传 */
  secretKey?: string;
  style: AddressingStyle;
  /** 签名版本：v4 默认；v2 用于 nginx 反代 RGW 等旧网关兼容 */
  signature?: "v4" | "v2";
  /** 允许自签证书（研发私有部署） */
  insecureTls: boolean;
  /** 集群标识色: red|green|blue|purple|amber */
  color: string;
  enabled: boolean;
}

export interface ConnectionStatus {
  ok: boolean;
  latencyMs: number | null;
  bucketCount: number | null;
  error: S3Error | null;
}

export interface Bucket {
  name: string;
  createdAt: string | null;
  region: string | null;
}

export interface S3Object {
  key: string;
  size: number | null;
  lastModified: string | null;
  etag: string | null;
  storageClass: string | null;
}

export interface ListResult {
  objects: S3Object[];
  /** 目录前缀（S3 CommonPrefix），与后端 Vec<String> 序列化一致 */
  prefixes: string[];
  nextToken: string | null;
  keyCount: number;
}

export interface S3Error {
  code: string;
  message: string;
  requestId: string | null;
  httpStatus: number | null;
}
