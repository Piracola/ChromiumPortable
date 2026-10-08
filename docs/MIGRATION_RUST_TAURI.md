# 迁移工程文档：Rust + Tauri 2

本文是整个重构的施工图：迁移什么、不迁移什么、每块代码落到哪里、分几步走、
每一步怎么验收、出问题怎么回滚。它**先于代码**存在——所有结构性决策在这里定死，
写代码时不再重新发明。

评估结论见会话记录，这里只复述一句话作为前提：**GUI 层是压倒性正收益（PyInstaller
打包问题被整类取消），引擎层小幅正收益（CI 分钟持平、确定性上升、PE 解析净删代码），
docs 站点不动。**

---

## 0. 范围与非目标

### 迁移

| 组件 | 现状 | 去向 |
|---|---|---|
| `portable_builder/` 引擎（23 文件，5289 行） | Python 3.12 | Rust crate `portable-builder`（CLI 面**一字不改**） |
| `scripts/gui*.py` 图形构建器（约 1950 行） | pywebview + WebView2 | Tauri 2 app crate `builder-app` |
| `portable_builder/web_ui.py`（1505 行） | Python 字符串渲染 HTML | `builder-app/ui/` 静态资产（CSS/骨架平移，渲染层重写为 JS） |
| `tests/`（1446 行） | unittest + 真窗口几何 | cargo test + Playwright 无头几何 + 真窗口冒烟 |
| `scripts/build_exe.py` + `打包EXE.bat` | PyInstaller | **删除**，被 `cargo tauri build` 取代 |

### 保留 Python（明确不迁）

| 组件 | 理由 |
|---|---|
| `docs/tools/generate.py`（1017 行）+ `docs-site.yml` | 跑 ubuntu、有 `--check` 防漂移、与 Pages 咬合良好，重写零收益 |
| `scripts/update_chrome_plus.py`、`dispatch_child_builds.py` + `update-chrome-plus.yml` | 与构建引擎无依赖关系的独立流水线 |

### 退役（迁移完成后删除）

- `scripts/wizard.py`（452 行）——控制台向导。GUI 完整覆盖其功能，且 WebView2 的
  系统要求（Win10 1803+）与现状 pywebview 完全相同，无可达性损失。`开始构建.bat`
  改为启动 GUI exe。这是**有意的破坏性变更**，记入第 11 节清单。
- `scripts/build_all_packages.py`——引擎已有 `build-packages` 子命令，纯冗余。
- `build/pyinstaller/` 工作目录、`requirements-build.txt`。
- Python 引擎本体（M6 才删，M3–M5 期间作为对照实现与回滚保险保留）。

---

## 1. 现状基线（迁移对照的锚点）

以下是写代码前必须对齐的事实，全部来自当前 `main` 实测：

- **引擎 CLI 面**（`python -m portable_builder`）：全局参数
  `--config / --target / --workdir / --builder-dir`；子命令 `check`、`build`、
  `archive`、`verify`（`--archive` / `--no-smoke`）、`verify-targets`、
  `inspect-package`（`--architecture` / `--json` / `--keep-extracted`）、
  `build-package`（`--output-dir` / `--archive`）、`build-packages`、
  `research-packages`、`render-release`、`update-release`、`check-targets`、
  `build-targets`、`render-release-targets`、`update-release-targets`。
- **Provider 四种**：`direct` / `google_omaha` / `microsoft_edge` / `script`。
  Edge provider 的 `verify_ssl` **默认 False**（微软 CDN 的证书链问题），移植时不得「顺手修正」。
- **布局四型**：`auto`（默认，静态识别）/ `move_version_root` / `move_version_dir` / `copy_version_root`。
- **GITHUB_ENV/GITHUB_OUTPUT 契约**：`UPDATE_NEEDED`、`CREATE_NEW_RELEASE`、
  `UPSTREAM_VERSION`、`MINOR_UPDATE`、`RELEASE_ID`、`RELEASE_TAG`、`RELEASE_TITLE`、
  `RELEASE_BODY_PATH`、`ARCHIVE_NAME`、`ARCHIVE_SHA256`、`ARCHIVE_SIZE`、
  `CHROME_PLUS_VERSION`、`env_json`（check 步骤输出，多行值走
  `PORTABLE_BUILDER_EOF` heredoc 分隔符）。**这些名字是三个子仓库 workflow 的
  真实依赖，一个都不能改。**
- **日志前缀约定**：`[INFO] / [WARN] / [FAIL] / [OK] / [DONE]`。保留原样，便于
  人工读 Actions 日志时新旧对照。
- **7z 工具发现顺序**（`tools.find_7z_tool`，顺序即语义）：
  系统安装路径（两个 Program Files 路径）→ 工作目录 `7zr.exe` → `PATH` 上的 `7z`
  → 从 7-zip.org / develar 镜像下载 `7zr.exe` → Chocolatey 安装。
  `inspect-package` 路径传 `allow_download=False, allow_system_install=False`。
- **GUI**：Controller 约 60 个桥接方法；事件泵为后台线程 80ms `drain_once()` →
  `bridge.emit`；文件对话框是「Python 侧问、JS 回调 `done`」的异步模式；
  日志文件重定向用环境变量 `CHROMIUMPORTABLE_LOG_FILE`；子进程输出按系统
  OEM/ANSI 码页解码。
- **测试四件套**：`test_discovery.py`（199）、`test_gui.py`（271，`inspect.signature`
  核对窗口参数）、`test_gui_layout.py`（840，真实 pywebview 窗口里 `evaluate_js`
  量几何）、`test_gui_launch.py`（136，判据是「新窗口标题含 ChromiumPortable」，
  不是进程存活）。

---

## 2. 目标架构

### 2.1 Cargo workspace 布局

```
ChromiumPortable/
├── Cargo.toml                  # workspace：members = ["crates/*"]
├── crates/
│   ├── portable-builder/       # 引擎：lib + bin，纯同步，无 GUI 依赖
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs         # clap 入口（对应 cli.py + __main__.py）
│   │       ├── lib.rs
│   │       ├── config.rs       #   ← config.py（serde 强校验，见 §4.1）
│   │       ├── builder.rs      #   ← builder.py
│   │       ├── discovery.rs    #   ← discovery.py
│   │       ├── pe.rs           #   ← pe.py + tools.py 里的三段手写 PE 解析（统一收口到 pelite）
│   │       ├── tools.rs        #   ← tools.py（下载/sha256/7z 发现/解压）
│   │       ├── verify.rs       #   ← verify.py
│   │       ├── multi.rs        #   ← multi.py
│   │       ├── release.rs      #   ← release.py
│   │       ├── versions.rs     #   ← versions.py
│   │       ├── github_env.rs   #   ← github_env.py
│   │       ├── ini_overlay.rs  #   ← ini_overlay.py（语义 1:1，见 §4.4）
│   │       ├── brave_bundle.rs #   ← brave_bundle.py（BCJ2，唯一算法风险，见 §4.5）
│   │       ├── i18n.rs         #   ← i18n.py（wizard.json 单一来源不变）
│   │       ├── gui/            #   ← gui_core.py / gui_plan.py / gui_runner.py / gui.py Controller
│   │       │   ├── mod.rs      #      环境探测、engine 定位、catalog、语言包、--selftest 检查
│   │       │   ├── plan.rs     #      BuildRequest → 子进程步骤（纯函数）
│   │       │   ├── runner.rs   #      子进程执行 + 日志增量回流
│   │       │   └── controller.rs #    全部状态与逻辑（不含渲染、不含框架）
│   │       └── providers/      #   ← providers/（mod/direct/google_omaha/microsoft_edge/script）
│   └── builder-app/            # Tauri 2 外壳：只有命令转发与窗口
│       ├── Cargo.toml
│       ├── tauri.conf.json
│       ├── build.rs
│       ├── icons/icon.ico      # 由现 icon.py 一次性生成后提交，不再运行时生成
│       ├── src/main.rs         # #[tauri::command] 命令面 + 80ms 事件泵 + --selftest 分支
│       └── ui/                 # ← web_ui.py 抽出；无构建步骤、无 CDN（原则平移）
│           ├── index.html
│           ├── styles.css      #   设计系统原样平移（三级表面/四级字号/五状态）
│           ├── app.js          #   渲染层重写 + invoke() 调用
│           └── locales/wizard.json  # 与引擎共用同一份文件
├── catalog/  setdll/  docs/    # 原样保留（docs 生成器仍是 Python）
└── scripts/                    # 只剩 update_chrome_plus.py / dispatch_child_builds.py
```

### 2.2 两条硬性架构决策

**决策 A：引擎保持纯同步，不引入 tokio。**
引擎的本质是子进程编排（下载 → 解压 → 拷贝 → setdll → 7z 打包），`std::process`
顺序执行最简单、最可测。GUI 的「不卡界面」由 `builder-app` 自己开工作线程解决
（镜像现在的后台事件泵），而不是把 async 感染进引擎。引擎 crate **不得依赖 tauri
或任何 async runtime**——这样 CI 里 `cargo test -p portable-builder` 在 headless
环境直接跑。

**决策 B：前端渲染层从「Rust 拼 HTML」改为「JS 拿状态渲染」。**
现状是 Python 侧渲染整页 HTML（翻译也发生在渲染前）。平移成 Rust 拼 HTML 看似
1:1，实际要把 700 行 f-string 模板翻成 Rust `format!`，且新旧实现无法做有意义的
逐字节对照（一次重排就全 diff）。改为：命令返回**状态快照 JSON + 一次性下发语言包**，
JS 渲染。收益：Rust 侧薄（Controller 只管状态，与现有文档「Controller 与框架解耦」
的原则一致）；渲染正确性用 DOM 断言测（见 §7），这才是真验收。CSS 与页面骨架从
`web_ui.py` **原样平移**——历史上那批版式事故的防线（设计规范 + 几何回归）完整保留。

### 2.3 tauri.conf.json 关键项

```jsonc
{
  "productName": "ChromiumPortableBuilder",
  "identifier": "org.piracola.chromiumportable.builder",
  "app": {
    "windows": [{
      "title": "ChromiumPortable 便携构建器",   // 与现窗口标题一致：test_gui_launch 判据平移
      "width": 980, "height": 720, "resizable": true
    }],
    "security": { "csp": null }                 // 离线本地静态页；见 §10 风险 R7
  },
  "build": {
    "frontendDist": "ui",                       // 相对 tauri.conf.json（静态目录，无打包器）
    "devUrl": null                              // 无 vite：dev 模式同样吃静态目录
  }
}
```

`withGlobalTauri: true`（bundle 配置里），`app.js` 用 `window.__TAURI__.core.invoke()`
调命令——保持「纯 JS、无构建步骤」的现行原则。

---

## 3. 依赖选型

| 用途 | 现状（Python） | Rust 选择 | 说明 |
|---|---|---|---|
| CLI | argparse | `clap` 4（derive） | 子命令/参数面按 §1 原样 |
| HTTP | requests | `reqwest` 0.12（blocking，rustls-tls） | 引擎是同步的，用 blocking API；`verify_ssl=false` 用 `danger_accept_invalid_certs`，只允许在 Edge provider 打开 |
| SHA256/编码 | hashlib/base64 | `sha2` + `hex` + `base64` | `normalize_sha256` 语义（hex / `sha256:` 前缀 / base64 三态）原样移植并配单测 |
| PE 解析 | pe.py 188 行 + tools/discovery 两段重复手写 | **`pelite` 0.10** | 机器类型、导入表、资源目录、VERSIONINFO 全有，纯 Rust 跨平台——check job 在 ubuntu 上从此也能读版本信息（现状读不到，因为走 Win32 version.dll） |
| XML（Omaha） | xml.etree | 不引新库：手写极简命名空间无关解析或用 `quick-xml` | Omaha 响应结构固定；`quick-xml` 只在手写不划算时引入 |
| INI 合并 | ini_overlay.py 自写 | 自写移植 | **不用 rust-ini**：合并语义是「逐行改基线、保留全部注释与顺序、未知键报错」，rust-ini 读写会重排文件。这是 1:1 移植 + 黄金测试（§4.4） |
| tar（Brave 解包） | tarfile | `tar` 0.4 | brave_bundle 的 BCJ2 见 §4.5 |
| LZMA | lzma（标准库） | `lzma-rust`（raw LZMA1） | M1 spike 验证 raw 流解码可用性 |
| 正则 | re | `regex` 1 | 版本号、`archive_name_regex`、`ABSOLUTE_VERSION_DLL` 检查 |
| 日期 | datetime | `time` 0.3 | 只需要 `yyyy-MM-dd` 与比较 |
| 错误 | raise/except | `anyhow`（引擎 bin）+ `thiserror`（lib 错误类型） | 失败路径类型化，但**不追求**全量错误分类，够用即止 |
| 子进程 OEM 解码 | codecs | `encoding_rs` + `windows-sys`（GetACP/GetOEMCP） | setdll 与 7z 输出解码，行为对齐 gui_runner 现状 |
| 文件/目录对话框 | pywebview 内建 | `tauri-plugin-dialog` | 官方插件，覆盖 pick_*/new_folder/_ask_save 全部场景 |
| 资源管理器打开 | os.startfile | `tauri-plugin-opener` | open_workdir / open_subdir / open_path |
| GUI | pywebview | `tauri` 2 | WebView2，依赖面与现状相同 |

刻意**不引入**：tokio（决策 A）、serde_yaml（见 §4.1）、任何前端框架/打包器
（无构建步骤原则）、`sevenzip-rust`（7z 仍走外部可执行文件：7zr 只解 .7z，
cab/msi/rar 本来就需要完整 7z，换库只省一次子进程却引入解压语义风险）。

---

## 4. 引擎迁移：逐模块语义要点

只列「容易在翻译中走样」的点，机械翻译的部分不赘述。

### 4.1 config.rs —— 配置强校验 + 收窄格式

- 支持 **JSON 与 TOML**；YAML 支持废除（现状 yaml 走可选依赖，实际子仓库全部用
  JSON）。记入第 11 节破坏性变更。
- `Target` 定义为 serde 结构体，**所有** `target.get("key", default)` 缺省链
  （exe_name / output_dir / version_dll_location / ini_location /
  chrome_plus_dir / allow_7z_download / …）落成 `#[serde(default)]` 字段，默认值
  逐一从 builder.py 抄，写一张「键 → 默认值」表进代码注释。坏配置在入口报
  逐字段错误，而不是跑到一半 panic。
- catalog（`catalog/browser_catalog.json`）用同一套类型解析；GUI 的在线目标列表
  与 `prepare-target` 子命令都吃它。

### 4.2 tools.rs —— 顺序与边界

- 7z 发现顺序按 §1 逐条移植；下载镜像 URL 列表原样。
- `download_file`：流式写盘 + 边下边算 sha256 + size 校验；缓存命中时先验摘要、
  失败删缓存重下——这套语义有既有行为依赖（CI 缓存的安装包），别简化。
- `human_size`：1024 进制、Windows 资源管理器式标签（B/KB/MB/GB，B 取整、其余
  一位小数）——release 模板里用了它，输出必须逐字符一致。
- `assert_portable_version_import`：PE 导入表必须含 **相对** `version.dll`，且
  不匹配 `^[A-Za-z]:\\.*version\.dll`（绝对路径 = 注入失败）。用 pelite 重写，
  断言语义不变。
- `assert_no_forbidden_files`：`forbidden_file_names` 拦截不变。

### 4.3 providers/ —— 各自的坑

- `google_omaha`：XML POST 体、渠道 appid/ap 表、`nextversion=""` 等字段逐字
  保留（服务端对这些字符串敏感）。
- `microsoft_edge`：**`verify_ssl` 默认 False**、UA 伪装串、两个 API 端点模板。
- `script`：命令执行 → stdout 里**从后往前**找第一个 `{...}` JSON 行的解析语义；
  该 provider 是 catalog 非内置上游的通用口子，必须保留。
- `direct`：`version_regex` 从 URL 提版本、缺省 `0.0.0.0`。
- `resolve.py`（350 行 Omaha/CDP 解析）不再作为外部脚本：移植为引擎子命令
  `resolve-upstream`，`script` provider 继续可用（语义不变），catalog 里指向
  自身的调用改为内置。见 §5。

### 4.4 ini_overlay.rs —— 语义 1:1 + 黄金测试

三层合并（基线 `setdll/chrome++.ini` → 全局 `chrome++.defaults.ini` →
子仓库 `chrome++/chrome++.override.ini`；完整 ini 覆盖时打 WARN）必须原样：
- 编码探测顺序 UTF-16 → UTF-8-SIG → UTF-8，**按探测结果原编码写回**（上游是
  UTF-16LE BOM）。
- 逐行合并、保留注释与顺序、**覆盖基线不存在的键 = 报错**（防拼写错误静默失效）。
- 验收：拿仓库里真实的三个 ini（基线/defaults/任一 override）做黄金用例，
  输出与 Python 版逐字节一致。

### 4.5 brave_bundle.rs —— 唯一的算法移植风险（M1 spike）

BCJ2 容器头（5×u32）+ x86 分支转换 + LZMA 解压。这是全项目唯一「实现算法」
而非「编排流程」的代码。处理方式：
1. M1 第一周做 spike：`lzma-rust` 解 raw LZMA1 + 手移植分支转换表；
2. 黄金验证：用 `bin/` 回归样本里的 Brave metainstaller，对 Python 版输出做
   逐字节比对；
3. 若 spike 失败，回退方案是把 Brave 解包降级为「7z 完整版能开就开，开不了报
   不支持」——Brave 不在产品面三渠道里，风险可控；**不允许**为它引入低质量依赖。

### 4.6 release.rs / multi.rs / github_env.rs —— CI 契约层

- 渲染模板的占位符正则（`PLACEHOLDER_PATTERNS`）、干跑样本版本 `123.456.789.0`
  与样本日期 `2099-12-31`、`assert_body_versions` 全部原样——这是防止 release
  标题/正文渲染回归的既有防线。
- 版本升级判断（`is_upgrade` / `should_create_new_release` / `create_new_release_on`
  策略）语义原样，配版本串比较单测（含不等长段、非数字段）。已知偏差（review-A 确认可
  接受）：Rust 侧全角数字（如 `１`）与溢出段按 i64 解析失败处理为非数字段（比较得 0），
  Python 的 `int()` 会接受全角数字——真实上游版本号全 ASCII 且段长 ≤3 位，不可达。
- `env_name`：`[^A-Za-z0-9] → _`、去首尾、大写——GITHUB_ENV 变量名清洗。
- `write_env` 的 `env_json` blob 是**保序紧凑 JSON**（`ensure_ascii=False` +
  `separators=(",",":")`）——serde_json 默认 Map 按 key 排序，必须启用
  `preserve_order` feature 或用有序 Vec 手工拼接，否则与 Python 字节序不一致。
- `append_lines` 在 Rust 侧恒写 LF（Python 文本模式在 Windows 写 CRLF）；S1 只冻结
  变量名与分隔符，Actions 消费端两种行尾都接受（review-A 判定接受）。
- heredoc 分隔符 `PORTABLE_BUILDER_EOF` 只用于含换行值，行为一致。
- GitHub API 调用（latest release、删旧资产、传新资产）用 reqwest 手写，路径与
  分页语义对齐 release.py；不引 octocrab（依赖面不值）。

### 4.7 verify.rs —— 冒烟判据

静态验证（解包 + 导入表断言）不变；smoke 模式（真启动浏览器、确认 profile 重定向
到便携 Data 目录）用 `std::process` + 超时移植。**启动判定不得换成「进程活着」**——
崩溃对话框也会让进程活着，这是写进 AGENTS.md 的既有教训。

---

## 5. CLI 兼容契约

引擎二进制名 `portable-builder.exe`。子命令、参数、退出码（0 成功 / 非 0 失败 +
`[FAIL]` 到 stderr）、`GITHUB_ENV`/`GITHUB_OUTPUT` 输出名、日志前缀，全部按 §1
保持不变。新增两个子命令（吸收 scripts/ 里的预处理逻辑，行为对齐）：

```
portable-builder prepare-target --browser <id> [--url URL] [--path PATH]
                                [--architecture ARCH] [--output build/selected.browser.json]
portable-builder resolve-upstream <provider-args>   # ← scripts/upstream/resolve.py
```

`--selftest`（GUI exe 的自检入口，`--out <report>`）平移到 `builder-app`（§6.6）。

环境变量：`CHROMIUMPORTABLE_LOG_FILE`（日志重定向）名称不变；
`WIZARD_LANG / CHROMIUMPORTABLE_LANG` 语言覆盖名称不变。

---

## 6. GUI 迁移

### 6.1 前端抽取（web_ui.py → ui/）

- CSS 整段平移；HTML 骨构从 Python 构建器里摘出为静态结构；`TOOL_GLYPHS`
  （几何字形，刻意非 emoji）与 `TOOL_ORDER` 原样。
- 渲染改 JS：`invoke('snapshot')` 拿状态 JSON，`invoke('strings')` 一次性拿语言包
  （含 `{field}` 占位替换语义：缺失键**回落显示裸 key** 而不是报错——这是现行
  行为，防「少一条语言包开不了窗」，必须平移）。
- 切语言 = 重新 render 一次（单语言页面原则不变）。

### 6.2 命令面映射（Controller → #[tauri::command]）

| 现方法（gui.py） | 新命令 | 说明 |
|---|---|---|
| `state / snapshot_for_render` | `snapshot` | 单一状态快照，前端渲染的唯一切口 |
| `set_lang`（`toggle_lang` 由前端换按钮文案） | `set_lang(lang)` | 返回新快照+新语言包 |
| `refresh_plan`（含整份表单入参） | `refresh(values?)` | 带 values 就落库+重算计划；不带就走 refresh_targets/refresh_installers |
| `set_* ` 全部 setter | `set_input(key, value)` | 键名沿用 JS 契约（`local_path` → `local`），非法枚举值忽略 |
| `start_build / run_tool / cancel` | `start / run_tool / cancel` | 语义不变（cancel 仍是两下） |
| `pick_installer / pick_local / pick_tool_file / pick_folder / pick_workdir / pick_tool_dir / _ask_save` | `pick(kind)` | tauri-plugin-dialog 的阻塞对话框——替换「Python 问、JS 回调 done」模式 |
| `open_workdir / open_subdir / open_path` | `open_path(path)` | 直接 shell 打开（Windows `start` / `open` / `xdg-open`），不引 opener 插件 |
| `append_log / log_text / clear_log / save_log / set_autoscroll / toggle_log` | `log(action, value?)` | action = clear / toggle / autoscroll / save / text |
| `done`（文件对话框回调） | 不再需要 | 阻塞命令天然等待 |

**返回值统一形状**：除 `strings` / `log(action="text")` 外，所有命令返回
`{snapshot, strings}`——前端拿到就整页重渲染，不需要按命令记返回类型。

**事件形状**（`builder-event`）：`{kind: "state"|"log"|"step"|"done", …}`；
`log` 带 `text`/`tag`，`step` 带 `index`/`total`/`title`，
`done` 带 `code`/`elapsed`/`result`/`state`。

### 6.3 事件泵（80ms drain → mpsc + emit）

现状：后台线程 80ms `drain_once()` 把队列事件批量 `bridge.emit`。
平移：`runner.rs` 工作线程跑步骤、日志增量推 `std::sync::mpsc`；一个 pump 线程
80ms 批量 `app_handle.emit("builder-event", …)`。**保留批量节流**——逐行 emit
会在解包几千行日志时把 IPC 打满（现状注释里写明的既有教训）。

### 6.4 pywebview 三坑 → 结构性对策（验收精神平移）

| 旧坑（AGENTS.md） | 新栈对策 | 对应测试 |
|---|---|---|
| `dir(js_api)` 静态枚举看不见动态转发 | `generate_handler!` 编译期注册，漏注册=编译失败 | 编译本身 + 一条「快照含全部命令」的冒烟单测 |
| JS 无参调用传 `undefined` → TypeError 只进日志 | serde 反序列化，类型错=显式 rejected promise | 前端无头用例：错误参数必须浮出 UI 横幅而非静默 |
| `create_window()` 塞编造参数，冻结后才炸 | `tauri.conf.json` 由 tauri-build 做 schema 校验 | CI 构建即验证 |

第四条（AGENTS.md 的历史教训）：**启动冒烟判据 = 出现标题含 ChromiumPortable
的新窗口**，不看进程存活——`test_gui_launch.py` 的精神原样平移（§7）。

### 6.5 i18n 与 catalog 资源

- `scripts/locales/wizard.json` 是唯一语言来源（zh-CN / en），引擎与 GUI 共用：
  `builder-app` 编译期 `include_str!` 内嵌（离线原则）；引擎子命令按磁盘读
  （builder-dir 定位逻辑平移 `resolve_builder_dir` 的四级探测：显式参数 →
  PYTHONPATH 同级 setdll → `_portable_builder` → 包旁目录——前两条在 Rust 里
  改为环境变量 `PORTABLE_BUILDER_DIR` 与 cwd 探测，写进迁移说明）。
- `catalog/browser_catalog.json` 作为 tauri resource 打进 app，也编译期内嵌一份
  供在线目标列表；引擎 CLI 从磁盘读。

### 6.6 `--selftest`

`main.rs` 在启动 tauri 之前解析 argv：`--selftest --out <path>` 时不创建窗口，
顺序执行「引擎自检（版本、7z 可用性、catalog 解析、i18n 加载）」写报告退出。
这延续了 `build_exe.py run_selftest` 的验收思路，但现在是 CI 在 Windows runner
上对**真实 release 产物**跑，而不是本地打包后手工跑。

### 6.7 图标

`icon.py`（131 行，程序化生成 .ico）不移植：用它生成一次 `icon.ico` 提交进
`builder-app/icons/`。图标几乎不变，运行时生成没有价值。

---

## 7. 测试迁移映射

| 现测试 | 新形态 | 跑在哪 |
|---|---|---|
| `test_discovery.py`（199 行） | cargo test：`bin/` 黄金样本 → `inspect-package --json` 输出深度相等 | windows（需要 7z；纯解析断言部分可在 ubuntu） |
| `test_gui.py`（271 行，signature 核对） | 大半被编译期吃掉（§6.4）；保留「命令注册完整性」「i18n 键完整性」两条单测 | ubuntu |
| `test_gui_layout.py`（840 行，真窗口量几何） | **Playwright 无头**：`ui/` 是独立静态页，注入 mock invoke 后量 DOM 几何（等宽数据表、文字不裁切、四级字号断言照 `docs/ui-architecture.md` 验收标准抄）；另保留一条**真 WebView2 窗口抽样**（tauri 冒烟 + 截图尺寸核对） | 无头=ubuntu；真窗口=windows |
| `test_gui_launch.py`（136 行，窗口标题判据） | tauri app 启动冒烟：断言主窗口标题出现且进程未提前退出、无错误对话框 | windows |

引擎侧新增：ini_overlay 黄金（§4.4）、versions/release 渲染黄金（§4.6）、
normalize_sha256 / human_size / env_name 单测、providers 的 HTTP 录制回放
（用 `bin/` 样本 + 离线 fixture，不打真实上游）。

**三渠道回归（既有纪律，平移）**：任何涉及注入/验证的改动，用 Chrome / Edge /
Helium 三个真实子仓库配置各跑一次完整 build→archive→verify——这条来自 AGENTS.md，
在新引擎 M3 门禁里强制执行。

---

## 8. CI 与分发迁移

### 8.1 workflow 逐个改法

| workflow | 改法 |
|---|---|
| `portable-browser.yml`（reusable，被 3 个子仓库调用） | **v2 tag**：check 与 build 两个 job 各删掉 checkout 构建器仓库 + setup-python + pip + PYTHONPATH 四步，改为一步「`gh release download` 下载自包含引擎包 `portable-builder-{linux,windows}-x64.zip` + sha256 校验 + 解包」；其余命令改成 `& $env:CPB_ENGINE …`。输出名、条件表达式、步骤顺序不动。`python-version` 输入已删除（v2 不再用 Python），子仓库若显式传过要一并去掉。v1（Python 版）保留到最后烧尽期结束 |
| `build-browser.yml`（单浏览器按需） | 同上：删 setup-python，`prepare_build_target.py` 调用换成 `portable-builder prepare-target` |
| `actions/build/action.yml`（composite） | 同步改为下载二进制并转发参数；或直接废弃（子仓库若已内联则删） |
| `update-chrome-plus.yml` | **不动**（Python） |
| `docs-site.yml` | **不动**（Python） |
| 新增 `engine-ci.yml` | PR：ubuntu `cargo fmt --check + clippy -D warnings + test -p portable-builder`（纯逻辑）；windows `cargo test --workspace + cargo build`；tauri 构建只在 release 与手动触发 |
| 新增 `release-engine.yml` | tag `v2*` 触发：windows 构建 `portable-builder.exe` 与 `builder-app` 安装包/目录版，附 sha256，挂本仓库 Release；此即子仓库 v2 的下载源 |

### 8.2 runner 经济账（诚实版）

- 发布构建变慢：tauri 全量冷编 6–12 分钟（windows runner、2x 计费），配
  `Swatinem/rust-cache` 降至 3–5 分钟。现状 PyInstaller 约 2–3 分钟——**builder
  仓库自己的发布 CI 变慢，这是已知代价**；但它低频（tag 触发），换来的是产品线
  每次构建省掉 setup-python + pip（每 job 约 20–40 秒）与整类环境性失败重跑。
- 引擎单测尽量下 ubuntu（1x）：config/versions/ini_overlay/release 渲染/providers
  fixture 全部无 OS 依赖；需要 7z 与 PE 样本的集成测试留 windows。
- **CI 总分钟预期：持平至略降（±10%）**。别把「降本」写进预期，写进预期的是
  「失败重跑类别归零 + 产物可复现」。
- 可选优化（不承诺）：`cargo-xwin` 从 ubuntu 交叉编译引擎二进制，把发布 job
  挪到 1x 计费；M5 之后再评估。

### 8.3 子仓库切换步骤（每个子仓库一次性）

1. `build.yml` 里 `uses: ...portable-browser.yml@v2` + `builder-ref: v2`；
2. 并行烧尽期：v1/v2 各跑一次，产物按 §9 M5 的对比法验收；
3. 切默认后删除旧调用残留。

### 8.4 分发产物

本仓库 Release（`v2*` tag）挂：`portable-builder-x86_64-pc-windows-msvc.exe`
（引擎）、`ChromiumPortableBuilder-windows/`（GUI 目录版，**不提供单文件**——
现行 onedir 决策的理由（子进程派发 + 杀软误报）在新栈同样成立，只是理由里的
「onefile 解压慢」消失了，目录版依旧更稳）、各产物 `.sha256`。

---

## 9. 分阶段计划（M0–M6）

每阶段有硬性验收门禁，不过门不进下一阶段。对照法用的命令一律内联执行，
不落临时脚本。

### M0 脚手架（0.5 周）
- 建 workspace、两个 crate 空壳、`engine-ci.yml` 绿灯、`tauri.conf.json` 通过
  构建校验、`ui/` 放一个静态页能开窗。
- **门禁**：windows runner 上 `cargo build` + tauri dev 冒烟（窗口标题判据）。

### M1 纯逻辑层 + BCJ2 spike（1–1.5 周）
- 移植：versions / config / ini_overlay / github_env / normalize_sha256 /
  human_size / env_name / i18n；BCJ2 spike（§4.5）。
- **门禁**：黄金测试全绿（ini 三层合并逐字节一致；版本比较含畸形输入；BCJ2
  对 Brave 样本逐字节一致，或按预案降级并记录）。

### M2 网络与工具层（1 周）
- providers 四种（含 fixture 回放）、tools.rs（下载/校验/7z 发现/解压）、
  pe.rs（pelite 收口三处手写解析）。
- **门禁**：对 `bin/` 样本，`inspect-package --json` 新旧输出深度相等；
  PE 机器类型/导入表断言与 Python 版一致（含 arm64 样本）。

### M3 引擎主体 + CLI 对照（2–3 周，最长的一段）
- builder / discovery / verify / multi / release + `prepare-target` /
  `resolve-upstream` 子命令。
- **门禁（核心验收）**：对同一批输入（`bin/` 回归样本 + 三渠道真实配置），
  新旧引擎各跑一遍：① `inspect-package --json` 深度相等；② build 产物解包后
  目录树 `git diff --no-index` 为空；③ `verify`（静态）全绿；④
  `render-release` 干跑输出逐字符一致。**禁止用压缩包字节对比**（7z 时间戳
  天然不同），一律解包后比树。三渠道各跑一次完整链路（AGENTS.md 既有纪律）。

### M4 GUI（1.5–2 周）
- 前端抽取（CSS/骨架平移、渲染层 JS 重写）、commands/controller/plan/runner、
  事件泵、对话框与 opener 插件、`--selftest`。
- **门禁**：§7 全部新测试绿——含无头几何断言（照 ui-architecture.md 验收标准）、
  真窗口启动冒烟（标题判据）、错误参数浮出横幅用例、日志批量节流用例。

### M5 CI 切换与烧尽（1 周 + 2 周并行观察）
- `engine-ci.yml` / `release-engine.yml` 上线，打 `v2` tag 出产物；子仓库按
  §8.3 切 v2 并行跑；观察期内 v1 保留。
- **门禁**：连续两周，三子仓库 v1/v2 产物解包对比零差异、Actions 零环境性失败
  重跑；期间出现的差异全部归档原因（预期差异仅限：压缩包时间戳、zip 元数据）。

### M6 清理与文档（0.5 周）
- 删除：`portable_builder/`（Python）、`scripts/wizard.py`、`build_exe.py`、
  `build_all_packages.py`、`upstream/resolve.py`、`prepare_build_target.py`、
  `gui*.py`、`web_ui.py`、`requirements*.txt`、`打包EXE.bat`、`build/pyinstaller/`、
  `tests/`（Python 版，M3–M5 期间保留作对照）。
- 更新：`AGENTS.md`（GUI 三坑章节改写为 §6.4 的四条验收精神；「怎么跑」改为
  cargo 命令）、`README.md` / `README.en.md`（双语）、`docs/DEVELOPMENT.md`
  （接入示例 `@v2`）、`docs/ui-architecture.md`（控件章节从 pywebview 措辞
  改为中性措辞，设计规范本身不变）、`.gitignore`（加 `target/`）。
- **门禁**：全仓 grep 无 `python -m portable_builder` 残留引用；三渠道
  post-switch 构建全绿。

### 工期估算（诚实版）

有 Rust 生产经验的开发者：M0–M6 约 6–9 周全职。Python 主栈、Rust 边学边写：
**2–3 个月起**，且 M3 的对照验收会反复——预算按 3 个月做。学习曲线是本项目
最大的单一成本项，比任何技术风险都大。

---

## 10. 风险登记册

| # | 风险 | 概率 | 影响 | 缓解 |
|---|---|---|---|---|
| R1 | BCJ2/LZMA 移植不对 | 中 | 低（Brave 非产品面） | M1 spike 前置 + 黄金比对 + 预案降级（§4.5） |
| R2 | Rust 曲线拖垮工期 | 高 | 高 | M1/M2 全是机械翻译练手；M3 才碰主体；估算按 3 个月 |
| R3 | CI 契约（GITHUB_ENV 名、env_json、日志前缀）遗漏 | 中 | 高（三子仓库连锁失败） | §1 清单即测试清单：M3 门禁显式断言这些名字；v1 并行烧尽兜底 |
| R4 | setdll/7z 输出解码错（OEM 码页） | 中 | 中 | 编码单测（GBK/437 样本）；解码失败时回落 latin-1 不中断（对齐现状） |
| R5 | tauri 冷编超预算拖慢发布 | 中 | 低 | rust-cache；发布低频；可选 cargo-xwin（§8.2） |
| R6 | WebView2 运行时版本差异（目标机） | 低 | 中 | 依赖面与 pywebview 相同，无回退；冒烟测试覆盖 Win10/Win11 各一 |
| R7 | tauri CSP 生产/开发不一致 | 中 | 中 | 离线本地页统一 `csp: null` + 无外链；无头测试断言零外部请求 |
| R8 | pelite 与手写解析在畸形 PE 上分歧 | 低 | 中 | M2 门禁含 arm64 与畸形样本（截断/零节表）对照 |
| R9 | 依赖锁定漂移（500+ crate 的锁文件维护） | 高 | 低 | dependabot 周更；CI 锁定 `--locked`；不动 minor 也能跑 |

---

## 11. 破坏性变更清单（对齐用户/子仓库）

1. YAML 配置不再支持（JSON/TOML 保留）——实际使用方全为 JSON。
2. 控制台向导 `scripts/wizard.py` 退役；`开始构建.bat` 改启 GUI。GUI 功能覆盖
   向导，WebView2 系统要求不变（Win10 1803+/Win11 自带）。
3. `python -m portable_builder` 消失，变为单文件 `portable-builder.exe`；
   子仓库 workflow 以 `@v2` tag 切换（v1 保留至烧尽期结束）。
4. `upstream/resolve.py` 外部脚本入口消失（吸收为 `resolve-upstream` 子命令）；
   `script` provider 的通用语义不变。
5. GUI 分发形态仍为目录版（不提供单文件 exe），理由见 §8.4。
6. **GUI 工具箱删掉「Chrome++ 更新」按钮**（原六件套 → 五件套）。
   `update_chrome_plus.py` 是维护者 CI 的活（本文件 §0 已定「保留 Python」，
   走 `update-chrome-plus.yml`），Rust 引擎没有对应子命令，
   终端用户也不该在 GUI 里更新仓库内置的 Chrome++。
7. **产物落点提示修好了**：Python 版 `_on_done` 先算 `_artifact_hint()` 再调
   `refresh_plan()`，而后者把 `result_text` 清空——所以界面上从来没显示过
   产物路径。Rust 版把顺序摆正（差异记录见 §13）。

---

## 12. 附录：新旧对照验收操作（M3/M5 用，内联执行）

```powershell
# ① 静态解析对照（同一安装包）
python -m portable_builder --config <cfg> --target <t> --workdir . inspect-package <pkg> --json > old.json
.\target\release\portable-builder.exe --config <cfg> --target <t> --workdir . inspect-package <pkg> --json > new.json
git diff --no-index old.json new.json

# ② 产物树对照（解包后比树，不比压缩包字节）
#    各自 build 后：
7z x build\assets\old.7z -oold_tree
7z x build\assets\new.7z -onew_tree
git diff --no-index --stat old_tree new_tree

# ③ release 渲染对照（干跑）
python -m portable_builder --config <cfg> --target <t> --workdir . render-release > old_rel.txt
.\target\release\portable-builder.exe --config <cfg> --target <t> --workdir . render-release > new_rel.txt
git diff --no-index old_rel.txt new_rel.txt
```

对照产物（old/new 树与 txt）放 `build/`（已 gitignore），验收完即删，不留目录。
---

## 13. 实施差异清单（相对本文件原计划，落地时改了什么）

1. **GUI 逻辑落在引擎 crate**（`crates/portable-builder/src/gui/`），不是 builder-app。
   理由：这四块不依赖 tauri，放引擎里就能在 ubuntu CI 上跑 GUI 单测
   （`cargo test -p portable-builder --lib gui::` 19 条），builder-app 只剩命令转发。
   引擎「不要 async runtime」的约束不变（runner 用 std::thread + mpsc）。
2. **资源根探测改成向上找**：`CPB_ENGINE_ROOT` → 当前目录向上 5 层的
   `catalog/`、`scripts/`、`crates/` → exe 目录。cargo test 的 cwd 是 crate 目录，
   Python 那套「cwd 或 exe 目录」的二分在 Rust 里会指错。
3. **引擎命令不再有 frozen/source 两套派发**：`gui_plan.child_command` 的
   `--run-cli` / `--run-script` 分支消失，所有步骤都是引擎自身的子命令
   （`prepare_build_target.py` → `prepare-target`，`upstream/resolve.py` → `resolve-upstream`）。
   引擎 exe 定位：`CPB_ENGINE_EXE` → GUI 旁的 `portable-builder(.exe)` → PATH。
4. **计时不再另开 Timer 线程**：事件泵按 80ms，每 12 拍（约 1s）刷新一次
   `elapsed` 并推 state——与旧行为等价，少一个线程。
5. **打开目录不引 opener 插件**：`os.startfile` 的等价物是
   `cmd /C start "" <path>`（macOS `open`、其余 `xdg-open`），少一个依赖。
6. **发布产物是自包含 zip**（引擎 exe + catalog + locales + setdll + 7zr.exe），
   不是单个 exe：子仓库因此不需要再 checkout 构建器仓库去拿 setdll。
   Windows 与 Linux 各一个（check job 在 ubuntu 上跑，需要原生二进制）。
7. **前端仍在补齐**（M4 收尾）：`app.js` 的渲染函数由 `web_ui.py` 的
   `_topbar/_build_page/_tools_page/_deck` 平移而来，工具箱五件套。
8. **移交项存档**（原 `_migration/wave2-briefs/carry-forward-items.md`，目录已清）：
   已修复：#1/#2（formatter+黄金再生纪律，生效中）、#8（总时限下载，Wave2 修复）、
   #9（digest panic 路径）、#11（provider 分发，Wave3 接线）、#12（测试临时目录清理）。
   已知可接受：#3（versions 全角数字 unreachable）、#6（CRLF 归一，对照门禁规则）、
   #7（prepare-target 的 target 键注入，黄金已再生）、#10（pelite 对损坏 PE 的行为差异，
   有效 PE 7/7 黄金恒等）。#4/#5（done 事件富化、log tag 语义）已在 M4 落地。
