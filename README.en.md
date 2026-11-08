<div align="center">

# ChromiumPortable

Shared build core for portable Chromium-family browsers

[![Core version][badge-version]][link-tags]
[![chrome++ sync][badge-sync]][link-sync]
[![License][badge-license]][link-license]
[![Stars][badge-stars]][link-repo]
[![Last commit][badge-last-commit]][link-repo]

**Language / 语言:** [简体中文](README.md) · **English**

[Download site](https://piracola.github.io/ChromiumPortable/) · [Help](https://piracola.github.io/ChromiumPortable/) · [Sibling engine](#sibling-engine) · [Development docs](./docs/DEVELOPMENT.md)

</div>

This repository provides a build engine and a graphical builder for portable Chromium-family browsers with Chrome++ integrated. Browser binaries are not distributed here; the three products are published by the repositories listed below.

## Current products

These three channels have dedicated release repositories and ongoing builds.

| Product | Repository | Latest | Published | Downloads | Stars |
| --- | --- | --- | --- | --- | --- |
| **Google Chrome Portable** | [Chrome-Portable][link-chrome] | [![][badge-chrome-release]][link-chrome] | 2026-09-09 | [![][badge-chrome-downloads]][link-chrome] | [![][badge-chrome-stars]][link-chrome] |
| **Microsoft Edge Portable** | [Edge_Portable][link-edge] | [![][badge-edge-release]][link-edge] | 2026-09-11 | [![][badge-edge-downloads]][link-edge] | [![][badge-edge-stars]][link-edge] |
| **Helium Portable** | [Helium_Portable][link-helium] | [![][badge-helium-release]][link-helium] | 2026-09-21 | [![][badge-helium-downloads]][link-helium] | [![][badge-helium-stars]][link-helium] |

See the [download site](https://piracola.github.io/ChromiumPortable/) for files, versions, and checksums.

## Using a portable build

1. Download the archive for your browser.
2. Extract it to any writable folder.
3. Run the browser exe, or double-click `开始.bat` in the archive.

Browsing data is usually stored next to the browser folder in `Data` and `Cache`. When updating, replace only the browser program folder and keep the data directories unchanged.

## Graphical builder

[Releases](../../releases) provides `ChromiumPortableBuilder-windows-x64.zip`, which contains the build engine and the graphical interface; the target machine does not need Python. Extract it, run `builder-app.exe`, and place browser installers in `installers\` to select the source, architecture, and output directory and start a build. The interface offers Simplified Chinese and English. Full operating steps, toolbox commands, and command-line usage are in the [development docs](./docs/DEVELOPMENT.md).

## Notes

- Published builds target Windows x64.
- Portability patches the browser main program and loads Chrome++; original code signatures may break and some security tools may warn.
- Back up important browser data before use.

## Sibling engine

Chromium-family and Firefox-family builds each have their own engine. The two repositories are maintained and released independently, and both follow the sequence of unpacking with 7-Zip → injecting the portability runtime (Chrome++ on the Chromium side, [libportable][link-libportable] on the Gecko side) → packaging as 7z and publishing a SHA256.

| Family | Engine repository | Browsers covered |
| --- | --- | --- |
| **Chromium family** | [ChromiumPortable][link-repo] (this repo) | Chrome, Edge, Helium, plus personal builds such as Brave / Vivaldi |
| **Gecko family** | [Gecko-Portable][link-gecko] | Firefox, Floorp, Zen |

**Gecko-family products (built by the sibling engine)**

| Product | Repository |
| --- | --- |
| **Firefox Portable** | [Firefox-Portable][link-gecko-firefox] |
| **Floorp Portable** | [Floorp_portable][link-gecko-floorp] |
| **Zen Portable** | [Zen-Portable][link-gecko-zen] |

The channels under [Current products](#current-products) are built by this repository; the three Gecko channels are built by [Gecko-Portable][link-gecko].

## Development docs

Adding a browser, wiring a build, and maintaining releases are documented in the [development docs](./docs/DEVELOPMENT.md), which also covers command-line usage and CI. Download-site maintenance: [docs/README.md](./docs/README.md). Trademarks, third-party components, and redistribution boundaries: [NOTICE](./NOTICE).

## License

Project source uses the [MIT License](./LICENSE). Chrome++, browser binaries, and other third-party components follow their own licenses; trademarks: [NOTICE](./NOTICE).

---

<div align="center">

<sub>Built and maintained with ❤️ by</sub>

**Piracola**

</div>

<!-- Badge labels are percent-encoded where needed for shields.io. -->
[badge-version]: https://img.shields.io/github/v/tag/Piracola/ChromiumPortable?style=flat-square&color=2f81f7&label=core
[badge-sync]: https://img.shields.io/github/actions/workflow/status/Piracola/ChromiumPortable/update-chrome-plus.yml?branch=main&style=flat-square&color=2ea043&label=chrome%2B%2B%20sync
[badge-license]: https://img.shields.io/github/license/Piracola/ChromiumPortable?style=flat-square&color=6e7681&label=license
[badge-stars]: https://img.shields.io/github/stars/Piracola/ChromiumPortable?style=flat-square&color=d8653f&label=Stars
[badge-last-commit]: https://img.shields.io/github/last-commit/Piracola/ChromiumPortable?style=flat-square&color=555555&label=last%20commit

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
