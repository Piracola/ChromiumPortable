<div align="center">

# ChromiumPortable

为 Chromium 系浏览器制作便携版的公共构建核心

[![核心版本][badge-version]][link-tags]
[![chrome++ 同步][badge-sync]][link-sync]
[![许可证][badge-license]][link-license]
[![Stars][badge-stars]][link-repo]
[![最近提交][badge-last-commit]][link-repo]

**语言 / Language:** **简体中文** · [English](README.en.md)

[下载页面](https://piracola.github.io/ChromiumPortable/) · [使用帮助](https://piracola.github.io/ChromiumPortable/) · [姊妹内核](#姊妹内核) · [开发文档](./docs/DEVELOPMENT.md)

</div>

本项目提供便携版构建引擎与图形构建器，用于制作集成 Chrome++ 的 Chromium 系浏览器便携版。本仓库不发布浏览器成品，三个产品的便携版由下列独立仓库发布。

## 当前项目

以下三个渠道有独立发布仓库、持续构建与下载支持。

| 项目 | 仓库 | 最新版本 | 发布 | 累计下载 | Star |
| --- | --- | --- | --- | --- | --- |
| **Google Chrome 便携版** | [Chrome-Portable][link-chrome] | [![][badge-chrome-release]][link-chrome] | 2026-09-09 | [![][badge-chrome-downloads]][link-chrome] | [![][badge-chrome-stars]][link-chrome] |
| **Microsoft Edge 便携版** | [Edge_Portable][link-edge] | [![][badge-edge-release]][link-edge] | 2026-09-11 | [![][badge-edge-downloads]][link-edge] | [![][badge-edge-stars]][link-edge] |
| **Helium 便携版** | [Helium_Portable][link-helium] | [![][badge-helium-release]][link-helium] | 2026-09-21 | [![][badge-helium-downloads]][link-helium] | [![][badge-helium-stars]][link-helium] |

更多下载、版本信息和文件校验值见 [便携版下载页面](https://piracola.github.io/ChromiumPortable/)。

## 使用方法

1. 下载所需浏览器的压缩包。
2. 解压到任意可写目录。
3. 运行浏览器主程序，或双击压缩包中提供的 `开始.bat`。

浏览数据通常保存在浏览器文件夹旁边的 `Data` 和 `Cache` 目录中。更新时替换浏览器程序目录，数据目录保持不变。

## 图形构建器

[Releases](../../releases) 提供 `ChromiumPortableBuilder-windows-x64.zip`，包含构建引擎与图形界面，目标机不需要 Python。解压后运行 `builder-app.exe`，将浏览器安装包放入 `installers\`，即可在界面中选择来源、架构与输出目录并启动构建。界面提供简体中文与 English 两种语言。完整操作步骤、工具箱命令与命令行用法见 [开发文档](./docs/DEVELOPMENT.md)。

## 注意事项

- 当前发布版本主要面向 Windows x64。
- 便携化过程会修改浏览器主程序并加载 Chrome++，原始数字签名可能因此失效，部分安全软件可能产生误报。
- 使用前建议备份重要的浏览器数据。

## 姊妹内核

Chromium 系和 Firefox 系各有一个构建核心，两个仓库独立维护、各自发布，构建流程都按 7-Zip 解包 → 注入便携化运行时（Chromium 侧是 Chrome++，Gecko 侧是 [libportable][link-libportable]） → 打包 7z 并公布 SHA256 的顺序进行。

| 内核 | 构建核心仓库 | 负责的浏览器 |
| --- | --- | --- |
| **Chromium 系** | [ChromiumPortable][link-repo]（本仓库） | Chrome、Edge、Helium，以及 Brave / Vivaldi 等自用构建 |
| **Gecko 系** | [Gecko-Portable][link-gecko] | Firefox、Floorp、Zen |

**Gecko 系成品（由姊妹内核构建）**

| 项目 | 仓库 |
| --- | --- |
| **Firefox 便携版** | [Firefox-Portable][link-gecko-firefox] |
| **Floorp 便携版** | [Floorp_portable][link-gecko-floorp] |
| **Zen 便携版** | [Zen-Portable][link-gecko-zen] |

上方的[当前项目](#当前项目)三个渠道由本仓库构建；Gecko 系三个渠道由 [Gecko-Portable][link-gecko] 构建。

## 开发文档

新增浏览器、配置构建流程与维护发布任务见 [开发与接入文档](./docs/DEVELOPMENT.md)，命令行与 CI 说明也收录其中。下载站维护说明见 [docs/README.md](./docs/README.md)。商标、第三方组件与再分发边界见 [NOTICE](./NOTICE)。

## 许可证

项目源码使用 [MIT License](./LICENSE)。Chrome++、浏览器本体及其他第三方组件遵循各自的许可证；商标归属见 [NOTICE](./NOTICE)。

---

<div align="center">

<sub>Built and maintained with ❤️ by</sub>

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

[badge-edge-release]: https://img.shields.io/github/v/release/Piracola/Edge_Portable?display_name=tag&style=flat-square&color=1d7c84&label=
[badge-edge-downloads]: https://img.shields.io/github/downloads/Piracola/Edge_Portable/total?style=flat-square&color=2ea043&label=
[badge-edge-stars]: https://img.shields.io/github/stars/Piracola/Edge_Portable?style=flat-square&color=2f81f7&label=
[link-edge]: https://github.com/Piracola/Edge_Portable/releases

[badge-helium-release]: https://img.shields.io/github/v/release/Piracola/Helium_Portable?display_name=tag&style=flat-square&color=5b5bd6&label=
[badge-helium-downloads]: https://img.shields.io/github/downloads/Piracola/Helium_Portable/total?style=flat-square&color=2ea043&label=
[badge-helium-stars]: https://img.shields.io/github/stars/Piracola/Helium_Portable?style=flat-square&color=2f81f7&label=
[link-helium]: https://github.com/Piracola/Helium_Portable/releases/latest

[link-gecko]: https://github.com/Piracola/Gecko-Portable
[link-libportable]: https://github.com/adonais/libportable

[link-gecko-firefox]: https://github.com/Piracola/Firefox-Portable
[link-gecko-floorp]: https://github.com/Piracola/Floorp_portable
[link-gecko-zen]: https://github.com/Piracola/Zen-Portable
