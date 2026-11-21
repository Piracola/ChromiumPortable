# 架构沿革与历史包袱盘点

四个仓库（`ChromiumPortable` 以及 `Chrome-Portable` / `Edge_Portable` / `Helium_Portable`）的 Git 史一共 334 个提交，只有 **1 个 PR**（`ea6739d`，PR #1 `rust-migration`，2026-10-13），其余全部直推 main。下文所有结论都可用 `git log` / `git show` 复现；标了「推断」的是我根据证据做的判断，不是 git 的原文。

## 一、时间线

| 阶段 | 时间 | 形态 | 转折提交 |
| --- | --- | --- | --- |
| P0 各自为政 | 2026-01-23 → 04-26 | 每个浏览器仓库自带一套 Python 脚本直接构建、发 Release | Edge `8837f39`（436 行一次入库，`run.py` 303 行内置 CDP 接口）；Chrome `b1eac58`（17 文件 1183 行、5 个脚本、`build.yml` 181 行） |
| P1 共享引擎 | 04-27 | 子仓库只留 `browser.json` + 一个包装脚本，`uses:` 复用工作流，`pip install` 引擎 | `30bc417` 建引擎仓 → `c99b798`（script provider）→ `2f3e2ca`（`multi.py` 多目标共用一个 Release）；同日两侧砍掉 -876 / -949 行 |
| P2 补护栏 | 06-06 → 09-23 | 不变，只加断言与站点 | `27bd558` 注入断言、`dc731ce` 沿用旧资产、`a280046` 上游哈希 + 三层 ini、`8d5aa9f` `create_new_release_on`、`b89769a` 站点单一数据源 |
| P3 膨胀与迁移 | 09-24 → 10-08 | Python 引擎扩到 2954 行后，同日整体迁到 Rust | `4a90314`/`86aaa1b`（discovery / pe / brave_bundle / catalog / wizard / resolve.py）→ `afa7255` Rust 骨架 + 冻结 CLI → `98d9cea` 删光 Python（-4823 行） |
| P4 GUI 与 provider 收尾 | 10-09 → 10-15 | 引擎变纯 CLI，界面全在 `builder-app`；三条产品线零脚本 | `2fb731b` 删引擎内 `gui/`、`692e10c` 视觉系统 v4、`b977c63` `github_release` provider 取代三个包装脚本、`fe7970a` 清理移植叙述注释 |
| P5 收敛维护 | 11-08 → 现在 | reusable 工作流支持多目标，子仓库 `build.yml` 只剩 32 行 | `8aadfac`（多目标 + verify 按主机架构）、`3c1d7cf`（配置严格校验） |

10 月一个月 116 个提交，其中 10-08 单日约 100 个——迁移是一次集中冲刺，不是日常节奏。

## 二、发布模型：两种同名 tag，别混读

| tag | 位置 | 含义 |
| --- | --- | --- |
| 核心仓 `v1` / `v1.1` / `v1.2` | 04-27 → 05-03 | Python 引擎的契约版本，配合 `uses: portable-browser.yml@v1.x` |
| 核心仓 `v2` | 滚动 | **引擎包指针**。子仓库 `builder-ref` 默认值，指向哪个提交，三条产品线就拿哪份引擎包 |
| 核心仓 `v2.1` … `v2.5` | 10-14 / 11-08 | 灰度通道；`v2.5` 甚至没有并回 main |
| 子仓库 `v<浏览器版本>` | 每个 Release | 只是 Release 书签，指向当时的 HEAD，与引擎契约无关 |

全部是轻量 tag，所以 `%(creatordate)` 反映的是提交时间，不是打 tag 的时间。

关键一点：**子仓库的 `setdll/` 不存在**，chrome++ 的 DLL 只活在引擎包里（`scripts/stage-dist.ps1` 把 `setdll/` 打进 zip），而子仓库用 `gh release download "$env:BUILDER_REF"` 取包，只认 tag。所以「chrome++ 更新」这件事的完整链路是：同步提交 → 重切 `v2` → `release-engine.yml` 出包 → 派发子仓库。2026-11-17 之前这条链断了两次（PAT 缺 `actions:write`、派发传 commit sha 而下载只认 tag），1.20.0 / 1.20.1 因此压在仓库里没出厂，现已修好并写进工作区 `AGENTS.md` 的「Chromium 系的坑」。

## 三、有意为之的设计（看着可疑，但都是承重墙）

1. **引擎纯 CLI，GUI 只派生子进程**（`2fb731b` 之后）：`build_plan()` 同时服务预览与执行，命令序列与手敲 CLI 一致。把 GUI 逻辑搬回引擎会立刻破坏这条。
2. **`chrome++.ini` 三层合并**（`a280046`）：上游基线 → 本仓 defaults → 子仓库 override，按注释和键序合并。上游加键自动流入，这是它存在的全部理由。
3. **注入断言取「第一条导入」**（`27bd558` 起）：Chromium 原生就导入系统 `VERSION.dll`，弱断言等于没有断言。任何放宽都会让「setdll 没干活」这类故障静默通过。
4. **引擎包按 tag 分发**（`346a2ed`）：子仓库不再 checkout 引擎仓、不装 Python。代价是发布链路多一跳，收益是产品仓只剩配置。
5. **`catalog/` 与 `examples/` 也过 serde 严格校验**（`3c1d7cf`）：拼错的键当场报错，而不是回退成 x64。
6. **`brave_bundle.rs` 是活能力**：`lib.rs:12` 声明、`discovery.rs:8` 引用、`discovery.rs:548` 在 `is_brave_metainstaller` 分支里调用，配 `_migration/pe-golden/discovery/brave.json`。名字像某个浏览器时代的遗留，其实还在 `inspect-package` / `build-package` / GUI 的路径上。
7. **`_migration/pe-golden/` 里被代码真读的 10 个 JSON**（`catalog_reference.json`、`multi_*.json`、`versions_reference.json`、`python_reference.json` 等）：缺一个就有测试 panic。它们是 release 正文、版本判定、上下文渲染的唯一回归锁。

## 四、为已消失的问题而写的补偿

| 残骸 | 证据 | 它当初补偿的问题 |
| --- | --- | --- |
| `PYTHONPATH` 探测分支 | `main.rs:695-705`；日志文案 `main.rs:720`；报错文案 `builder.rs:645` | Python 引擎按包导入。引擎 `98d9cea` 已删，仓库里唯一设置 `PYTHONPATH` 的是 `.github/actions/build/action.yml:26`——而那个文件从没入库 |
| `.github/actions/build/action.yml` | `git ls-files` 为 0；`git check-ignore -v` 命中 `.gitignore:4` 的 `build/` | Python 时代的 composite action。被 `.gitignore` 的 `build/` 规则误伤（没有前导 `/`），本地躺着但永远进不了仓库 |
| `resolve-upstream` 子命令 | `main.rs:293` 直接 `bail!`；GUI 有按钮 `builder-app/main.rs:536`；`app.js:64,139` 有 i18n 键 | 迁移时打算把 `scripts/upstream/resolve.py` 移植成子命令，S9 波次没做 |
| catalog 里 6 个 `script` provider target | `catalog/browser_catalog.json:86-189` 全部指向 `scripts/upstream/resolve.py`，该目录不存在 | P1 的「子仓库自带包装脚本」模型；脚本 10-15 全部退役，catalog 没跟着改 |
| `helium_stable`（catalog） | `browser_catalog.json:79-100`：`script` + `layout:auto` + `ini_location:app_root` | Python 时代副本。真身 `Helium_Portable/browser.json:17-34` 是 `github_release` + `move_version_root` + `repack_layout`。`build-browser.yml` 的下拉里这一项今天必然失败 |
| `update-release`（单目标） | `main.rs:106`；CI 与 GUI 均 0 调用（CI 走 `update-release-targets`） | 多目标模型（`2f3e2ca`）之后的残留入口 |
| 15 处 `builder.py::` / `review-Wave3` / `round 36` 之类标记 | `builder.rs:63,142,170,601,852,1075,1118,1196`、`release.rs:80` 等 | 迁移期的语义锚点。函数级对照还有参考价值，波次/轮次编号已经没有对应文档 |
| 三个子仓库 `.gitignore` 的 Python 条目 | Chrome：`Pipfile.lock`、`chrome_installer-main`、`__pycache__/`、`*.py[cod]`；Edge/Helium 类似 | 子仓库曾经自带 `.py` |
| `chrome++/injectpe.bat`（三个子仓库都有） | 构建路径不读它（`builder.rs:707-774` 只拷 `version.dll`、setdll、ini）；Chrome 版默认还是 `msedge.exe` | P0 时代手工注入的说明书。Helium 版默认 `helium.exe`，上游早没这名字 |

## 五、可以安全移除的清单

### A · 立即可删（零引用、零 CI 依赖、用户不可见）——已于 2026-11-17 执行

| 路径 | 证据 |
| --- | --- |
| ~~`.github/actions/build/action.yml`~~ | 已删。`.gitignore` 的 `build/` 已改成 `/build/`（顺带补 `/target/`） |
| ~~`main.rs` 的 PYTHONPATH / `_portable_builder` 探测，`builder.rs` 里的 PYTHONPATH 文案~~ | 已删。引擎已无 Python 包 |
| ~~`log_fmt::done`、`IniEncoding::from_name`、`release::asset_matches_target`、`release::delete_assets_by_pattern`~~ | 已删。全仓 0 调用（`pub` 让 clippy 也不会报） |
| ~~`_migration/pe-golden/` 下 15 个零引用文件~~ | 已删。代码 / docs / CI / scripts 全 0 命中 |
| 三子仓库 `.gitignore` 里的 Python 条目 | 仓库内已无 `.py` |

### B · 需一次验证后删

| 路径 | 说明 |
| --- | --- |
| ~~`resolve-upstream` 全套~~ | 已删：引擎子命令 + GUI 工具卡片 + `app.js`/`index.html` 的 i18n 键同时移除 |
| ~~catalog 6 个 `script` target~~ | 已修：`helium_stable` 改成真身 `github_release`；其余 5 个改成 `direct`，必须给 `package_url` / `package_path`。`docs/DEVELOPMENT.md` 已同步 |
| `providers/script.rs` | 保留：只有 `examples/script-provider.browser.json` 与自写配置在用，产品线 0 使用，是自定义上游的通用口子 |
| `update-release`（单目标） | 保留：文档已标为手动单目标用，CI 走 `update-release-targets` |
| `chrome++/injectpe.bat` × 3 | 构建路径不读它；保留最多一个（Edge 形态最贴近真实路径），Helium 版可直接删 |
| `_migration/pe-golden/{build_context_reference.json, config_toml_errors_golden.json, verify_archive_locate_golden.json}` | 只在注释里点名，期望值已内联；删文件要顺手改注释 |

### C · 不要动（像包袱，其实承重）

- `brave_bundle.rs`（`discovery.rs:8,548` 真调用 + 黄金样本 + `build-browser.yml` 的 `brave_stable`）。
- `builder.rs:863` 的 `relpath_from`：`verify` 拒绝绝对路径导入的判定仍在，`exe_name: ..\chrome.exe` 的配置形状今天还在用。只有 `(review-Wave3 finding 1, HIGH)` 这个标签可以丢。
- `tools.rs:237-246` 的「无总超时 + connect_timeout + keepalive」：慢速大包会被总超时腰斩，这是行为不是考古。
- `config.rs` 里 11 个当前没有配置使用的字段（`smoke_*`、`verify_architecture`、`version_dll_name` 等）：`Target` 是 `deny_unknown_fields` 的封闭集合，删字段等于用户写了就报错；它们都有真实读取点。
- `_migration/pe-golden` 的 10 个真读取文件、`setdll/`、`scripts/` 三个活脚本、`docs/` 生成链。

## 六、建议的处理顺序

1–4 已于 2026-11-17 执行：catalog 修好、`resolve-upstream` 三处同删、Python 残迹清掉、
4 个 `pub fn` 与 15 个零引用 golden 删除。剩下的：

1. **`injectpe.bat`**：确认没人手工依赖后删（三子仓库各一份）。
2. **三子仓库 `.gitignore`** 的 Python 条目。
3. 三个只在注释里点名的 golden（B 表最后一行）：删文件时顺手改注释。

## 七、事实与推断

- 事实：所有 hash、日期、行数、文件树、引用计数、tag 指向与轻量性、唯一 PR、`builder.py::` 仍存 15 处、`script` provider 无产品消费者。
- 推断：`docs/MIGRATION_RUST_TAURI.md` 里写的「迁移前 GUI 约 1950 行 pywebview + `portable_builder/web_ui.py` 1505 行」在 git 全历史里都不存在（`git log --all -- portable_builder/web_ui.py` 为空）；最可能是迁移前只存在于本地、文档照它写了基线。GUI 在 git 里的第一次出现是 `6c0f42d` 的 938 行静态资产。
- 推断：`v2.1`–`v2.4` 集中在 10-14 一天，应属引擎包模型刚落地时的临时验证 tag，稳定后才把 `v2` 移到 `8aadfac`。
- 推断：`script` provider 保留是给非产品目标留的通用能力（catalog 里 5 个非官方浏览器），不是漏删；但 catalog 指向的脚本没了，这条能力目前是断的。
