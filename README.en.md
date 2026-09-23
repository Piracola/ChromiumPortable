<div align="center">

# ChromiumPortable

Shared build core for portable Chromium-family browsers

[![Core version][badge-version]][link-tags]
[![chrome++ sync][badge-sync]][link-sync]
[![License][badge-license]][link-license]
[![Stars][badge-stars]][link-repo]
[![Last commit][badge-last-commit]][link-repo]

**Language / 语言:** [简体中文](README.md) · **English**

[Download site](https://piracola.github.io/ChromiumPortable/) · [Help](https://piracola.github.io/ChromiumPortable/) · [Development docs](./docs/DEVELOPMENT.md)

</div>

This project builds portable browsers with Chrome++ integrated. It does not ship browser installers. Everyday users should get ready-made builds from the projects below or the unified download site.

## Current products

These three channels have dedicated release repositories and ongoing builds.

| Product | Repository | Latest | Published | Downloads | Stars |
| --- | --- | --- | --- | --- | --- |
| **Google Chrome Portable** | [Chrome-Portable][link-chrome] | [![][badge-chrome-release]][link-chrome] | 2026-09-09 | [![][badge-chrome-downloads]][link-chrome] | [![][badge-chrome-stars]][link-chrome] |
| **Microsoft Edge Portable** | [Edge_Portable][link-edge] | [![][badge-edge-release]][link-edge] | 2026-09-11 | [![][badge-edge-downloads]][link-edge] | [![][badge-edge-stars]][link-edge] |
| **Helium Portable** | [Helium_Portable][link-helium] | [![][badge-helium-release]][link-helium] | 2026-09-21 | [![][badge-helium-downloads]][link-helium] | [![][badge-helium-stars]][link-helium] |

**Channel notes**

- **Chrome**: `stable` / `beta`; official Omaha packages; artifacts `Chrome++_stable_*` / `Chrome++_beta_*`.
- **Edge**: stable; Microsoft CDP API + installer source; artifacts `Edge_Portable_Win64_*`.
- **Helium**: stable; auto layout parsing; artifacts `Helium_*`. Official product builds live in the child repo. If upstream has no public Windows package, pass `package_url`.

See the [download site](https://piracola.github.io/ChromiumPortable/) for files, versions, and checksums.

## Using a portable build

1. Download the archive for your browser.
2. Extract it to any writable folder.
3. Run the browser exe, or double-click `开始.bat` in the archive.

Browsing data is usually stored next to the browser folder in `Data` and `Cache`. When updating, replace only the browser program folder and keep your data directories.

## Other Chromium-family browsers (personal builds)

This toolchain can also produce portable builds of Brave, Vivaldi, Opera, Thorium, 360 Extreme Browser X, and similar.  
These are **not** products of this project: no download-site product cards, builds are for learning and personal use only. Read upstream terms before redistributing (see [NOTICE](./NOTICE)).

**Local one-shot (recommended)**

1. Install [Python 3](https://www.python.org/) (enable “Add to PATH”).
2. Copy installer packages into the dedicated folder **`installers\`** (see `README.txt` inside).
3. Double-click **`开始构建.bat`** at the repo root, or run:

```powershell
python scripts\wizard.py
```

4. Pick a mode (single package / whole folder / online download), architecture, and whether to archive.  
   Portable output lands in `build\release\`; archives in `build\assets\`.

The wizard UI language follows your system (Simplified Chinese / English). Force one with `WIZARD_LANG=en`.

Supported online browsers, CI release flow, and advanced CLI: [development docs](./docs/DEVELOPMENT.md).

## Notes

- Published builds target Windows x64.
- Portability patches the browser main program and loads Chrome++; original code signatures may break and some security tools may warn.
- Get product-channel files only from the child projects’ GitHub Releases or the unified download site, and verify SHA256 when needed. `Unofficial` builds of other browsers are not official.
- Back up important browser data before use.
- Trademarks, third-party components, and redistribution boundaries: [NOTICE](./NOTICE).

## Development

This repository is the build core, not a browser-binary distribution. To add a browser, wire a build, or maintain releases, see [development docs](./docs/DEVELOPMENT.md). Download-site maintenance: [docs/README.md](./docs/README.md).

## License

Project source uses the [MIT License](./LICENSE). Chrome++, browser binaries, and other third-party components follow their own licenses; trademarks: [NOTICE](./NOTICE).

---

<div align="center">

<sub>Built and maintained by</sub>

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

[badge-edge-release]: https://img.shields.io/github/v/release/betacola/Edge_Portable?display_name=tag&style=flat-square&color=1d7c84&label=
[badge-edge-downloads]: https://img.shields.io/github/downloads/betacola/Edge_Portable/total?style=flat-square&color=2ea043&label=
[badge-edge-stars]: https://img.shields.io/github/stars/betacola/Edge_Portable?style=flat-square&color=2f81f7&label=
[link-edge]: https://github.com/betacola/Edge_Portable/releases/latest

[badge-helium-release]: https://img.shields.io/github/v/release/Piracola/Helium_Portable?display_name=tag&style=flat-square&color=5b5bd6&label=
[badge-helium-downloads]: https://img.shields.io/github/downloads/Piracola/Helium_Portable/total?style=flat-square&color=2ea043&label=
[badge-helium-stars]: https://img.shields.io/github/stars/Piracola/Helium_Portable?style=flat-square&color=2f81f7&label=
[link-helium]: https://github.com/Piracola/Helium_Portable/releases/latest
