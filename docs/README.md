# Chromium Portable Builder Showcase

这个目录是 `ChromiumPortable` 构建脚本仓库的 GitHub Pages 静态展示站，不依赖服务器或构建步骤。

## 部署

1. 把 `ChromiumPortable/docs/` 提交到仓库默认分支。
2. 在 `ChromiumPortable` 仓库设置中打开 `Pages`。
3. Source 选择 `Deploy from a branch`。
4. Branch 选择默认分支，目录选择 `/docs`。
5. 保存后等待 GitHub 生成站点。

## 维护

- 页面结构在 `index.html`
- 样式在 `styles.css`
- 渲染逻辑在 `app.js`
- 构建版数据在 `site-data.js`
- 浏览器图标在 `assets/`

后续如果新增子项目或新增频道，优先修改 `site-data.js`，一般不需要改页面结构。

## 图标来源

- `assets/chrome.svg`: Wikimedia Commons, Google Chrome icon (February 2022)
- `assets/edge.svg`: Wikimedia Commons, Microsoft Edge logo (2019)
- `assets/helium.svg`: `imputnet/helium` 上游仓库的 `product_logo.svg`
