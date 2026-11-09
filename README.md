# SSSC · S3 多集群桌面客户端

面向研发人员的跨平台（Windows / macOS / Linux）S3 对象存储客户端，支持多集群（Profile）管理与切换，兼容 AWS S3 及 MinIO / Ceph RGW / SeaweedFS / 各云厂商 S3 兼容端点。

## 技术栈

Tauri 2 + React 18 + TypeScript + Vite；Rust 后端 + aws-sdk-rust。选型论证见 [docs/01](docs/01-技术选型-Wails-vs-Tauri.md)。

## 开发

```bash
pnpm install
pnpm tauri dev      # 桌面应用（需 Rust 工具链）
pnpm dev            # 仅浏览器模式（mock 数据，无需 Rust）
pnpm build          # 前端类型检查 + 构建
cd src-tauri && cargo check   # Rust 侧类型检查
```

## 测试

```bash
cd src-tauri && cargo test --test integration -- --nocapture --test-threads=1
# 指向其他集群：SSSC_ENDPOINT=... SSSC_AK=... SSSC_SK=... cargo test --test integration
# 服务端怪癖调查（SeaweedFS）：cargo test --test diag -- --nocapture
```

测试计划与用例结果见 [docs/04-测试计划.md](docs/04-测试计划.md)；注意 SeaweedFS 的 S3 网关与 Filer Web UI 是不同端口（如 8333 vs 8888）。

## 项目结构

```
├── docs/                     调研与设计文档
│   ├── 00-产品概述与多集群需求.md
│   ├── 01-技术选型-Wails-vs-Tauri.md
│   ├── 02-S3能力清单与功能需求.md      # 含 P0/P1/P2 与架构约束
│   └── 03-设计规范与交互设计.md        # DeepSeek 风格设计 token
├── mockup/index.html         高保真交互原型（浏览器直接打开）
├── src/                      前端（React）
│   ├── styles/tokens.css     设计 token（与 docs/03 对应）
│   ├── lib/backend.ts        Tauri/浏览器 双模式适配层
│   ├── types.ts              前后端共享领域模型
│   └── components/           侧栏/表格/命令面板/详情/传输条
└── src-tauri/                Rust 后端
    ├── src/lib.rs            Profile 存储 + S3 命令（list/presign/delete）
    └── tauri.conf.json       最小权限 capability（core:default）
```

## 已实现（v0.1）

- 多集群 Profile：增删、连接测试（ListBuckets 探活 + 延迟）、**断开连接 / 重新连接**（本地状态，重连重新探活）、标识色、endpoint/region/path-style/**自签开关（insecure_tls：rustls 跳过证书校验）**；**密钥持久化（secrets.json 0600 原子写入），重启不丢**
- 桶：列表/创建/删除（空桶校验 + 二次确认）
- 对象浏览（ListObjectsV2 + delimiter 目录树 + encoding-type=url + **continuation token 分页"加载更多"**）、删除、SigV4 预签名（1h/24h/7d，V4/V2 兼容）
- **对象多选**：复选框批量选择（含文件夹），批量删除；**文件夹递归删除**（ListObjectsV2 列举 + DeleteObjects 批量，每批 1000 个）
- 上传/下载（系统文件对话框 + 传输条实时进度事件流，>5GB 单请求给出明确错误）；⌘K 命令面板、Ctrl+1..9 切集群
- 错误三元组呈现（Code/Message/RequestId，服务端错误码解析）、深浅色模式（记忆 + 跟随系统）、完整设计 token 落地、键盘可达性（对话框 Esc/焦点管理、列表 role/tabIndex）

## Roadmap（见 docs/02 §3）

- **v0.2**：密钥迁移系统凭据库（keyring）、传输引擎（MPU + Range 断点续传 + SQLite 状态库 + 进度事件流 + SlowDown 降并发）
- **v0.3**：桶配置管理（Versioning/Lifecycle/CORS/Policy 编辑器）、能力探测降级
- **v0.4**：跨集群迁移、MPU 残留巡检、CLI 命令导出
