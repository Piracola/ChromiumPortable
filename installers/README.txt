把要构建的浏览器安装包放在这里
Put installer packages for portable builds in this folder
=========================================================

支持的扩展名 / Supported types:
  .exe  .msi  .7z  .zip  .rar  .cab

使用方法 / How to use
1. 从浏览器官网或你已合法取得的渠道下载安装包，复制到本目录。
2. 双击仓库根目录的「开始构建.bat」。
3. 选择「构建单个安装包」或「构建文件夹里的全部安装包」。

说明 / Notes
- 只做解包与 Chrome++ 注入，不会安装或启动浏览器。
- 本目录下的安装包不会被提交到 Git（已在 .gitignore 忽略）。
- 构建结果在 build\release\，打包后在 build\assets\。
- 仅供学习与自用；再分发前请阅读上游协议（见仓库 NOTICE）。
