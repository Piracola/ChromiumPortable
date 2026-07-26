# ChromiumPortable

ChromiumPortable 是 Chromium 系浏览器便携版的可复用构建核心，用来统一处理上游版本检查、安装包下载、解压、`chrome++` 集成、DLL 注入、打包和 GitHub Release 发布。

它本身不发布浏览器成品；成品由各个子仓库根据自己的上游浏览器配置自动构建。

> ⚠️ 本仓库**不发布浏览器成品**，只提供可复用的构建流程。如需下载便携版浏览器，请前往：
> - [Chrome-Portable](https://github.com/Piracola/Chrome-Portable)（Google Chrome，Stable / Beta）
> - [Edge_Portable](https://github.com/betacola/Edge_Portable)（Microsoft Edge，Stable）
> - [Helium_Portable](https://github.com/Piracola/Helium_Portable)（Helium，Stable / Preview）
>
> 想了解构建系统或新增浏览器支持？继续往下读。

## 仓库导航

本仓库为构建核心，以下子仓库各自配置上游浏览器并调用本仓库的 reusable workflow：

| 子仓库 | 浏览器 | 渠道 |
| --- | --- | --- |
| [Chrome-Portable](https://github.com/Piracola/Chrome-Portable) | Google Chrome | Stable / Beta |
| [Edge_Portable](https://github.com/betacola/Edge_Portable) | Microsoft Edge | Stable |
| [Helium_Portable](https://github.com/Piracola/Helium_Portable) | Helium | Stable / Preview |

后续新增浏览器时，优先新建子仓库并引用本仓库的 reusable workflow，而不是复制整套构建脚本。

## 本仓库的用途

- 复用便携化构建流程。
- 降低新增浏览器支持时的维护成本。
- 让每个子仓库只维护 `browser.json`、`chrome++` 配置和项目说明。
- 统一 GitHub Actions 自动检查、构建、打包和发行流程。

`docs/` 目录是一个 GitHub Pages 静态展示站，汇总各子仓库和构建目标的入口链接（数据手工维护在 `docs/site-data.js`，不展示实时版本号），部署方式见 [docs/README.md](./docs/README.md)。

## 快速开始：接入新子仓库

子仓库的 `.github/workflows/build.yml` 可以引用本仓库：

```yaml
jobs:
  portable:
    permissions:
      contents: write
    uses: Piracola/ChromiumPortable/.github/workflows/portable-browser.yml@v1.1
    with:
      builder-repository: Piracola/ChromiumPortable
      builder-ref: v1.1
      config: browser.json
      target: edge_stable
```

> `uses` 和 `builder-ref` 里的 `v1.1` 是本仓库的发布 tag。新接入的子仓库建议固定引用某个 tag；core 发新版本时打新 tag（如 `v1.2`）再让子仓库迁移。详见 [版本与发布](#版本与发布)。

reusable workflow 内部分成两个 job：`check` 跑在 `ubuntu-latest`（上游版本查询只是几次 HTTP 请求，不需要 Windows），`build` 跑在 `windows-latest` 且 `needs: check`。定时触发时只有 `check` 判定需要更新才会启动 Windows runner。两个 job 之间通过 `check` 的 `env_json` output 传递全部环境变量（含按 target 动态生成的键），`build` 的第一步把它还原进 `GITHUB_ENV`。

浏览器差异写在子仓库的 `browser.json`。示例见 [examples](./examples)。

## 保留子仓库下载脚本

如果某个浏览器的上游查询逻辑比较特殊，可以把下载地址获取脚本留在子仓库，只让本仓库复用后续的解压、注入、打包和发版逻辑。

`browser.json` 使用 `script` provider：

```json
{
  "provider": {
    "type": "script",
    "command": ["python", "scripts/chrome_package.py", "--channel", "stable"]
  }
}
```

子仓库脚本向 stdout 输出 JSON：

```json
{
  "version": "123.0.0.0",
  "url": "https://dl.google.com/example/chrome_installer.exe",
  "file_name": "chrome_installer.exe",
  "sha256": "ebd1e560964b89aa28e5841d3b380fd09433dfe9e2373dde41cd4a0c3d945965",
  "size": 489274792,
  "verify_ssl": true
}
```

`sha256` 与 `size` 可选但强烈建议提供：构建器会在下载时逐块计算摘要并比对，不一致就删除文件并中止构建。三个上游都直接提供了摘要（Chrome 的 Omaha 响应带 `hash_sha256`，Edge 的 CDP API 带 `Hashes.Sha256`，GitHub Release 资产带 `digest`），所以现有子仓库脚本都强制要求拿到摘要才输出结果。`sha256` 接受十六进制、`sha256:<hex>` 前缀和 base64 三种写法。

也可以由子仓库脚本自己下载好安装包，然后返回本地路径：

```json
{
  "version": "123.0.0.0",
  "installer_path": "downloads/chrome_installer.exe",
  "sha256": "..."
}
```

这样 Chrome、Edge 或其他浏览器的上游变动只需要改各自子仓库里的脚本，主仓库继续保持通用。

## 维护者指南

通用构建逻辑位于 [portable_builder](./portable_builder)。新增浏览器时，先尝试通过 `direct`、`google_omaha` 或 `microsoft_edge` provider 配置完成；如果上游版本 API 或安装包结构不同，再新增 `portable_builder/providers/*.py`。

本地测试（Windows + Python 3）：

```powershell
python -m compileall portable_builder
$env:PYTHONPATH="<path-to-ChromiumPortable>"
python -m portable_builder --config examples\edge.browser.json --target edge_stable --workdir . check
```

Chrome 这类一个仓库同时发布多个渠道的项目，可以直接 checkout 本仓库后调用 `check-targets`、`build-targets`、`verify-targets`、`render-release-targets` 和 `update-release-targets`。

多目标共享一个 GitHub Release 时，构建器会优先用每个 target 的 `archive_name` 精确识别和清理对应资产，并在创建新的 shared release 时自动继承未更新 target 的旧资产，避免“发行说明里有版本号但附件缺包”。

如果显式配置 `release.asset_match`，请确保不同 target 之间不会交叉匹配。默认推荐让每个 target 使用可区分的 `archive_name`，由构建器自动按归档命名推导资产匹配规则。

构建器会优先使用系统 7-Zip 或 PATH 中的 `7z`；如果都不可用，会尝试下载独立解压器，下载失败后再尝试通过 Chocolatey 安装 7-Zip。

### 产物验证

构建阶段会在 DLL 注入后解析浏览器可执行文件的 PE 导入表，断言**第一条导入项**就是我们注入的 `version.dll`，并且它是相对路径。

这里只检查「存在 `version.dll` 导入」是不够的：Chromium 本身就原生导入系统的 `VERSION.dll`，所以那种检查在完全没注入的情况下也会通过，产出一个没有 Chrome++ 功能的普通浏览器。setdll（Detours）会把注入的 DLL 前置，使加载器优先解析它，因此「第一条导入项」才是真正的判据。绝对路径检查同时保留，避免把构建机的路径写进产物导致换机器后找不到 `version.dll`。

打包后还可以用 `verify` / `verify-targets` 验证最终交付物：解压 `.7z`，重新检查导入表，确认没有 setdll 残留的 `<exe>~` 备份，然后真正启动一次浏览器（`--version` 与 headless），断言 Chrome++ 把配置目录建在了解压目录内而不是用户目录——这是「便携」这一承诺唯一的自动化证据。

```powershell
python -m portable_builder --config browser.json --target chrome_stable --workdir . verify
python -m portable_builder --config browser.json --target chrome_stable --workdir . verify --no-smoke  # 只查导入表，不启动浏览器
```

可用 target 配置项微调：`smoke_args`（默认 headless + `--dump-dom about:blank`）、`smoke_data_dir`（默认 `Data`）、`smoke_timeout`。

### 发行说明可用的占位符

单 target（`release.body`）：

| 占位符 | 含义 |
| --- | --- |
| `{version}` / `{package_version}` | 浏览器版本 |
| `{display_name}` / `{name}` / `{output_dir}` / `{arch}` | target 基本信息 |
| `{archive}` / `{size}` / `{sha256}` | 归档文件名、大小、SHA256 |
| `{date}` | 构建日期 |
| `{chrome_plus_version}` | 打包进去的 Chrome++ 版本（来自 `setdll/version.txt`） |
| `{run_url}` | 本次 GitHub Actions 运行记录链接 |

多 target（顶层 `release.body`）把上面的按 target 名加前缀，例如 `{chrome_stable_version}`、`{chrome_beta_sha256}`，另外仍可用 `{date}`、`{chrome_plus_version}`、`{run_url}`。

某个渠道本次没有重新构建时，它的 `_archive` / `_sha256` / `_size` 会退化为读取 GitHub 上现有资产的摘要；连这个也拿不到时渲染为 `-`。

> ⚠️ **发行说明正文实际上是一份 schema。** `check` 是靠 `version_pattern` 正则从 Release 正文里抠出当前已发布的版本号的。改了文案却没同步改正则，正则就静默匹配不到，`check` 认为「还没发布过」，于是**之后每天都会无条件重新构建**，而且不会报任何错。
>
> 因此 `render-release` / `render-release-targets` 会在渲染完成后立刻用 `version_pattern` 把版本号**回读一遍**，对不上就直接让构建失败。改正文时请保留 `<渠道> 版本: <版本号>` 这样的锚点行，或者在同一次提交里改正则——两种情况这个自检都会兜住。

## chrome++ 自动更新

主仓库通过 `.github/workflows/update-chrome-plus.yml` 定时检查 [Bush2021/chrome_plus](https://github.com/Bush2021/chrome_plus/releases) 的最新 `setdll.7z`，只把其中的 `version-x64.dll`、`setdll-x64.exe`、`README.md` 和 `chrome++.ini` 更新到本仓库的 `setdll/` 目录，并把上游版本号记进 `setdll/version.txt`（发行说明里的 `{chrome_plus_version}` 就来自这里）。

### chrome++.ini 的三层结构

`chrome++.ini` 有 166 行，其中绝大部分是中英双语注释，真正因浏览器而异的只有几个键。所以它由三层合并而成：

| 层 | 文件 | 谁维护 |
| --- | --- | --- |
| 基线 | `setdll/chrome++.ini` | 上游，每次同步整份覆盖，**不要手改** |
| 通用默认 | `setdll/chrome++.defaults.ini` | 本项目，所有浏览器共用的偏好 |
| 浏览器差异 | 子仓库 `chrome++/chrome++.override.ini` | 各子仓库，只写自己不一样的键 |

合并是按行进行的，**注释和键的顺序全部保留**，所以上游新增的配置项会自动出现在最终产物里，不需要手工同步到每个子仓库。

`chrome++.defaults.ini` 故意不在同步流程的文件列表里，因此不会被上游覆盖。

覆盖一个基线里不存在的键会**直接让构建失败**：chrome++ 运行时会静默忽略不认识的键，拼错了就永远发现不了，所以这里选择尽早报错。

子仓库如果仍然放一份完整的 `chrome++.ini`，为兼容旧配置它依然生效，但会遮蔽自动同步的基线，构建时会给出警告。

如果 chrome++ 文件发生变化，workflow 会先提交主仓库更新；只有 `version-x64.dll` 或 `setdll-x64.exe` 变化时，才会调度子仓库的 `workflow_dispatch` 构建。调度时会把这次主仓库更新提交作为 `builder_ref` 传给子仓库，保证子仓库构建使用刚更新的 chrome++ 文件。子仓库的手动触发构建会强制重打包；在浏览器版本没有变化时，会更新现有 GitHub Release 并替换对应附件。

跨仓库调度需要在主仓库配置 `CHILD_REPO_TOKEN` secret。这个 token 需要能访问并触发子仓库 Actions workflow，例如细粒度 token 授权目标子仓库的 `Actions: Read and write` 和 `Contents: Read`。

### 版本与发布

发布稳定版本时给本仓库打 tag，例如 `v1.1`。core 发新版本时打新 tag（如 `v1.2`），再让子仓库迁移 `builder-ref`。

现状：本仓库已有 `v1`、`v1.1`、`v1.2` 三个 tag，但 Chrome、Edge、Helium 三个子仓库目前**都固定引用 `main`**，即 core 的任何一次提交都会立即影响三条生产线。若要恢复 tag pin 的隔离效果，需要逐个修改子仓库 `build.yml` 里的 `builder-ref`（Edge 还需同时修改 `uses` 的 ref）。

## 许可证

本项目源码遵循 MIT 许可证。
