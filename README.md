# ChromiumPortable

ChromiumPortable 是 Chromium 系浏览器便携版的可复用构建核心，用来统一处理上游版本检查、安装包下载、解压、`chrome++` 集成、DLL 注入、打包和 GitHub Release 发布。

它本身不发布浏览器成品；成品由各个子仓库根据自己的上游浏览器配置自动构建。

## 子仓库

- [Chrome-Portable](https://github.com/Piracola/Chrome-Portable)：Google Chrome 便携版，跟随官方 Stable/Beta 更新。
- [Edge_Portable](https://github.com/betacola/Edge_Portable)：Microsoft Edge 便携版，跟随官方 Stable 更新。

后续新增浏览器时，优先新建子仓库并引用本仓库的 reusable workflow，而不是复制整套构建脚本。

## 用途

- 复用便携化构建流程。
- 降低新增浏览器支持时的维护成本。
- 让每个子仓库只维护 `browser.json`、`chrome++` 配置和项目说明。
- 统一 GitHub Actions 自动检查、构建、打包和发行流程。

## 子仓库接入

子仓库的 `.github/workflows/build.yml` 可以引用本仓库：

```yaml
jobs:
  portable:
    permissions:
      contents: write
    uses: Piracola/ChromiumPortable/.github/workflows/portable-browser.yml@v1
    with:
      builder-repository: Piracola/ChromiumPortable
      builder-ref: v1
      config: browser.json
      target: edge_stable
```

浏览器差异写在子仓库的 `browser.json`。示例见 [examples](./examples)。

## 开发指南

通用构建逻辑位于 [portable_builder](./portable_builder)。新增浏览器时，先尝试通过 `direct`、`google_omaha` 或 `microsoft_edge` provider 配置完成；如果上游版本 API 或安装包结构不同，再新增 `portable_builder/providers/*.py`。

本地测试：

```powershell
python -m compileall portable_builder
$env:PYTHONPATH="<path-to-ChromiumPortable>"
python -m portable_builder --config examples\edge.browser.json --target edge_stable --workdir . check
```

发布稳定版本时给本仓库打 tag，例如 `v1`，子仓库固定引用该 tag。
