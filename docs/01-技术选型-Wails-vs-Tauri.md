# 技术选型：Wails v3 vs Tauri 2

> 文档版本：v0.1 · 2026-11-05
> 场景：跨平台（Win/macOS/Linux）S3 多集群管理桌面客户端，目标用户为研发人员，持有多个集群的 AK/SK 与临时凭证。

---

## 0. 结论先行

**推荐 Tauri 2.x（当前 v2.12）作为首选框架**，前端 React + TypeScript + Vite，后端 Rust + aws-sdk-rust。

**改选 Wails v3 的触发条件**（满足任意两条即应重评）：
1. 团队主力语言是 Go，且已有可复用的 Go 版 S3/运维代码；
2. 需要深度依赖 minio-go / aws-sdk-go-v2 生态中 Rust 侧缺乏的成熟能力；
3. 无法接受 Rust 的编译迭代速度与技能门槛。

> 时效说明：Wails v3.0.0 已于 2026-10-26 正式 stable（v3.1.0 于 11-01 发布），并非旧资料所称 alpha/beta；Tauri 当前为 v2.12（2026-09-26）。

---

## 1. 对比总表

| 维度 | Tauri 2.12 | Wails v3.0+ | 对本场景权重 |
| --- | --- | --- | --- |
| 后端语言/SDK | Rust；aws-sdk-rust 已 GA（2023-11），生态较薄 | Go；aws-sdk-go-v2 + minio-go/v7 极成熟 | ★★★ |
| **安全模型** | deny-by-default ACL/Capability、强制 CSP、Isolation、stronghold 加密存储、**独立安全审计（Radically Open Security, 2024-08）** | 绑定按服务暴露，无 ACL 等价物，密钥加密需自行实现，无审计 | ★★★（AK/SK 是最高敏感资产） |
| 包体 / 内存 | ~3–5MB / 20–80MB | ~8–12MB / 25–90MB | ★★ |
| 跨平台 | Win/macOS/Linux + **移动端稳定**；Linux 依赖 webkit2gtk(gtk3)，老发行版是痛点；v2.12 起放弃 Win7 | Win/macOS/Linux（amd64+arm64）；Linux 升 GTK4/WebKitGTK 6（更依赖新发行版）；移动端 experimental | ★★★ |
| 自动更新 | 官方 updater 插件（Ed25519 + latest.json，GH Releases/静态托管） | v3 全新 updater：可插拔 provider、Ed25519/ECDSA、**更新清单可直接托管在 S3**、原子交换 | ★★ |
| 桌面能力 | 多窗口/托盘/全局快捷键/拖放/深色模式/通知/单实例，30+ 官方插件 | v3 重写后基本对等（原生多窗口、托盘、菜单、拖放、主题） | ★★ |
| 前端自由度 | 任意 Vite 项目；TS 绑定需 ts-rs/specta 自行配 | 静态源码分析生成 TS 绑定，类型质量高，`wails3 generate bindings` | ★★ |
| 开发体验 | Rust 全量编译 3–7 分钟（中型项目） | Go 秒级编译；`wails3 dev` 热重载 | ★★ |
| 社区/维护 | ~106K stars，基金会 + CrabNebula 治理，2.x 稳定两年 | ~35K stars，核心维护者驱动（官方自述 issue/PR 积压），v3 stable 仅一周、生产案例少 | ★★★ |
| 签名分发 | MSI/NSIS/DMG/AppImage/deb/rpm/flatpak/snap + CrabNebula Cloud | 各平台安装器 + MSIX 指南齐备 | ★ |

## 2. 面向本场景的关键论证

### 2.1 安全是第一权重

本产品同时保存**多个集群的永久 AK/SK 与 STS 临时凭证**（含 prod 凭据），是研发机器上最敏感的资产之一。Tauri 2 的最小权限模型（fs/http/shell 能力显式声明、按窗口分配）、默认 CSP、可选 Isolation 模式，配合官方 stronghold 插件（加密存储，可桥接系统 Keychain）与第三方独立审计，构成完整的凭据安全链路。Wails v3 当前没有等价物，需要自研加密存储并自负其责——这在选型上是一个明确的减分项。

### 2.2 S3 SDK：Go 生态占优，但 Rust 已够用

- Go：aws-sdk-go-v2（AWS 官方一等公民）+ minio-go/v7（S3 兼容生态事实标准），自定义 endpoint、AssumeRole 多账号、兼容性探测等运维场景验证充分。
- Rust：aws-sdk-rust 已 GA，S3/STS 全覆盖，分页列举、MPU、进度流式回调均无障碍；劣势在生态规模、示例与社区案例。

结论：SDK 不足以推翻安全与维护风险的权衡；若最终选 Wails，则 SDK 从「够用」升级为「优势」。

### 2.3 多集群架构影响

多集群 + 传输引擎（见《02》§4）要求：并发多连接池、SIGV4 定制 transport、本地传输状态库、系统凭据库访问。两者都能实现；差异在于：
- Tauri：凭据库走 keyring/stronghold 插件链路成熟；
- Wails：zalando/go-keyring 亦成熟，但需自行设计加密与权限边界。

### 2.4 维护风险

Wails v3 stable 刚满一周（v2→v3 为彻底重写），生产案例少、巴士系数低；Tauri 2.x 已稳定两年且有基金会治理。研发工具的受众（研发人员）对「工具本身是否可信、是否持续维护」敏感，长期风险权重应高于编译速度的体验差异。

## 3. 推荐技术栈（Tauri 路线）

| 层 | 选型 | 说明 |
| --- | --- | --- |
| 框架 | Tauri 2.12 | capability ACL 收紧到最小；updater 用 Ed25519 + GitHub Releases |
| 前端 | React 18 + TypeScript + Vite + Tailwind(v4) | 设计 token 见《03》，用 CSS 变量实现深浅色 |
| 后端 | Rust + aws-sdk-rust | S3/STS；自定义 transport 处理 path-style、自签 CA、重试/限速 |
| 凭据 | keyring（系统凭据库）+ stronghold 可选 | AK/SK 不落明文盘 |
| 状态 | 前端 TanStack Query + Zustand；后端 SQLite（传输任务/分块指纹） | 断点续传的本地状态库 |
| 传输 | Rust tokio 并发 + MPU/Range 引擎 | 全局并发控制、SlowDown 降并发 |

## 4. 风险与缓解

| 风险 | 缓解 |
| --- | --- |
| Rust 编译慢影响迭代 | 前端 HMR 覆盖大部分 UI 迭代；CI 缓存 cargo target；core 与 command 层分离 |
| Linux WebKitGTK 碎片化 | CI 矩阵覆盖 Ubuntu 22.04/24.04 + Fedora；AppImage/deb 双格式发布 |
| aws-sdk-rust 生态案例少 | 传输引擎自建（本就要求深度定制 SigV4/重试）；关键路径补集成测试 |
| 团队无 Rust 经验 | 这是改选 Wails v3 的第一触发条件；S3 引擎若已存在于 Go，可评估 Tauri sidecar 挂 Go 二进制的折中（双语言复杂度高，非首选） |

## 5. 主要信息来源

- Wails v3.0.0 发布：https://v3.wails.io/blog/wails-v3-released/ · Updater：https://v3.wails.io/guides/updater/
- Tauri 2.12：https://tauri.app/blog/tauri-2-12/ · 安全：https://tauri.app/security/ · 插件：https://tauri.app/plugin/
- aws-sdk-rust GA 历史：https://docs.aws.amazon.com/sdk-for-rust/latest/dg/document-history.html
- 中文横评（体积/内存量级，仅采信与官方一致部分）：https://www.codef.cc/webview-desktop-framework-horizontal-review.html
