#!/usr/bin/env python3
"""生成 assets/og.png（1200×630 社交分享预览图）。

分享到 IM、社交平台或出现在搜索结果卡片里时使用。og:image 不支持 SVG，
所以这里用 Pillow 画一张位图并提交进仓库；内容基本不变，改动文案时重跑即可：

    python -m pip install pillow
    python tools/make_og.py

依赖 Pillow 与系统中文字体，只在本地手动执行，不参与 CI 构建。
"""

from __future__ import annotations

import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

DOCS = Path(__file__).resolve().parent.parent
OUTPUT = DOCS / "assets" / "og.png"

WIDTH, HEIGHT = 1200, 630
PAD = 84

BG = (11, 12, 15)
GRID = (255, 255, 255, 10)
TEXT = (236, 238, 242)
MUTED = (169, 176, 187)
DIM = (122, 130, 141)

BRANDS = [
    ("Chrome", (216, 101, 63)),
    ("Edge", (45, 150, 160)),
    ("Helium", (123, 111, 240)),
]

FONT_CANDIDATES = {
    "bold": ["msyhbd.ttc", "msyh.ttc", "simhei.ttf"],
    "regular": ["msyh.ttc", "msyhl.ttc", "simsun.ttc"],
}


def load_font(kind: str, size: int) -> ImageFont.FreeTypeFont:
    for name in FONT_CANDIDATES[kind]:
        path = Path("C:/Windows/Fonts") / name
        if path.exists():
            return ImageFont.truetype(str(path), size, index=0)
    raise SystemExit("找不到可用的中文字体，请在 FONT_CANDIDATES 中补充字体文件名")


def main() -> int:
    image = Image.new("RGB", (WIDTH, HEIGHT), BG)
    draw = ImageDraw.Draw(image, "RGBA")

    # 背景网格，和站点 body 的网格保持一致的呼吸感
    for x in range(0, WIDTH, 44):
        draw.line([(x, 0), (x, HEIGHT)], fill=GRID, width=1)
    for y in range(0, HEIGHT, 44):
        draw.line([(0, y), (WIDTH, y)], fill=GRID, width=1)

    # 顶部品牌色条
    segment = WIDTH // len(BRANDS)
    for index, (_, color) in enumerate(BRANDS):
        draw.rectangle([index * segment, 0, (index + 1) * segment, 5], fill=color)

    eyebrow = load_font("regular", 25)
    title = load_font("bold", 78)
    subtitle = load_font("regular", 31)
    label = load_font("bold", 25)
    url_font = load_font("regular", 24)

    draw.text((PAD, PAD + 6), "WINDOWS x64 · 免安装 · 开源自动构建", font=eyebrow, fill=DIM)

    draw.text((PAD, PAD + 68), "Chrome、Edge、Helium", font=title, fill=TEXT)
    draw.text((PAD, PAD + 170), "便携版下载", font=title, fill=TEXT)

    draw.text(
        (PAD, PAD + 296),
        "不写注册表，数据留在自己的文件夹里",
        font=subtitle,
        fill=MUTED,
    )
    draw.text(
        (PAD, PAD + 342),
        "每日跟随官方版本自动构建 · 全流程 SHA256 校验 · 发布前实机验证",
        font=subtitle,
        fill=MUTED,
    )

    # 底部品牌点 + 站点地址
    baseline = HEIGHT - PAD - 12
    cursor = PAD
    for name, color in BRANDS:
        draw.ellipse([cursor, baseline + 6, cursor + 15, baseline + 21], fill=color)
        cursor += 26
        draw.text((cursor, baseline), name, font=label, fill=TEXT)
        cursor += int(draw.textlength(name, font=label)) + 40

    url = "piracola.github.io/ChromiumPortable"
    url_width = draw.textlength(url, font=url_font)
    draw.text((WIDTH - PAD - url_width, baseline + 2), url, font=url_font, fill=DIM)

    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    image.save(OUTPUT, "PNG", optimize=True)
    print(f"已生成 {OUTPUT.relative_to(DOCS).as_posix()}（{OUTPUT.stat().st_size // 1024} KB）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
