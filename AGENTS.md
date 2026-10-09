# AGENTS.md

ChromiumPortable：Chromium 系浏览器便携构建核心（解包 → Chrome++ 注入 → 打包/验证/发版）。不发布浏览器成品。

## 怎么跑

```powershell
cargo test --workspace                       # 引擎 + GUI 后端测试
cargo clippy --workspace -- -D warnings      # CI 门禁之一
cargo run -p builder-app                     # 图形界面（开发模式）
cargo build -p portable-builder --release    # 引擎 CLI
target\release\portable-builder.exe --workdir . build-package <安装包> --archive
```

产品渠道 CI 入口是 reusable workflow（v2：子仓库从本仓库 Release 下载预编译引擎包）；
单浏览器按需构建用 `build-browser.yml`。下载站生成不变：`cd docs; python tools/generate.py`。

## 技术栈

Rust（引擎 `crates/portable-builder`：clap / reqwest / pelite，**纯同步、无 async runtime**；
GUI `crates/builder-app`：Tauri 2 + WebView2）、7-Zip/`7zr.exe`（外部工具，发现顺序见
`src/tools.rs`）、GitHub Actions。站点为 `docs/` 静态页（`tools/generate.py` 由
`data/site.json` 渲染，这部分保持 Python）。

## 图形界面

- 2026-10 重写版：**GUI 逻辑全部在 `crates/builder-app`**（`src/main.rs` 命令面 + `ui/` 静态渲染层）；引擎 crate 是纯 CLI，不含任何 GUI 代码
- `ui/` 无构建步骤、无 CDN、无外部字体；文案集中在 `app.js` 的 STR 表（zh/en），HTML 的 `data-i18n` 键与之对应
- Rust 侧只有 7 个命令：snapshot / workdir / plan / start / cancel / open / save_log；所有引擎工作 = 子进程调同目录 `portable-builder.exe`，命令序列与手敲 CLI 完全一致（`--workdir` 在前）
- 计划只有一个事实来源：`build_plan()` 同时服务「计划预览」与「实际执行」，预览即真实命令
- 事件泵：子进程 stdout/stderr → mpsc → 80ms 批量 emit `app-event`（节流保留——逐行推会把 IPC 打满）
- Tauri 2 capabilities 在 `crates/builder-app/capabilities/main.json`；对话框/剪贴板/事件权限都在这里声明
- 验证 GUI 真能开：看**主窗口标题**（含 ChromiumPortable），别看进程还在不在

## 目录与约定

- `crates/portable-builder/` — 构建引擎（CLI 面与旧 `python -m portable_builder` 一字不改）；`layout: auto` 静态识别主程序，绝不运行安装包/浏览器
- `crates/builder-app/` — Tauri 2 图形构建器；`ui/` 静态资产 + `src/main.rs` 命令面 + `capabilities/`
- `scripts/` — 只剩 `update_chrome_plus.py`（维护者 CI 专用）与 `dispatch_child_builds.py`
- `catalog/browser_catalog.json` — 在线 target；非产品必须 `Unofficial` + Disclaimer
- `installers/` — 用户投放安装包（已 ignore）；`bin/` — 回归样本（已 ignore，勿提交）
- `docs/` — GitHub Pages 下载站；HTML 是生成产物，改 `data/site.json` 后重跑生成器
- README 仅中英双语；下载站 `docs/` **保持四语**（zh-CN / zh-TW / en / ja）
- 徽标用 shields `flat-square`，表内数字只出现在徽标里；发布日期与 `docs/data/releases.json` 对齐

## 当前状态与下一步

Rust + Tauri 2 迁移完成（M0–M6，见 `docs/MIGRATION_RUST_TAURI.md`）：引擎 17 子命令、GUI 后端、
CI（engine-ci.yml / release-engine.yml）与 v2 引擎包下载全部就位。产品面仅 Chrome / Edge / Helium，
三子仓库按迁移文档 §8.3 切 `@v2`。语言策略：README 双语、下载站四语。新浏览器优先 `layout: auto`
+ catalog；涉及注入/验证的改动需用三渠道实际配置回归。
