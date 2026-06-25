# Chrome Builder Showcase

这个目录是给 GitHub Pages 用的纯静态展示站，不依赖服务器。

## 部署

1. 把 `docs/` 提交到仓库默认分支。
2. 在 GitHub 仓库设置中打开 `Pages`。
3. Source 选择 `Deploy from a branch`。
4. Branch 选择默认分支，目录选择 `/docs`。
5. 保存后等待 GitHub 生成站点。

## 维护

- 页面结构在 `index.html`
- 样式在 `styles.css`
- 渲染逻辑在 `app.js`
- 构建版数据在 `site-data.js`

后续如果新增子项目或新增频道，优先修改 `site-data.js`，一般不需要改页面结构。
