# 便携版下载站（GitHub Pages）

`https://piracola.github.io/ChromiumPortable/` 的源文件。静态站点，不依赖服务器或前端构建工具链。

## 部署

1. 把 `ChromiumPortable/docs/` 提交到仓库默认分支。
2. 仓库设置 → `Pages` → Source 选 `Deploy from a branch`。
3. Branch 选默认分支，目录选 `/docs`，保存后等待 GitHub 生成站点。

## 结构

```
docs/
├── data/site.json        # 单一数据源：文案、构建目标、仓库、FAQ
├── data/releases.json    # 由脚本抓取的各仓库最新 Release（版本 / 体积 / 日期 / SHA256）
├── tools/generate.py     # 渲染器：产出下面所有 HTML 与 sitemap.xml
├── tools/make_og.py      # 生成 assets/og.png 社交预览图（本地手动跑）
├── index.html            # ← 生成产物，不要手改
├── chrome|edge|helium/   # ← 生成产物，各浏览器的落地页
├── sitemap.xml           # ← 生成产物
├── styles.css            # 手写
└── app.js                # 手写，仅深色模式切换与卡片筛选
```

**`index.html` 和三个落地页都是生成产物。**改内容请改 `data/site.json`，然后重新生成：

```powershell
cd docs
python tools/generate.py            # 重新生成全部页面
python tools/generate.py --check    # 只校验产物是否与数据源同步（CI 用）
```

`--check` 由 `.github/workflows/docs-site.yml` 在每次改动 `docs/` 时执行，手改产物会导致 CI 失败。
页面内容全部预渲染成静态 HTML，`app.js` 只负责交互，禁用 JavaScript 时页面依然完整可读。

## 版本号从哪来

页面上的版本、体积、更新日期和下载直链来自 `data/releases.json`，由生成器按各构建目标的
`assetPattern` 从 GitHub Releases API 匹配得到：

```powershell
$env:GITHUB_TOKEN="<token>"          # 可选，避开未认证的 60 次/小时限额
python tools/generate.py --fetch-releases
```

同一个工作流每天定时跑一次并自动提交，所以页面版本号会自己跟上，`sitemap.xml` 的 `lastmod`
也取自这份数据。抓取失败时保留上一次的结果，不会把页面清空。

## 新增一个浏览器

1. 在 `data/site.json` 的 `repos`、`builds`、`pages` 三处各加一项（`assetPattern` 必须含
   `(?P<version>...)` 分组，且不能和已有目标交叉命中）。
2. 把图标放进 `assets/`。
3. `python tools/generate.py --fetch-releases` 重新生成，落地页、结构化数据、导航和 sitemap 会自动跟上。

## 一次性的收录工作（需要人工操作）

站点是 `github.io` 项目页，没有外链就不会被抓取。以下几步只做一次：

- [ ] 四个仓库的 About → Website 字段填站点地址（Chrome-Portable / Edge_Portable / Helium_Portable / ChromiumPortable）
- [ ] [Google Search Console](https://search.google.com/search-console) 添加资源，用 HTML 文件方式验证
      （把验证文件放进 `docs/`，它会随 Pages 一起发布），提交 `sitemap.xml`
- [ ] [Bing 站长工具](https://www.bing.com/webmasters) 同样验证并提交 sitemap（DuckDuckGo 与多个 AI 搜索取用 Bing 索引）

关于百度：`github.io` 长期不被百度有效收录，中文流量基本进不来。要覆盖百度只能绑定自定义域名，
当前没有这个计划，故不做百度站长验证。

## 图标来源

- `assets/chrome.svg`: Wikimedia Commons, Google Chrome icon (February 2022)
- `assets/edge.svg`: Wikimedia Commons, Microsoft Edge logo (2019)
- `assets/helium.svg`: `imputnet/helium` 上游仓库的 `product_logo.svg`
