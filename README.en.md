# SSSC · Multi-Cluster S3 Desktop Client

English | [简体中文](README.md)

A cross-platform (Windows / macOS / Linux) S3 object storage client built for developers, with multi-cluster (Profile) management and switching. Compatible with AWS S3 and S3-compatible endpoints such as MinIO, Ceph RGW, SeaweedFS, and major cloud providers.

## Tech Stack

Tauri 2 + React 18 + TypeScript + Vite; Rust backend + aws-sdk-rust. See the design notes in [docs/](docs/) (Chinese).

## Development

```bash
pnpm install
pnpm tauri dev      # Desktop app (requires Rust toolchain)
pnpm dev            # Browser-only mode (mock data, no Rust needed)
pnpm build          # Frontend type-check + build
cd src-tauri && cargo check   # Rust type-check
```

## Testing

```bash
cd src-tauri && cargo test --test integration -- --nocapture --test-threads=1
# Target another cluster: SSSC_ENDPOINT=... SSSC_AK=... SSSC_SK=... cargo test --test integration
# Server quirk investigation (SeaweedFS): cargo test --test diag -- --nocapture
```

Note: SeaweedFS exposes the S3 gateway and the Filer Web UI on different ports (e.g. 8333 vs 8888).

## Project Structure

```
├── docs/                     Research & design notes (Chinese)
├── mockup/index.html         High-fidelity interactive prototype (open in browser)
├── src/                      Frontend (React)
│   ├── styles/tokens.css     Design tokens
│   ├── lib/backend.ts        Tauri/browser dual-mode adapter layer
│   ├── types.ts              Shared domain model (frontend + backend)
│   └── components/           Sidebar / table / command palette / detail panel / transfer bar
└── src-tauri/                Rust backend
    ├── src/lib.rs            Profile storage + S3 commands (list/presign/delete/upload/download)
    └── tauri.conf.json       Minimal-permission capability (core:default)
```

## Features (v0.1)

- **Multi-cluster profiles**: create/delete, connection test (ListBuckets probe + latency), **disconnect / reconnect** (local state; reconnect re-probes), identity colors, endpoint/region/path-style/**self-signed cert toggle (insecure_tls: rustls skips certificate verification)**; **secret persistence (secrets.json, 0600 atomic writes) survives restarts**
- **Buckets**: list / create / delete (empty-bucket validation + confirmation)
- **Object browsing** (ListObjectsV2 + delimiter directory tree + encoding-type=url + **continuation-token pagination with "load more"**), delete, SigV4 presigned URLs (1h/24h/7d, V4/V2 compatible)
- **Multi-select**: checkbox batch selection (folders included), batch delete; **recursive folder delete** (ListObjectsV2 enumeration + DeleteObjects batches of 1000) with ghost-directory cleanup for SeaweedFS filer metadata
- **Upload/download** (native file dialogs + real-time progress events in the transfer bar; clear error for >5GB single-request uploads); ⌘K command palette, Ctrl+1..9 to switch clusters
- Error presentation with code/message/request-id triple, dark & light themes (remembered + follows system), full design-token coverage, keyboard accessibility (dialog Esc/focus management, list role/tabIndex)

## Roadmap (see docs/02 §3, Chinese)

- **v0.2**: secrets migration to system keyring; transfer engine (multipart upload + ranged resume + SQLite state store + progress event stream + SlowDown backoff)
- **v0.3**: bucket configuration management (Versioning/Lifecycle/CORS/Policy editors), capability probing & graceful degradation
- **v0.4**: cross-cluster migration, orphaned-MPU inspection, CLI command export

## CI

GitHub Actions build both platforms on every push/PR; pushing a `v*` tag creates a GitHub Release with the installers attached:

- **Windows**: NSIS `.exe` + `.msi` (x86_64) — [windows-build.yml](.github/workflows/windows-build.yml)
- **macOS**: `.dmg` + `.app` (Apple Silicon) — [macos-build.yml](.github/workflows/macos-build.yml)

> Note: builds are unsigned/notarized. On macOS, right-click the app → Open to bypass Gatekeeper on first launch.
