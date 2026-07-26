#!/usr/bin/env python3
"""从 data/site.json 生成整站静态页面。

站点部署在 GitHub Pages（Deploy from a branch，目录 /docs），没有服务端构建步骤，
因此生成结果必须提交进仓库。这个脚本是唯一的内容出口：页面结构、结构化数据和
sitemap 全部由 data/site.json 渲染，避免同一份构建清单在 HTML、JSON-LD 和前端
脚本里各维护一遍。

用法（在 docs/ 下或任意目录执行均可）：

    python tools/generate.py                  # 重新生成全部页面
    python tools/generate.py --check          # 只校验产物是否与数据源一致
    python tools/generate.py --fetch-releases # 先抓取各仓库最新 Release 再生成

--fetch-releases 会写入 data/releases.json（版本号、体积、发布日期、SHA256），
生成时把这些信息烘焙进 HTML，让页面带上真实版本号，并作为 sitemap 的 lastmod。
只用标准库，不依赖 requests。
"""

from __future__ import annotations

import argparse
import html
import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

DOCS = Path(__file__).resolve().parent.parent
DATA = DOCS / "data"
SITE_JSON = DATA / "site.json"
RELEASES_JSON = DATA / "releases.json"

GITHUB_API = "https://api.github.com/repos/{owner}/{repo}/releases/latest"


# --------------------------------------------------------------------------- #
# 基础工具
# --------------------------------------------------------------------------- #


def esc(value: object) -> str:
    """HTML 转义，用于所有插值。"""
    return html.escape(str(value), quote=True)


def load_json(path: Path) -> dict:
    with path.open(encoding="utf-8") as fh:
        return json.load(fh)


def human_size(num_bytes: int | None) -> str:
    if not num_bytes:
        return "—"
    mb = num_bytes / 1024 / 1024
    if mb >= 1024:
        return f"{mb / 1024:.2f} GB"
    return f"{mb:.0f} MB"


def short_date(stamp: str | None) -> str:
    if not stamp:
        return "—"
    return stamp[:10]


def indent(block: str, spaces: int) -> str:
    pad = " " * spaces
    return "\n".join(pad + line if line.strip() else line for line in block.splitlines())


# --------------------------------------------------------------------------- #
# Release 抓取
# --------------------------------------------------------------------------- #


def fetch_releases(site: dict) -> dict:
    """抓取每个仓库的 latest release，并按 assetPattern 匹配到具体构建目标。"""
    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": "ChromiumPortable-docs-generator",
    }
    token = os.environ.get("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"

    repos: dict[str, dict] = {}
    latest_stamp = ""

    for repo in site["repos"]:
        url = GITHUB_API.format(owner=repo["owner"], repo=repo["name"])
        request = urllib.request.Request(url, headers=headers)
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                payload = json.load(response)
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
            print(f"  ! {repo['name']}: 抓取失败（{exc}），保留旧数据", file=sys.stderr)
            continue

        published = payload.get("published_at") or ""
        latest_stamp = max(latest_stamp, published)
        repos[repo["id"]] = {
            "tag": payload.get("tag_name") or "",
            "publishedAt": published,
            "htmlUrl": payload.get("html_url") or repo["releasesUrl"],
            "assets": [
                {
                    "name": asset.get("name", ""),
                    "size": asset.get("size", 0),
                    "url": asset.get("browser_download_url", ""),
                    "digest": asset.get("digest") or "",
                }
                for asset in payload.get("assets", [])
            ],
        }
        print(f"  · {repo['name']}: {payload.get('tag_name')} ({len(repos[repo['id']]['assets'])} assets)")

    previous = load_json(RELEASES_JSON) if RELEASES_JSON.exists() else {}
    for repo_id, old in (previous.get("repos") or {}).items():
        repos.setdefault(repo_id, old)

    builds: dict[str, dict] = {}
    for build in site["builds"]:
        repo_data = repos.get(build["repoId"])
        if not repo_data:
            continue
        pattern = re.compile(build["assetPattern"])
        for asset in repo_data["assets"]:
            match = pattern.match(asset["name"])
            if not match:
                continue
            digest = asset["digest"]
            builds[build["id"]] = {
                "version": match.group("version"),
                "assetName": asset["name"],
                "size": asset["size"],
                "url": asset["url"],
                "sha256": digest.split(":", 1)[-1] if digest else "",
                "publishedAt": repo_data["publishedAt"],
            }
            break

    if not latest_stamp:
        latest_stamp = previous.get("fetchedAt") or ""

    return {
        "fetchedAt": short_date(latest_stamp) or previous.get("fetchedAt", ""),
        "repos": repos,
        "builds": builds,
    }


# --------------------------------------------------------------------------- #
# 页面片段
# --------------------------------------------------------------------------- #


def nav_html(site: dict, rel: str, active: str) -> str:
    links = []
    for page in site["pages"]:
        current = ' aria-current="page"' if active == page["slug"] else ""
        links.append(
            f'<a class="nav__link" href="{rel}{page["slug"]}/"{current}>{esc(page["navLabel"])}</a>'
        )
    links.append(
        f'<a class="nav__link nav__link--external" href="{esc(site["meta"]["builderRepo"])}"'
        ' target="_blank" rel="noopener">GitHub</a>'
    )
    home_current = ' aria-current="page"' if active == "home" else ""
    home_href = rel or "./"
    return f"""<a class="skip-link" href="#main">跳到主要内容</a>
<header class="site-header">
  <div class="site-header__inner">
    <a class="brand" href="{home_href}"{home_current}>
      <img class="brand__mark" src="{rel}assets/favicon.svg" alt="" width="30" height="30" />
      <span class="brand__text">Chromium 便携版</span>
    </a>
    <nav class="nav" aria-label="主导航">
      {chr(10).join('      ' + link for link in links).strip()}
    </nav>
    <button class="theme-toggle" type="button" data-theme-toggle aria-label="切换深色模式">
      <svg class="theme-toggle__sun" viewBox="0 0 24 24" aria-hidden="true" width="18" height="18"><circle cx="12" cy="12" r="4.2"/><path d="M12 2.4v2.6M12 19v2.6M4.6 4.6l1.9 1.9M17.5 17.5l1.9 1.9M2.4 12h2.6M19 12h2.6M4.6 19.4l1.9-1.9M17.5 6.5l1.9-1.9"/></svg>
      <svg class="theme-toggle__moon" viewBox="0 0 24 24" aria-hidden="true" width="18" height="18"><path d="M20 13.4A8.2 8.2 0 0 1 10.6 4a8.4 8.4 0 1 0 9.4 9.4Z"/></svg>
    </button>
  </div>
</header>"""


def build_card_html(build: dict, repo: dict, release: dict | None, rel: str, *, detail_link: bool) -> str:
    version = release["version"] if release else None
    size = human_size(release["size"]) if release else "—"
    updated = short_date(release["publishedAt"]) if release else "—"
    download_url = release["url"] if release else repo["releasesUrl"]
    download_label = "下载 .7z" if release else "前往 Releases"

    version_chip = (
        f'<span class="build-card__version">v{esc(version)}</span>' if version else ""
    )
    detail = (
        f'<a class="btn btn--soft" href="{rel}{build["pageSlug"]}/">使用说明</a>'
        if detail_link
        else f'<a class="btn btn--soft" href="{esc(repo["url"])}" target="_blank" rel="noopener">项目仓库</a>'
    )

    return f"""<article class="build-card" data-family="{esc(build["family"])}" style="--accent: {esc(build["accent"])}">
  <div class="build-card__head">
    <img class="build-card__icon" src="{rel}{esc(build["icon"])}" alt="" width="40" height="40" loading="lazy" />
    <div class="build-card__title">
      <h3>{esc(build["title"])}</h3>
      <p>{esc(build["channel"])} · {esc(build["highlight"])}</p>
    </div>
    <span class="pill">{esc(build["channel"])}</span>
  </div>
  <p class="build-card__summary">{esc(build["summary"])}</p>
  <dl class="build-card__meta">
    <div><dt>版本</dt><dd>{esc(version) if version else "—"}</dd></div>
    <div><dt>体积</dt><dd>{esc(size)}</dd></div>
    <div><dt>更新</dt><dd>{esc(updated)}</dd></div>
    <div><dt>平台</dt><dd>Windows {esc(build["architecture"])}</dd></div>
  </dl>
  <div class="build-card__actions">
    <a class="btn btn--primary" href="{esc(download_url)}" target="_blank" rel="noopener">{download_label}{version_chip}</a>
    {detail}
    <a class="btn btn--quiet" href="{esc(repo["workflowUrl"])}" target="_blank" rel="noopener">构建日志</a>
  </div>
</article>"""


def faq_html(site: dict, *, open_first: bool) -> str:
    items = []
    for index, item in enumerate(site["faq"]["shared"]):
        is_open = " open" if (open_first and index == 0) else ""
        items.append(
            f"""<details class="faq__item"{is_open}>
  <summary>{esc(item["q"])}</summary>
  <div class="faq__answer"><p>{esc(item["a"])}</p></div>
</details>"""
        )
    return f"""<section class="section" id="faq">
  <div class="section__head">
    <h2>{esc(site["faq"]["heading"])}</h2>
  </div>
  <div class="faq">
{indent(chr(10).join(items), 4)}
  </div>
</section>"""


def features_html(site: dict) -> str:
    items = "\n".join(
        f'    <li>{esc(text)}</li>' for text in site["features"]["items"]
    )
    return f"""<section class="section" id="features">
  <div class="section__head">
    <h2>{esc(site["features"]["heading"])}</h2>
    <p>{esc(site["features"]["description"])}</p>
  </div>
  <ul class="feature-list">
{items}
  </ul>
</section>"""


def verify_html(site: dict, asset_hint: str) -> str:
    verify = site["verify"]
    command = verify["command"].replace("{asset}", asset_hint)
    return f"""<section class="section" id="verify">
  <div class="section__head">
    <h2>{esc(verify["heading"])}</h2>
    <p>{esc(verify["description"])}</p>
  </div>
  <pre class="code"><code>{esc(command)}</code></pre>
  <p class="note">{esc(verify["note"])}</p>
</section>"""


def footer_html(site: dict, rel: str, updated: str) -> str:
    credits = "\n".join(
        f'          <li><span>{esc(credit["label"])}</span>'
        f'<a href="{esc(credit["url"])}" target="_blank" rel="noopener">{esc(credit["name"])}</a></li>'
        for credit in site["footer"]["credits"]
    )
    page_links = "\n".join(
        f'          <li><a href="{rel}{page["slug"]}/">{esc(page["h1"])}</a></li>'
        for page in site["pages"]
    )
    stamp = f'<p class="footer__stamp">版本信息更新于 {esc(updated)}</p>' if updated else ""
    return f"""<footer class="site-footer">
  <div class="site-footer__inner">
    <div class="site-footer__about">
      <p class="site-footer__tagline">{esc(site["footer"]["tagline"])}</p>
      {stamp}
    </div>
    <div class="site-footer__cols">
      <div>
        <h2>浏览器</h2>
        <ul>
{page_links}
        </ul>
      </div>
      <div>
        <h2>上游与致谢</h2>
        <ul class="site-footer__credits">
{credits}
        </ul>
      </div>
    </div>
  </div>
</footer>"""


def sections_html(page: dict) -> str:
    blocks = []
    for section in page["sections"]:
        parts = [f'  <h2>{esc(section["heading"])}</h2>']
        for paragraph in section.get("paragraphs", []):
            parts.append(f"  <p>{esc(paragraph)}</p>")
        if section.get("list"):
            items = "\n".join(f"    <li>{esc(item)}</li>" for item in section["list"])
            parts.append(f'  <ul class="prose__list">\n{items}\n  </ul>')
        if section.get("steps"):
            items = "\n".join(f"    <li>{esc(item)}</li>" for item in section["steps"])
            parts.append(f'  <ol class="prose__steps">\n{items}\n  </ol>')
        blocks.append("<section class=\"prose\">\n" + "\n".join(parts) + "\n</section>")
    return "\n\n".join(blocks)


# --------------------------------------------------------------------------- #
# 结构化数据
# --------------------------------------------------------------------------- #


def software_application(build: dict, repo: dict, release: dict | None, site_url: str, page_url: str) -> dict:
    node = {
        "@type": "SoftwareApplication",
        "name": build["title"],
        "applicationCategory": "BrowserApplication",
        "operatingSystem": "Windows 10, Windows 11",
        "processorRequirements": "x64",
        "url": page_url,
        "downloadUrl": release["url"] if release else repo["releasesUrl"],
        "softwareHelp": page_url,
        "isAccessibleForFree": True,
        "offers": {"@type": "Offer", "price": "0", "priceCurrency": "CNY"},
        "author": {"@type": "Person", "name": repo["owner"]},
        "license": "https://opensource.org/licenses/MIT",
    }
    if release:
        node["softwareVersion"] = release["version"]
        node["datePublished"] = short_date(release["publishedAt"])
        node["fileSize"] = f"{release['size']}"
    return node


def home_jsonld(site: dict, releases: dict) -> dict:
    site_url = site["meta"]["siteUrl"]
    repo_map = {repo["id"]: repo for repo in site["repos"]}
    items = []
    for position, build in enumerate(site["builds"], start=1):
        repo = repo_map[build["repoId"]]
        release = (releases.get("builds") or {}).get(build["id"])
        page_url = f"{site_url}{build['pageSlug']}/"
        items.append(
            {
                "@type": "ListItem",
                "position": position,
                "item": software_application(build, repo, release, site_url, page_url),
            }
        )

    return {
        "@context": "https://schema.org",
        "@graph": [
            {
                "@type": "WebSite",
                "@id": f"{site_url}#website",
                "name": site["meta"]["siteName"],
                "url": site_url,
                "inLanguage": "zh-CN",
                "publisher": {"@type": "Person", "name": site["meta"]["author"]},
            },
            {
                "@type": "CollectionPage",
                "@id": f"{site_url}#webpage",
                "url": site_url,
                "name": site["home"]["title"],
                "description": site["home"]["description"],
                "isPartOf": {"@id": f"{site_url}#website"},
                "inLanguage": "zh-CN",
            },
            {"@type": "ItemList", "name": "可下载的便携版构建", "itemListElement": items},
            {
                "@type": "FAQPage",
                "@id": f"{site_url}#faq",
                "mainEntity": [
                    {
                        "@type": "Question",
                        "name": item["q"],
                        "acceptedAnswer": {"@type": "Answer", "text": item["a"]},
                    }
                    for item in site["faq"]["shared"]
                ],
            },
        ],
    }


def page_jsonld(site: dict, page: dict, builds: list[dict], releases: dict) -> dict:
    site_url = site["meta"]["siteUrl"]
    page_url = f"{site_url}{page['slug']}/"
    repo_map = {repo["id"]: repo for repo in site["repos"]}
    repo = repo_map[page["repoId"]]

    apps = [
        software_application(
            build, repo, (releases.get("builds") or {}).get(build["id"]), site_url, page_url
        )
        for build in builds
    ]

    return {
        "@context": "https://schema.org",
        "@graph": [
            {
                "@type": "WebPage",
                "@id": f"{page_url}#webpage",
                "url": page_url,
                "name": page["title"],
                "description": page["description"],
                "inLanguage": "zh-CN",
                "isPartOf": {"@id": f"{site_url}#website"},
            },
            {
                "@type": "BreadcrumbList",
                "itemListElement": [
                    {"@type": "ListItem", "position": 1, "name": "首页", "item": site_url},
                    {"@type": "ListItem", "position": 2, "name": page["h1"], "item": page_url},
                ],
            },
            *apps,
        ],
    }


# --------------------------------------------------------------------------- #
# 页面渲染
# --------------------------------------------------------------------------- #


def head_html(site: dict, *, rel: str, title: str, description: str, canonical: str, jsonld: dict) -> str:
    og_image = site["meta"]["siteUrl"] + site["meta"]["ogImage"]
    payload = json.dumps(jsonld, ensure_ascii=False, indent=2)
    return f"""<meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{esc(title)}</title>
    <meta name="description" content="{esc(description)}" />
    <meta name="author" content="{esc(site["meta"]["author"])}" />
    <meta name="robots" content="index, follow, max-image-preview:large" />
    <meta name="color-scheme" content="light dark" />
    <meta name="theme-color" content="#ffffff" media="(prefers-color-scheme: light)" />
    <meta name="theme-color" content="#0c0d11" media="(prefers-color-scheme: dark)" />
    <link rel="canonical" href="{esc(canonical)}" />
    <link rel="icon" type="image/svg+xml" href="{rel}assets/favicon.svg" />
    <link rel="manifest" href="{rel}site.webmanifest" />
    <meta property="og:type" content="website" />
    <meta property="og:locale" content="zh_CN" />
    <meta property="og:site_name" content="{esc(site["meta"]["siteName"])}" />
    <meta property="og:title" content="{esc(title)}" />
    <meta property="og:description" content="{esc(description)}" />
    <meta property="og:url" content="{esc(canonical)}" />
    <meta property="og:image" content="{esc(og_image)}" />
    <meta property="og:image:width" content="1200" />
    <meta property="og:image:height" content="630" />
    <meta name="twitter:card" content="summary_large_image" />
    <meta name="twitter:title" content="{esc(title)}" />
    <meta name="twitter:description" content="{esc(description)}" />
    <meta name="twitter:image" content="{esc(og_image)}" />
    <link rel="stylesheet" href="{rel}styles.css" />
    <script>
      (function () {{
        try {{
          var stored = localStorage.getItem("cp-theme");
          if (stored === "dark" || stored === "light") {{
            document.documentElement.dataset.theme = stored;
          }}
        }} catch (error) {{}}
      }})();
    </script>
    <script type="application/ld+json">
{indent(payload, 6)}
    </script>"""


def render_home(site: dict, releases: dict) -> str:
    repo_map = {repo["id"]: repo for repo in site["repos"]}
    release_map = releases.get("builds") or {}
    rel = ""

    stats = indent(
        "\n".join(
            f"""<div class="stat">
  <span class="stat__value">{esc(stat["value"])}</span>
  <span class="stat__label">{esc(stat["label"])}</span>
  <span class="stat__detail">{esc(stat["detail"])}</span>
</div>"""
            for stat in site["home"]["stats"]
        ),
        10,
    )

    families = ["全部"] + [page["family"] for page in site["pages"]]
    filters = indent(
        "\n".join(
            f'<button class="filter{" is-active" if index == 0 else ""}" type="button"'
            f' data-filter="{esc("all" if index == 0 else family)}"'
            f' aria-pressed="{"true" if index == 0 else "false"}">{esc(family)}</button>'
            for index, family in enumerate(families)
        ),
        10,
    )

    cards = indent(
        "\n".join(
            build_card_html(build, repo_map[build["repoId"]], release_map.get(build["id"]), rel, detail_link=True)
            for build in site["builds"]
        ),
        10,
    )

    trust = indent(
        "\n".join(
            f"""<article class="trust-card">
  <h3>{esc(item["title"])}</h3>
  <p>{esc(item["body"])}</p>
</article>"""
            for item in site["trust"]["items"]
        ),
        10,
    )

    repos = indent(
        "\n".join(
            f"""<article class="repo-card">
  <div class="repo-card__head">
    <img src="{esc(repo["icon"])}" alt="" width="32" height="32" loading="lazy" />
    <div>
      <h3>{esc(repo["name"])}</h3>
      <p>{esc(repo["owner"])}</p>
    </div>
  </div>
  <p class="repo-card__summary">{esc(repo["summary"])}</p>
  <div class="repo-card__links">
    <a href="{esc(repo["url"])}" target="_blank" rel="noopener">仓库</a>
    <a href="{esc(repo["releasesUrl"])}" target="_blank" rel="noopener">Releases</a>
    <a href="{esc(repo["workflowUrl"])}" target="_blank" rel="noopener">Actions</a>
  </div>
</article>"""
            for repo in site["repos"]
        ),
        10,
    )

    head = head_html(
        site,
        rel=rel,
        title=site["home"]["title"],
        description=site["home"]["description"],
        canonical=site["meta"]["siteUrl"],
        jsonld=home_jsonld(site, releases),
    )

    return f"""<!doctype html>
<html lang="zh-CN">
  <head>
    {head}
  </head>
  <body>
    {indent(nav_html(site, rel, "home"), 4).strip()}
    <main id="main">
      <section class="hero">
        <p class="eyebrow">Windows x64 · 免安装 · 开源自动构建</p>
        <h1>{esc(site["home"]["h1"])}</h1>
        <p class="hero__lede">{esc(site["home"]["lede"])}</p>
        <div class="hero__actions">
          <a class="btn btn--primary btn--lg" href="#downloads">查看下载</a>
          <a class="btn btn--soft btn--lg" href="{esc(site["meta"]["builderRepo"])}" target="_blank" rel="noopener">构建核心源码</a>
        </div>
        <div class="stats">
{stats}
        </div>
      </section>

      <section class="section" id="downloads">
        <div class="section__head">
          <h2>下载</h2>
          <p>选择浏览器与渠道。所有压缩包由 GitHub Actions 自动构建并直接发布到对应仓库的 Releases。</p>
        </div>
        <div class="filters" role="group" aria-label="按浏览器筛选">
{filters}
        </div>
        <div class="build-grid" id="buildGrid">
{cards}
        </div>
      </section>

      <section class="section" id="why">
        <div class="section__head">
          <h2>{esc(site["trust"]["heading"])}</h2>
          <p>{esc(site["trust"]["description"])}</p>
        </div>
        <div class="trust-grid">
{trust}
        </div>
      </section>

      {indent(features_html(site), 6).strip()}

      <section class="section" id="repos">
        <div class="section__head">
          <h2>项目仓库</h2>
          <p>每个浏览器一个独立仓库，共用同一套构建核心。</p>
        </div>
        <div class="repo-grid">
{repos}
        </div>
      </section>

      {indent(faq_html(site, open_first=True), 6).strip()}
    </main>
    {indent(footer_html(site, rel, releases.get("fetchedAt", "")), 4).strip()}
    <script src="{rel}app.js" defer></script>
  </body>
</html>
"""


def render_page(site: dict, page: dict, releases: dict) -> str:
    rel = "../"
    repo_map = {repo["id"]: repo for repo in site["repos"]}
    repo = repo_map[page["repoId"]]
    release_map = releases.get("builds") or {}
    builds = [build for build in site["builds"] if build["pageSlug"] == page["slug"]]

    cards = indent(
        "\n".join(
            build_card_html(build, repo, release_map.get(build["id"]), rel, detail_link=False)
            for build in builds
        ),
        10,
    )

    head = head_html(
        site,
        rel=rel,
        title=page["title"],
        description=page["description"],
        canonical=f"{site['meta']['siteUrl']}{page['slug']}/",
        jsonld=page_jsonld(site, page, builds, releases),
    )

    return f"""<!doctype html>
<html lang="zh-CN">
  <head>
    {head}
  </head>
  <body>
    {indent(nav_html(site, rel, page["slug"]), 4).strip()}
    <main id="main">
      <nav class="breadcrumb" aria-label="面包屑">
        <a href="{rel}">首页</a>
        <span aria-hidden="true">/</span>
        <span>{esc(page["h1"])}</span>
      </nav>

      <section class="hero hero--page">
        <p class="eyebrow">{esc(page["upstream"])} · Windows x64 · 免安装</p>
        <h1>{esc(page["h1"])}</h1>
        <p class="hero__lede">{esc(page["lede"])}</p>
        <p class="hero__note">{esc(page["upstreamNote"])}</p>
      </section>

      <section class="section" id="downloads">
        <div class="section__head">
          <h2>下载</h2>
          <p>压缩包直接来自 {esc(repo["name"])} 仓库的 Releases，附带 SHA256 校验值。</p>
        </div>
        <div class="build-grid build-grid--page">
{cards}
        </div>
      </section>

      {indent(sections_html(page), 6).strip()}

      {indent(verify_html(site, builds[0]["assetHint"].replace("…", "版本号_日期.")), 6).strip()}

      {indent(features_html(site), 6).strip()}

      {indent(faq_html(site, open_first=False), 6).strip()}
    </main>
    {indent(footer_html(site, rel, releases.get("fetchedAt", "")), 4).strip()}
    <script src="{rel}app.js" defer></script>
  </body>
</html>
"""


def render_sitemap(site: dict, releases: dict) -> str:
    lastmod = releases.get("fetchedAt") or ""
    entries = [(site["meta"]["siteUrl"], "1.0")]
    entries += [(f"{site['meta']['siteUrl']}{page['slug']}/", "0.9") for page in site["pages"]]

    lines = ['<?xml version="1.0" encoding="UTF-8"?>', '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">']
    for url, priority in entries:
        lines.append("  <url>")
        lines.append(f"    <loc>{url}</loc>")
        if lastmod:
            lines.append(f"    <lastmod>{lastmod}</lastmod>")
        lines.append("    <changefreq>daily</changefreq>")
        lines.append(f"    <priority>{priority}</priority>")
        lines.append("  </url>")
    lines.append("</urlset>")
    return "\n".join(lines) + "\n"


# --------------------------------------------------------------------------- #
# 入口
# --------------------------------------------------------------------------- #


def build_outputs(site: dict, releases: dict) -> dict[Path, str]:
    outputs: dict[Path, str] = {
        DOCS / "index.html": render_home(site, releases),
        DOCS / "sitemap.xml": render_sitemap(site, releases),
    }
    for page in site["pages"]:
        outputs[DOCS / page["slug"] / "index.html"] = render_page(site, page, releases)
    return outputs


def main() -> int:
    parser = argparse.ArgumentParser(description="生成 GitHub Pages 静态站点")
    parser.add_argument("--check", action="store_true", help="只校验产物是否与数据源一致，不写文件")
    parser.add_argument("--fetch-releases", action="store_true", help="抓取各仓库最新 Release 并写入 data/releases.json")
    args = parser.parse_args()

    site = load_json(SITE_JSON)

    if args.fetch_releases:
        print("抓取最新 Release：")
        releases = fetch_releases(site)
        RELEASES_JSON.parent.mkdir(parents=True, exist_ok=True)
        RELEASES_JSON.write_text(
            json.dumps(releases, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        print(f"已写入 {RELEASES_JSON.relative_to(DOCS)}（{len(releases['builds'])} 个构建目标）")
    else:
        releases = load_json(RELEASES_JSON) if RELEASES_JSON.exists() else {}

    outputs = build_outputs(site, releases)

    if args.check:
        stale = []
        for path, content in outputs.items():
            if not path.exists() or path.read_text(encoding="utf-8") != content:
                stale.append(path.relative_to(DOCS).as_posix())
        if stale:
            print("以下文件与 data/site.json 不同步，请运行 python tools/generate.py：", file=sys.stderr)
            for name in stale:
                print(f"  - {name}", file=sys.stderr)
            return 1
        print(f"检查通过，{len(outputs)} 个产物均与数据源一致。")
        return 0

    for path, content in outputs.items():
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        print(f"已生成 {path.relative_to(DOCS).as_posix()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
