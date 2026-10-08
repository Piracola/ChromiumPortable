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

- 规范见 `docs/MIGRATION_RUST_TAURI.md` §2.2/§6（设计系统不变：唯一主角/配角、三级表面、四级字号、五种状态）
- `crates/builder-app/ui/` 是静态资产（无构建步骤、无 CDN、withGlobalTauri）；渲染层是 JS 拿状态快照填充，Rust 侧 Controller 只管状态
- **GUI 全部逻辑在引擎 crate**（`crates/portable-builder/src/gui/`），不依赖 tauri，`cargo test -p portable-builder --lib gui::` 无头可测；builder-app 只做命令转发
- 事件泵：runner 工作线程 → mpsc → 80ms 批量 emit `builder-event`（节流保留——逐行推会把 IPC 打满）
- 命令名与参数形状由 `app.js` 尾部「命令面契约」表定义，改一边要同步改另一边
- 验证 GUI 真能开：看**主窗口标题**（含 ChromiumPortable），别看进程还在不在
- 旧 pywebview 三坑的对应物见迁移文档 §6.4（编译期命令注册 / serde 反序列化拒绝 / tauri-build schema 校验）

## 目录与约定

- `crates/portable-builder/` — 构建引擎（CLI 面与旧 `python -m portable_builder` 一字不改）；`layout: auto` 静态识别主程序，绝不运行安装包/浏览器
- `crates/builder-app/` — Tauri 2 图形构建器；`ui/` 静态资产 + `src/main.rs` 命令面
- `scripts/` — 只剩 `update_chrome_plus.py`（维护者 CI 专用）与 `dispatch_child_builds.py`、`locales/wizard.json`（单一语言来源，引擎与 GUI 共用）
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
