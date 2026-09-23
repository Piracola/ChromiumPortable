# 开发与接入文档

本文面向 ChromiumPortable 的维护者和希望接入新浏览器的开发者。普通用户请阅读仓库根目录的 [README](../README.md) 或访问 [下载页面](https://piracola.github.io/ChromiumPortable/)。

## 项目职责

ChromiumPortable 统一处理上游版本检查、安装包下载、解压、Chrome++ 配置合并、DLL 注入、打包、成品验证和 GitHub Release 更新。

### 产品面 / 产物面政策（必读）

- **产品面**（README「当前项目」、下载站产品页、品牌化措辞）只覆盖 **Chrome、Edge、Helium** 三个已有发布仓库。
- **产物面**（GitHub Actions → Release 资产）可以覆盖一切能构建的 Chromium 系浏览器：能取到最新安装包、auto 解析成功，就可以出包并挂 Release。
- 非产品浏览器的 Release **必须**使用 `Unofficial` 标题前缀与免责声明正文（见 `catalog/browser_catalog.json`），**不得**在下载站做成产品卡片或使用商标 logo。
- 根因是降低「产品化 / 商标观感」风险，而不是宣称再分发已被上游许可允许。再分发判断见 [NOTICE](../NOTICE)。

浏览器成品由独立子仓库维护。每个子仓库主要保存 `browser.json`、必要的上游查询脚本、Chrome++ 覆盖配置和启动脚本，并通过 reusable workflow 调用本仓库。

当前生产项目：

- [Chrome-Portable](https://github.com/Piracola/Chrome-Portable)
- [Edge_Portable](https://github.com/betacola/Edge_Portable)
- [Helium_Portable](https://github.com/Piracola/Helium_Portable)

## 接入子仓库

子仓库可以通过以下 workflow 调用构建核心：

```yaml
jobs:
  portable:
    permissions:
      contents: write
    uses: Piracola/ChromiumPortable/.github/workflows/portable-browser.yml@v1.2
    with:
      builder-repository: Piracola/ChromiumPortable
      builder-ref: v1.2
      config: browser.json
      target: browser_stable
```

建议固定使用正式 tag，确认兼容后再升级 core 版本，避免主分支上的改动直接影响生产构建。

workflow 的版本检查运行在 Linux，只有需要重新构建时才启动 Windows runner。Windows job 会依次执行构建、打包、成品验证和 Release 更新。

## browser.json

配置支持 JSON、TOML 和 YAML。常用字段可参考 [examples](../examples/)：

- `name`、`display_name`、`output_dir`：项目及产物名称。
- `architecture`：目标架构，目前核心自带的 Chrome++ 文件为 x64。
- `provider`：上游版本和安装包来源。
- `version_root`、`inner_archive`、`layout`：安装包解压结构。
- `exe_name`：相对于版本目录的浏览器主程序路径，也可以包含 `..`。
- `ini_location`、`version_dll_location`：Chrome++ 配置和 DLL 的放置位置。
- `archive_name`：最终压缩包名称。
- `release`：Release 标题、正文、版本提取和资产匹配规则。

已有 Chrome、Edge、Helium 等 target 继续使用显式结构，行为不会改变。新浏览器可把 `layout` 设为 `auto`，省略 `version_root`、`inner_archive` 和 `exe_name`。自动模式会递归检查解包内容，结合文件位置、版本信息、架构、Chromium 核心文件和导入项选择主程序；遇到多个接近的候选会直接失败，不会猜测。

自动模式支持主程序位于版本目录外层、位于版本目录内、或所有文件平铺在同一层的情况。它会排除安装器、更新器、崩溃报告、辅助程序、驱动和代理程序，也不会按 EXE 文件大小选择。`direct` provider 的 `path` 可以指向本地安装包，也可以指向已解压的目录。

最小配置示例：

```json
{
  "targets": {
    "browser_stable": {
      "name": "Browser",
      "output_dir": "Browser",
      "architecture": "x64",
      "layout": "auto",
      "provider": {
        "type": "script",
        "command": "python scripts/get_browser_package.py"
      }
    }
  }
}
```

## Provider

核心内置 `direct`、`google_omaha`、`microsoft_edge` 和 `script` provider。特殊浏览器推荐使用 `script`，由子仓库脚本查询版本和下载地址。

脚本向标准输出返回 JSON：

```json
{
  "version": "123.0.0.0",
  "url": "https://example.com/browser.exe",
  "file_name": "browser.exe",
  "sha256": "...",
  "size": 123456789,
  "verify_ssl": true
}
```

脚本也可以自行下载文件，然后返回 `installer_path` 或 `path`。建议始终提供 SHA256 和文件大小，构建器会在使用前校验。`layout: "auto"` 时，`path` 也可以是已经解压的目录。

## 单浏览器 GitHub Actions

本仓库自带 [`.github/workflows/build-browser.yml`](../.github/workflows/build-browser.yml)：

- `workflow_dispatch` 选择 `catalog/browser_catalog.json` 中的一个 target；
- 自动从上游取最新安装包（或用 `package_url` / `package_path` 覆盖）；
- `layout: auto` 解包、定位主程序、注入 Chrome++；
- 可选发布到本仓库 GitHub Release（默认开）；`verify_smoke` 默认关，只做静态 PE 校验。

```text
Actions → Build single browser → Run workflow
  browser = brave_stable
  publish_release = true
```

fork 用户无需额外 secret 即可对公开上游使用；请自行评估再分发责任（见 NOTICE）。

### 在线 target（catalog）

`catalog/browser_catalog.json` 中非产品 target（`Unofficial` 发布用）：

| target | 浏览器 | 自动取包 |
| --- | --- | --- |
| `brave_stable` | Brave | CDN / Omaha |
| `vivaldi_stable` | Vivaldi | 官网下载页 CDN |
| `opera_stable` | Opera | FTP 版本目录 + sha256sum |
| `thorium_stable` | Thorium | GitHub（gz83/thorium） |
| `cse360_stable` | 360 极速浏览器 X | `browser.360.cn/browser_download_link.js`（仅 csex） |
| `helium_stable` | Helium | 可能需 `package_url` |

本地向导：`python scripts\wizard.py` 或双击 `开始构建.bat`（交互选安装包/在线下载/架构/是否打包）。
安装包请放入专用目录 `installers\`（向导默认识别该目录；`bin\` 仍是开发用回归样本，不是用户投放目录）。
GitHub 系解析（Thorium / Helium）建议设置 `GITHUB_TOKEN`，避免未认证 API 限流；Helium 常无公开 Windows 资产，请自备安装包。
批量：`python scripts\build_all_packages.py <目录> [--archive]`。
单包：`python -m portable_builder --workdir . build-package <安装包> [--archive]`。

### 资产命名规范

| 类型 | Release 标题 | 资产名 | 正文 |
| --- | --- | --- | --- |
| 产品（三渠道） | 可用产品名 | 子仓库既有 `archive_name` | 产品说明 + 校验 |
| 非产品 | **必须** `Unofficial {display_name} {version}` | `{Name}_Portable_{version}_{date}.7z` | **必须**含 Disclaimer（不是官方发行、无隶属关系、EULA 自审） |

tag 对非产品使用 `unofficial-{key}-v{version}`，避免与产品 tag 混淆。

## 本地运行

需要 Windows 和 Python 3。分析安装包、构建自安装包或打包成品时，还需要已有的 7-Zip：

```powershell
python -m pip install -r requirements.txt
python -m compileall portable_builder
$env:PYTHONPATH="<ChromiumPortable 路径>"
python -m portable_builder --config examples\edge.browser.json --target edge_stable --workdir . check
```

主要命令包括 `check`、`build`、`archive`、`verify`、`render-release` 和 `update-release`。多目标项目可以使用对应的 `*-targets` 命令。

自动模式还提供三个本地命令：

```powershell
# 只分析一个安装包或已解压目录，不注入、不启动任何程序
python -m portable_builder --workdir . inspect-package bin\BrowserSetup.exe

# 批量分析样本目录，不需要逐个指定浏览器
python -m portable_builder --workdir . research-packages bin

# 自动解包、定位主程序、注入 Chrome++；加 --archive 会打包并进行静态验包
python -m portable_builder --workdir . build-package bin\BrowserSetup.exe --archive

# 一套脚本处理目录下全部安装包（同一 auto 流程适配不同结构）
python -m portable_builder --workdir . build-packages bin
# 或
python scripts\build_all_packages.py bin

# 从 catalog 生成单 target 配置（CI / 按需构建同一路径）
python scripts\prepare_build_target.py --browser brave_stable --output build\selected.browser.json
```

`inspect-package` 和 `research-packages` 只读取、解包和检查文件，不运行安装包或浏览器。安装包分析只使用已有的 7-Zip/`7zr.exe`，找不到时会停止，不会下载工具或安装系统组件；直接传入已经解压的目录则不需要 7-Zip。`build-package` 只会调用项目自带的 Chrome++ 注入工具修改工作目录中的副本；同样不会运行安装包或浏览器。

## Chrome++ 配置

最终的 `chrome++.ini` 由三层组成：

1. `setdll/chrome++.ini`：从 Chrome++ 上游同步的完整基线。
2. `setdll/chrome++.defaults.ini`：本项目所有浏览器共用的默认覆盖。
3. 子仓库的 `chrome++/chrome++.override.ini`：浏览器自己的差异配置。

子仓库仍可提供完整 `chrome++.ini`，但它会覆盖上游基线，构建时会显示警告。覆盖文件中的键必须已经存在于基线中，拼写错误会直接导致构建失败。

`scripts/update_chrome_plus.py` 和 `.github/workflows/update-chrome-plus.yml` 负责同步 Chrome++ 文件。目前自动同步 `version-x64.dll`、`setdll-x64.exe`、`README.md` 和 `chrome++.ini`。

## 注入和成品验证

构建器使用 setdll 把 `version.dll` 加入浏览器主程序的 PE 导入表，并使用相对路径引用 DLL。注入后会检查第一条导入项、DLL 路径、setdll 备份和禁止发布的文件。

`verify` 会重新解压最终 7z，再次检查注入结果，并默认启动浏览器执行烟雾测试，确认 Chrome++ 把用户数据写入便携目录。可通过 `smoke_args`、`smoke_data_dir`、`smoke_timeout`、`smoke_data_timeout` 和 `forbidden_file_names` 调整验证。

仅调试时可以使用 `verify --no-smoke` 跳过启动测试，正式发布应保留完整验证。

## Release 和多目标项目

`archive_name` 应在不同 target 之间保持可区分。如果使用 `release.asset_match`，必须避免多个 target 匹配同一个文件。

Release 正文中的版本号会被 `version_pattern` 重新读取，因此正文文案和正则必须一起维护。正文支持版本、日期、压缩包名称、大小、SHA256、Chrome++ 版本和工作流链接等占位符。多目标项目使用带 target 前缀的占位符，例如 `{chrome_stable_version}`。

## 版本管理

构建核心发布新版本时应创建 tag，并让子仓库逐个升级 `uses` 和 `builder-ref`。涉及目录解析、注入或验证的改动，应先用 Chrome、Edge 和 Helium 的实际配置完成回归测试。

智能结构解析器的安装包调研结果见 [常见浏览器安装包结构](./PACKAGE_ANALYSIS.md)。

## 下载站

`docs/` 同时包含 GitHub Pages 下载站。页面内容、生成方式和发布说明见 [docs/README.md](./README.md)。
