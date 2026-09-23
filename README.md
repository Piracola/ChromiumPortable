<div align="center">

# ChromiumPortable

为 Chromium 系浏览器制作便携版的公共构建核心

[![核心版本][badge-version]][link-tags]
[![chrome++ 同步][badge-sync]][link-sync]
[![许可证][badge-license]][link-license]
[![Stars][badge-stars]][link-repo]
[![最近提交][badge-last-commit]][link-repo]

**语言 / Language:** **简体中文** · [English](README.en.md)

[下载页面](https://piracola.github.io/ChromiumPortable/) · [使用帮助](https://piracola.github.io/ChromiumPortable/) · [开发文档](./docs/DEVELOPMENT.md)

</div>

本项目用于自动制作集成 Chrome++ 的便携浏览器。它本身不提供浏览器安装程序，普通用户请从下方项目或统一下载页面获取已经构建好的版本。

## 当前项目

以下三个渠道有独立发布仓库、持续构建与下载支持。

| 项目 | 仓库 | 最新版本 | 发布 | 累计下载 | Star |
| --- | --- | --- | --- | --- | --- |
| **Google Chrome 便携版** | [Chrome-Portable][link-chrome] | [![][badge-chrome-release]][link-chrome] | 2026-09-09 | [![][badge-chrome-downloads]][link-chrome] | [![][badge-chrome-stars]][link-chrome] |
| **Microsoft Edge 便携版** | [Edge_Portable][link-edge] | [![][badge-edge-release]][link-edge] | 2026-09-11 | [![][badge-edge-downloads]][link-edge] | [![][badge-edge-stars]][link-edge] |
| **Helium 便携版** | [Helium_Portable][link-helium] | [![][badge-helium-release]][link-helium] | 2026-09-21 | [![][badge-helium-downloads]][link-helium] | [![][badge-helium-stars]][link-helium] |

**渠道说明**

- **Chrome**：`stable` / `beta` 两线；官方 Omaha 取包；产物 `Chrome++_stable_*` / `Chrome++_beta_*`。
- **Edge**：稳定线；微软 CDP API + 安装器源；产物 `Edge_Portable_Win64_*`。
- **Helium**：稳定线；auto 结构解析；产物 `Helium_*`。官方产品构建在子仓库；本核心 `build-browser` 若上游未挂公开安装包资产，需提供 `package_url`。

更多下载、版本信息和文件校验值请访问：[便携版下载页面](https://piracola.github.io/ChromiumPortable/)。

## 使用方法

1. 下载所需浏览器的压缩包。
2. 解压到任意可写目录。
3. 运行浏览器主程序，或双击压缩包中提供的 `开始.bat`。

浏览数据通常保存在浏览器文件夹旁边的 `Data` 和 `Cache` 目录中。更新时只需替换浏览器程序目录，请勿删除自己的数据目录。

## 其他 Chromium 系浏览器（自用构建）

本工具也能为 Brave、Vivaldi、Opera、Thorium、360 极速浏览器 X 等制作便携版。  
这些**不是**本项目产品：无下载站产品页，构建结果仅供学习与自用；再分发请先阅读上游协议（见 [NOTICE](./NOTICE)）。

**本地一键（推荐）**

1. 安装 [Python 3](https://www.python.org/)（安装时勾选 “Add to PATH”）。
2. 把下载好的浏览器安装包复制到专用目录 **`installers\`**（里面有一份 `README.txt` 说明）。
3. 双击仓库根目录的 **`开始构建.bat`**，或在终端运行：

```powershell
python scripts\wizard.py
```

4. 按菜单选择：构建单个安装包 / 构建整个文件夹 / 在线下载最新版，再选架构与是否打包即可。  
   完成后在 `build\release\` 查看便携目录；勾选打包时在 `build\assets\` 查看 7z。

界面语言按系统自动切换（简体中文 / English）；也可用环境变量 `WIZARD_LANG=en` 强制指定。

支持的在线浏览器、CI 发布流程与高级命令行，见 [开发文档](./docs/DEVELOPMENT.md)。

## 注意事项

- 当前发布版本主要面向 Windows x64。
- 便携化过程会修改浏览器主程序并加载 Chrome++，原始数字签名可能因此失效，部分安全软件可能产生误报。
- 请只从上方项目的 GitHub Releases 或统一下载页面获取产品渠道文件，并在需要时核对 SHA256。其他浏览器的 `Unofficial` 构建不代表官方立场。
- 使用前建议备份重要的浏览器数据。
- 商标、第三方组件与再分发边界见 [NOTICE](./NOTICE)。

## 参与开发

本仓库是构建核心，不是浏览器成品仓库。新增浏览器、配置构建流程或维护发布任务，请阅读 [开发与接入文档](./docs/DEVELOPMENT.md)。下载站维护说明见 [docs/README.md](./docs/README.md)。

## 许可证

项目源码使用 [MIT License](./LICENSE)。Chrome++、浏览器本体及其他第三方组件遵循各自的许可证；商标归属见 [NOTICE](./NOTICE)。

---

<div align="center">

<sub>Built and maintained by</sub>

**Piracola**

</div>

<!-- 徽标：中文标签需 percent-encode，否则 shields.io 无法解析。 -->
[badge-version]: https://img.shields.io/github/v/tag/Piracola/ChromiumPortable?style=flat-square&color=2f81f7&label=%E6%A0%B8%E5%BF%83%E7%89%88%E6%9C%AC
[badge-sync]: https://img.shields.io/github/actions/workflow/status/Piracola/ChromiumPortable/update-chrome-plus.yml?branch=main&style=flat-square&color=2ea043&label=chrome%2B%2B%20%E5%90%8C%E6%AD%A5
[badge-license]: https://img.shields.io/github/license/Piracola/ChromiumPortable?style=flat-square&color=6e7681&label=%E8%AE%B8%E5%8F%AF%E8%AF%81
[badge-stars]: https://img.shields.io/github/stars/Piracola/ChromiumPortable?style=flat-square&color=d8653f&label=Stars
[badge-last-commit]: https://img.shields.io/github/last-commit/Piracola/ChromiumPortable?style=flat-square&color=555555&label=%E6%9C%80%E8%BF%91%E6%8F%90%E4%BA%A4

[link-tags]: https://github.com/Piracola/ChromiumPortable/tags
[link-sync]: https://github.com/Piracola/ChromiumPortable/actions/workflows/update-chrome-plus.yml
[link-license]: https://github.com/Piracola/ChromiumPortable/blob/main/LICENSE
[link-repo]: https://github.com/Piracola/ChromiumPortable

[badge-chrome-release]: https://img.shields.io/github/v/release/Piracola/Chrome-Portable?display_name=tag&style=flat-square&color=d8653f&label=
[badge-chrome-downloads]: https://img.shields.io/github/downloads/Piracola/Chrome-Portable/total?style=flat-square&color=2ea043&label=
[badge-chrome-stars]: https://img.shields.io/github/stars/Piracola/Chrome-Portable?style=flat-square&color=2f81f7&label=
[link-chrome]: https://github.com/Piracola/Chrome-Portable/releases/latest

[badge-edge-release]: https://img.shields.io/github/v/release/betacola/Edge_Portable?display_name=tag&style=flat-square&color=1d7c84&label=
[badge-edge-downloads]: https://img.shields.io/github/downloads/betacola/Edge_Portable/total?style=flat-square&color=2ea043&label=
[badge-edge-stars]: https://img.shields.io/github/stars/betacola/Edge_Portable?style=flat-square&color=2f81f7&label=
[link-edge]: https://github.com/betacola/Edge_Portable/releases/latest

[badge-helium-release]: https://img.shields.io/github/v/release/Piracola/Helium_Portable?display_name=tag&style=flat-square&color=5b5bd6&label=
[badge-helium-downloads]: https://img.shields.io/github/downloads/Piracola/Helium_Portable/total?style=flat-square&color=2ea043&label=
[badge-helium-stars]: https://img.shields.io/github/stars/Piracola/Helium_Portable?style=flat-square&color=2f81f7&label=
[link-helium]: https://github.com/Piracola/Helium_Portable/releases/latest
