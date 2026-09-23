# AGENTS.md

ChromiumPortable：Chromium 系浏览器便携构建核心（解包 → Chrome++ 注入 → 打包/验证/发版）。不发布浏览器成品。

## 怎么跑

```powershell
python -m pip install -r requirements.txt
python -m compileall portable_builder scripts tests
python -m unittest discover -s tests
python scripts\wizard.py                 # 或双击 开始构建.bat
python -m portable_builder --workdir . build-package <安装包> --archive
```

产品渠道 CI 入口是 reusable workflow；单浏览器按需构建用 `build-browser.yml`。下载站生成：`cd docs; python tools/generate.py`。

## 技术栈

Python 3.12+（`requests`）、Windows PE 解析（`portable_builder/pe.py`）、7-Zip/`7zr.exe`、GitHub Actions。站点为 `docs/` 静态页（`tools/generate.py` 由 `data/site.json` 渲染）。

## 目录与约定

- `portable_builder/` — 构建核心；`layout: auto` 静态识别主程序，绝不运行安装包/浏览器
- `scripts/` — 向导（**仅 zh-CN / en**）、`catalog` 选靶、`upstream/resolve.py` 取包
- `catalog/browser_catalog.json` — 在线 target；非产品必须 `Unofficial` + Disclaimer
- `installers/` — 用户投放安装包（已 ignore）；`bin/` — 回归样本（已 ignore，勿提交）
- `docs/` — GitHub Pages 下载站；HTML 是生成产物，改 `data/site.json` 后重跑生成器
- README 仅中英双语；徽标用 shields `flat-square`，表内数字只出现在徽标里

## 当前状态与下一步

核心 auto 布局 / 向导 / build-browser 已合入 `main`（2026-09-24）。产品面仅 Chrome / Edge / Helium。下载站仍是四语（zh-CN / zh-TW / en / ja），若收成中英需改 `site.json` 并重新生成。新浏览器优先 `layout: auto` + catalog；涉及注入/验证的改动需用三渠道实际配置回归。
