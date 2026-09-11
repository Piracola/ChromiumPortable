#!/usr/bin/env python3
"""从 data/site.json 生成整站静态页面（中英双语）。

站点部署在 GitHub Pages（Deploy from a branch，目录 /docs），没有服务端构建步骤，
因此生成结果必须提交进仓库。这个脚本是唯一的内容出口：页面结构、结构化数据、
hreflang 与 sitemap 全部由 data/site.json 渲染。

路径约定：
  中文（默认）  /  /chrome/  /edge/  /helium/
  英文          /en/  /en/chrome/  /en/edge/  /en/helium/

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
# 多语言路径
# --------------------------------------------------------------------------- #


def locales(site: dict) -> list[dict]:
    return site["meta"]["locales"]


def locale_by_code(site: dict, code: str) -> dict:
    for loc in locales(site):
        if loc["code"] == code:
            return loc
    raise KeyError(f"unknown locale: {code}")


def default_locale(site: dict) -> dict:
    return locale_by_code(site, site["meta"]["defaultLocale"])


def t(site: dict, code: str) -> dict:
    return site["i18n"][code]


def page_dir(locale: dict, slug: str | None) -> str:
    prefix = locale["path"]
    return f"{prefix}{slug}/" if slug else prefix


def depth_of(locale: dict, slug: str | None) -> int:
    parts = [p for p in page_dir(locale, slug).split("/") if p]
    return len(parts)


def rel_to_root(locale: dict, slug: str | None) -> str:
    d = depth_of(locale, slug)
    return "../" * d if d else ""


def rel_between(from_locale: dict, from_slug: str | None, to_locale: dict, to_slug: str | None) -> str:
    from_parts = [p for p in page_dir(from_locale, from_slug).split("/") if p]
    to_parts = [p for p in page_dir(to_locale, to_slug).split("/") if p]
    if from_parts == to_parts:
        return "./"
    up = "../" * len(from_parts)
    down = "/".join(to_parts)
    if down:
        return f"{up}{down}/"
    return up if up else "./"


def abs_url(site: dict, locale: dict, slug: str | None = None) -> str:
    return f"{site['meta']['siteUrl']}{page_dir(locale, slug)}"


def hreflang_links(site: dict, locale: dict, slug: str | None) -> str:
    lines = []
    for other in locales(site):
        lines.append(
            f'<link rel="alternate" hreflang="{esc(other["htmlLang"])}"'
            f' href="{esc(abs_url(site, other, slug))}" />'
        )
    default = default_locale(site)
    lines.append(
        f'<link rel="alternate" hreflang="x-default"'
        f' href="{esc(abs_url(site, default, slug))}" />'
    )
    return "\n    ".join(lines)


def lang_switch_html(site: dict, locale: dict, slug: str | None) -> str:
    links = []
    for other in locales(site):
        href = rel_between(locale, slug, other, slug)
        current = ' aria-current="true"' if other["code"] == locale["code"] else ""
        links.append(
            f'<a class="lang-switch__link" href="{esc(href)}"'
            f' lang="{esc(other["htmlLang"])}" hreflang="{esc(other["htmlLang"])}"{current}>'
            f'{esc(other["shortLabel"])}</a>'
        )
    label = t(site, locale["code"])["ui"]["languageSwitcher"]
    return (
        f'<div class="lang-switch" role="group" aria-label="{esc(label)}">'
        + "".join(links)
        + "</div>"
    )


def merged_build(site: dict, build: dict, code: str) -> dict:
    copy = t(site, code)["builds"][build["id"]]
    return {**build, **copy}


def merged_repo(site: dict, repo: dict, code: str) -> dict:
    copy = t(site, code)["repos"][repo["id"]]
    return {**repo, **copy}


def merged_page(site: dict, page: dict, code: str) -> dict:
    copy = t(site, code)["pages"][page["slug"]]
    return {**page, **copy}


def all_builds(site: dict, code: str) -> list[dict]:
    return [merged_build(site, b, code) for b in site["builds"]]


def all_repos(site: dict, code: str) -> list[dict]:
    return [merged_repo(site, r, code) for r in site["repos"]]


def all_pages(site: dict, code: str) -> list[dict]:
    return [merged_page(site, p, code) for p in site["pages"]]


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


def nav_html(site: dict, locale: dict, code: str, slug: str | None) -> str:
    ui = t(site, code)["ui"]
    rel = rel_to_root(locale, slug)
    links = []
    for page in all_pages(site, code):
        current = ' aria-current="page"' if slug == page["slug"] else ""
        href = rel_between(locale, slug, locale, page["slug"])
        links.append(
            f'<a class="nav__link" href="{esc(href)}"{current}>{esc(page["navLabel"])}</a>'
        )
    links.append(
        f'<a class="nav__link nav__link--external" href="{esc(site["meta"]["builderRepo"])}"'
        ' target="_blank" rel="noopener">GitHub</a>'
    )
    home_current = ' aria-current="page"' if slug is None else ""
    home_href = rel_between(locale, slug, locale, None)
    brand = t(site, code)["brand"]
    theme_light = esc(ui["themeToLight"])
    theme_dark = esc(ui["themeToDark"])
    return f"""<a class="skip-link" href="#main">{esc(ui["skipToContent"])}</a>
<header class="site-header">
  <div class="site-header__inner">
    <a class="brand" href="{home_href}"{home_current}>
      <img class="brand__mark" src="{rel}assets/favicon.svg" alt="" width="30" height="30" />
      <span class="brand__text">{esc(brand)}</span>
    </a>
    <nav class="nav" aria-label="{esc(ui["mainNav"])}">
      {chr(10).join('      ' + link for link in links).strip()}
    </nav>
    {lang_switch_html(site, locale, slug)}
    <button class="theme-toggle" type="button" data-theme-toggle
      data-label-light="{theme_light}" data-label-dark="{theme_dark}"
      aria-label="{theme_dark}">
      <svg class="theme-toggle__sun" viewBox="0 0 24 24" aria-hidden="true" width="18" height="18"><circle cx="12" cy="12" r="4.2"/><path d="M12 2.4v2.6M12 19v2.6M4.6 4.6l1.9 1.9M17.5 17.5l1.9 1.9M2.4 12h2.6M19 12h2.6M4.6 19.4l1.9-1.9M17.5 6.5l1.9-1.9"/></svg>
      <svg class="theme-toggle__moon" viewBox="0 0 24 24" aria-hidden="true" width="18" height="18"><path d="M20 13.4A8.2 8.2 0 0 1 10.6 4a8.4 8.4 0 1 0 9.4 9.4Z"/></svg>
    </button>
  </div>
</header>"""


def build_card_html(
    build: dict,
    repo: dict,
    release: dict | None,
    rel: str,
    ui: dict,
    *,
    detail_link: bool,
    detail_href: str,
) -> str:
    version = release["version"] if release else None
    size = human_size(release["size"]) if release else "—"
    updated = short_date(release["publishedAt"]) if release else "—"
    download_url = release["url"] if release else repo["releasesUrl"]
    download_label = ui["download7z"] if release else ui["goReleases"]

    version_chip = (
        f'<span class="build-card__version">v{esc(version)}</span>' if version else ""
    )
    detail = (
        f'<a class="btn btn--soft" href="{esc(detail_href)}">{esc(ui["usageGuide"])}</a>'
        if detail_link
        else f'<a class="btn btn--soft" href="{esc(repo["url"])}" target="_blank" rel="noopener">{esc(ui["projectRepo"])}</a>'
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
    <div><dt>{esc(ui["version"])}</dt><dd>{esc(version) if version else "—"}</dd></div>
    <div><dt>{esc(ui["size"])}</dt><dd>{esc(size)}</dd></div>
    <div><dt>{esc(ui["updated"])}</dt><dd>{esc(updated)}</dd></div>
    <div><dt>{esc(ui["platform"])}</dt><dd>Windows {esc(build["architecture"])}</dd></div>
  </dl>
  <div class="build-card__actions">
    <a class="btn btn--primary" href="{esc(download_url)}" target="_blank" rel="noopener">{download_label}{version_chip}</a>
    {detail}
    <a class="btn btn--quiet" href="{esc(repo["workflowUrl"])}" target="_blank" rel="noopener">{esc(ui["buildLog"])}</a>
  </div>
</article>"""


def faq_html(site: dict, code: str, *, open_first: bool) -> str:
    faq = t(site, code)["faq"]
    items = []
    for index, item in enumerate(faq["shared"]):
        is_open = " open" if (open_first and index == 0) else ""
        items.append(
            f"""<details class="faq__item"{is_open}>
  <summary>{esc(item["q"])}</summary>
  <div class="faq__answer"><p>{esc(item["a"])}</p></div>
</details>"""
        )
    return f"""<section class="section" id="faq">
  <div class="section__head">
    <h2>{esc(faq["heading"])}</h2>
  </div>
  <div class="faq">
{indent(chr(10).join(items), 4)}
  </div>
</section>"""


def features_html(site: dict, code: str) -> str:
    features = t(site, code)["features"]
    items = "\n".join(f"    <li>{esc(text)}</li>" for text in features["items"])
    return f"""<section class="section" id="features">
  <div class="section__head">
    <h2>{esc(features["heading"])}</h2>
    <p>{esc(features["description"])}</p>
  </div>
  <ul class="feature-list">
{items}
  </ul>
</section>"""


def verify_html(site: dict, code: str, asset_hint: str) -> str:
    verify = t(site, code)["verify"]
    command = verify["command"].replace("{asset}", asset_hint)
    return f"""<section class="section" id="verify">
  <div class="section__head">
    <h2>{esc(verify["heading"])}</h2>
    <p>{esc(verify["description"])}</p>
  </div>
  <pre class="code"><code>{esc(command)}</code></pre>
  <p class="note">{esc(verify["note"])}</p>
</section>"""


def footer_html(site: dict, locale: dict, code: str, slug: str | None, updated: str) -> str:
    footer = t(site, code)["footer"]
    ui = t(site, code)["ui"]
    pages = all_pages(site, code)
    credits = "\n".join(
        f'          <li><span>{esc(credit["label"])}</span>'
        f'<a href="{esc(credit["url"])}" target="_blank" rel="noopener">{esc(credit["name"])}</a></li>'
        for credit in footer["credits"]
    )
    page_links = "\n".join(
        f'          <li><a href="{rel_between(locale, slug, locale, page["slug"])}">{esc(page["h1"])}</a></li>'
        for page in pages
    )
    stamp = (
        f'<p class="footer__stamp">{esc(ui["footerStamp"].format(date=esc(updated)))}</p>'
        if updated
        else ""
    )
    return f"""<footer class="site-footer">
  <div class="site-footer__inner">
    <div class="site-footer__about">
      <p class="site-footer__tagline">{esc(footer["tagline"])}</p>
      {stamp}
    </div>
    <div class="site-footer__cols">
      <div>
        <h2>{esc(ui["browsers"])}</h2>
        <ul>
{page_links}
        </ul>
      </div>
      <div>
        <h2>{esc(ui["upstreamCredits"])}</h2>
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


def software_application(
    build: dict,
    repo: dict,
    release: dict | None,
    page_url: str,
    currency: str,
) -> dict:
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
        "offers": {"@type": "Offer", "price": "0", "priceCurrency": currency},
        "author": {"@type": "Person", "name": repo["owner"]},
        "license": "https://opensource.org/licenses/MIT",
    }
    if release:
        node["softwareVersion"] = release["version"]
        node["datePublished"] = short_date(release["publishedAt"])
        node["fileSize"] = f"{release['size']}"
    return node


def home_jsonld(site: dict, locale: dict, code: str, releases: dict) -> dict:
    content = t(site, code)
    site_url = abs_url(site, locale, None)
    currency = "CNY" if code.startswith("zh") else "USD"
    repo_map = {repo["id"]: repo for repo in all_repos(site, code)}
    items = []
    for position, build in enumerate(all_builds(site, code), start=1):
        repo = repo_map[build["repoId"]]
        release = (releases.get("builds") or {}).get(build["id"])
        page_url = abs_url(site, locale, build["pageSlug"])
        items.append(
            {
                "@type": "ListItem",
                "position": position,
                "item": software_application(build, repo, release, page_url, currency),
            }
        )

    return {
        "@context": "https://schema.org",
        "@graph": [
            {
                "@type": "WebSite",
                "@id": f"{site['meta']['siteUrl']}#website",
                "name": content["siteName"],
                "url": site["meta"]["siteUrl"],
                "inLanguage": locale["htmlLang"],
                "publisher": {"@type": "Person", "name": site["meta"]["author"]},
            },
            {
                "@type": "CollectionPage",
                "@id": f"{site_url}#webpage",
                "url": site_url,
                "name": content["home"]["title"],
                "description": content["home"]["description"],
                "isPartOf": {"@id": f"{site['meta']['siteUrl']}#website"},
                "inLanguage": locale["htmlLang"],
            },
            {"@type": "ItemList", "name": content["jsonld"]["itemListName"], "itemListElement": items},
            {
                "@type": "FAQPage",
                "@id": f"{site_url}#faq",
                "mainEntity": [
                    {
                        "@type": "Question",
                        "name": item["q"],
                        "acceptedAnswer": {"@type": "Answer", "text": item["a"]},
                    }
                    for item in content["faq"]["shared"]
                ],
            },
        ],
    }


def page_jsonld(site: dict, locale: dict, code: str, page: dict, builds: list[dict], releases: dict) -> dict:
    content = t(site, code)
    currency = "CNY" if code.startswith("zh") else "USD"
    site_root = site["meta"]["siteUrl"]
    page_url = abs_url(site, locale, page["slug"])
    repo_map = {repo["id"]: repo for repo in all_repos(site, code)}
    repo = repo_map[page["repoId"]]

    apps = [
        software_application(
            build, repo, (releases.get("builds") or {}).get(build["id"]), page_url, currency
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
                "inLanguage": locale["htmlLang"],
                "isPartOf": {"@id": f"{site_root}#website"},
            },
            {
                "@type": "BreadcrumbList",
                "itemListElement": [
                    {
                        "@type": "ListItem",
                        "position": 1,
                        "name": content["ui"]["home"],
                        "item": abs_url(site, locale, None),
                    },
                    {"@type": "ListItem", "position": 2, "name": page["h1"], "item": page_url},
                ],
            },
            *apps,
        ],
    }


# --------------------------------------------------------------------------- #
# 页面渲染
# --------------------------------------------------------------------------- #


def head_html(
    site: dict,
    locale: dict,
    code: str,
    *,
    rel: str,
    slug: str | None,
    title: str,
    description: str,
    canonical: str,
    jsonld: dict,
) -> str:
    og_image = site["meta"]["siteUrl"] + site["meta"]["ogImage"]
    payload = json.dumps(jsonld, ensure_ascii=False, indent=2)
    alternates = hreflang_links(site, locale, slug)
    site_name = t(site, code)["siteName"]
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
    {alternates}
    <link rel="icon" type="image/svg+xml" href="{rel}assets/favicon.svg" />
    <link rel="manifest" href="{rel}site.webmanifest" />
    <meta property="og:type" content="website" />
    <meta property="og:locale" content="{esc(locale["ogLocale"])}" />
    <meta property="og:site_name" content="{esc(site_name)}" />
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


def render_home(site: dict, locale: dict, code: str, releases: dict) -> str:
    content = t(site, code)
    ui = content["ui"]
    slug = None
    rel = rel_to_root(locale, slug)
    repo_map = {repo["id"]: repo for repo in all_repos(site, code)}
    release_map = releases.get("builds") or {}
    builds = all_builds(site, code)
    pages = all_pages(site, code)

    stats = indent(
        "\n".join(
            f"""<div class="stat">
  <span class="stat__value">{esc(stat["value"])}</span>
  <span class="stat__label">{esc(stat["label"])}</span>
  <span class="stat__detail">{esc(stat["detail"])}</span>
</div>"""
            for stat in content["home"]["stats"]
        ),
        10,
    )

    families = [ui["filterAll"]] + [page["family"] for page in pages]
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
            build_card_html(
                build,
                repo_map[build["repoId"]],
                release_map.get(build["id"]),
                rel,
                ui,
                detail_link=True,
                detail_href=rel_between(locale, slug, locale, build["pageSlug"]),
            )
            for build in builds
        ),
        10,
    )

    trust = indent(
        "\n".join(
            f"""<article class="trust-card">
  <h3>{esc(item["title"])}</h3>
  <p>{esc(item["body"])}</p>
</article>"""
            for item in content["trust"]["items"]
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
    <a href="{esc(repo["url"])}" target="_blank" rel="noopener">{esc(ui["repoLink"])}</a>
    <a href="{esc(repo["releasesUrl"])}" target="_blank" rel="noopener">Releases</a>
    <a href="{esc(repo["workflowUrl"])}" target="_blank" rel="noopener">Actions</a>
  </div>
</article>"""
            for repo in all_repos(site, code)
        ),
        10,
    )

    head = head_html(
        site,
        locale,
        code,
        rel=rel,
        slug=slug,
        title=content["home"]["title"],
        description=content["home"]["description"],
        canonical=abs_url(site, locale, slug),
        jsonld=home_jsonld(site, locale, code, releases),
    )

    return f"""<!doctype html>
<html lang="{esc(locale["htmlLang"])}">
  <head>
    {head}
  </head>
  <body>
    {indent(nav_html(site, locale, code, slug), 4).strip()}
    <main id="main">
      <section class="hero">
        <p class="eyebrow">{esc(content["home"]["eyebrow"])}</p>
        <h1>{esc(content["home"]["h1"])}</h1>
        <p class="hero__lede">{esc(content["home"]["lede"])}</p>
        <div class="hero__actions">
          <a class="btn btn--primary btn--lg" href="#downloads">{esc(ui["viewDownloads"])}</a>
          <a class="btn btn--soft btn--lg" href="{esc(site["meta"]["builderRepo"])}" target="_blank" rel="noopener">{esc(ui["builderSource"])}</a>
        </div>
        <div class="stats">
{stats}
        </div>
      </section>

      <section class="section" id="downloads">
        <div class="section__head">
          <h2>{esc(content["downloads"]["heading"])}</h2>
          <p>{esc(content["downloads"]["description"])}</p>
        </div>
        <div class="filters" role="group" aria-label="{esc(ui["filterByBrowser"])}">
{filters}
        </div>
        <div class="build-grid" id="buildGrid">
{cards}
        </div>
      </section>

      <section class="section" id="why">
        <div class="section__head">
          <h2>{esc(content["trust"]["heading"])}</h2>
          <p>{esc(content["trust"]["description"])}</p>
        </div>
        <div class="trust-grid">
{trust}
        </div>
      </section>

      {indent(features_html(site, code), 6).strip()}

      <section class="section" id="repos">
        <div class="section__head">
          <h2>{esc(content["reposSection"]["heading"])}</h2>
          <p>{esc(content["reposSection"]["description"])}</p>
        </div>
        <div class="repo-grid">
{repos}
        </div>
      </section>

      {indent(faq_html(site, code, open_first=True), 6).strip()}
    </main>
    {indent(footer_html(site, locale, code, slug, releases.get("fetchedAt", "")), 4).strip()}
    <script src="{rel}app.js" defer></script>
  </body>
</html>
"""


def render_page(site: dict, locale: dict, code: str, page: dict, releases: dict) -> str:
    content = t(site, code)
    ui = content["ui"]
    slug = page["slug"]
    rel = rel_to_root(locale, slug)
    repo_map = {repo["id"]: repo for repo in all_repos(site, code)}
    repo = repo_map[page["repoId"]]
    release_map = releases.get("builds") or {}
    builds = [b for b in all_builds(site, code) if b["pageSlug"] == slug]

    cards = indent(
        "\n".join(
            build_card_html(
                build,
                repo,
                release_map.get(build["id"]),
                rel,
                ui,
                detail_link=False,
                detail_href="#",
            )
            for build in builds
        ),
        10,
    )

    head = head_html(
        site,
        locale,
        code,
        rel=rel,
        slug=slug,
        title=page["title"],
        description=page["description"],
        canonical=abs_url(site, locale, slug),
        jsonld=page_jsonld(site, locale, code, page, builds, releases),
    )

    downloads_note = content["pageChrome"]["downloadsNote"].format(repo=repo["name"])
    eyebrow = content["pageChrome"]["eyebrow"].format(upstream=page["upstream"])

    return f"""<!doctype html>
<html lang="{esc(locale["htmlLang"])}">
  <head>
    {head}
  </head>
  <body>
    {indent(nav_html(site, locale, code, slug), 4).strip()}
    <main id="main">
      <nav class="breadcrumb" aria-label="{esc(ui["breadcrumb"])}">
        <a href="{rel_between(locale, slug, locale, None)}">{esc(ui["home"])}</a>
        <span aria-hidden="true">/</span>
        <span>{esc(page["h1"])}</span>
      </nav>

      <section class="hero hero--page">
        <p class="eyebrow">{esc(eyebrow)}</p>
        <h1>{esc(page["h1"])}</h1>
        <p class="hero__lede">{esc(page["lede"])}</p>
        <p class="hero__note">{esc(page["upstreamNote"])}</p>
      </section>

      <section class="section" id="downloads">
        <div class="section__head">
          <h2>{esc(content["downloads"]["heading"])}</h2>
          <p>{esc(downloads_note)}</p>
        </div>
        <div class="build-grid build-grid--page">
{cards}
        </div>
      </section>

      {indent(sections_html(page), 6).strip()}

      {indent(verify_html(site, code, builds[0]["assetHint"].replace("…", ui["assetHintSuffix"])), 6).strip()}

      {indent(features_html(site, code), 6).strip()}

      {indent(faq_html(site, code, open_first=False), 6).strip()}
    </main>
    {indent(footer_html(site, locale, code, slug, releases.get("fetchedAt", "")), 4).strip()}
    <script src="{rel}app.js" defer></script>
  </body>
</html>
"""


def render_sitemap(site: dict, releases: dict) -> str:
    lastmod = releases.get("fetchedAt") or ""
    lines = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"'
        ' xmlns:xhtml="http://www.w3.org/1999/xhtml">',
    ]

    entries: list[tuple[str, float, str | None]] = []
    for locale in locales(site):
        prefix = locale["path"]
        entries.append((f"{site['meta']['siteUrl']}{prefix}", 1.0, None))
        for page in site["pages"]:
            entries.append(
                (f"{site['meta']['siteUrl']}{prefix}{page['slug']}/", 0.9, page["slug"])
            )

    for url, priority, slug in entries:
        lines.append("  <url>")
        lines.append(f"    <loc>{url}</loc>")
        for other in locales(site):
            alt = abs_url(site, other, slug)
            lines.append(
                f'    <xhtml:link rel="alternate" hreflang="{other["htmlLang"]}" href="{alt}" />'
            )
        default = default_locale(site)
        lines.append(
            f'    <xhtml:link rel="alternate" hreflang="x-default" href="{abs_url(site, default, slug)}" />'
        )
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
        DOCS / "sitemap.xml": render_sitemap(site, releases),
    }
    for locale in locales(site):
        code = locale["code"]
        prefix = locale["path"]
        outputs[DOCS / prefix / "index.html" if prefix else DOCS / "index.html"] = render_home(
            site, locale, code, releases
        )
        for page in all_pages(site, code):
            out = DOCS / prefix / page["slug"] / "index.html" if prefix else DOCS / page["slug"] / "index.html"
            outputs[out] = render_page(site, locale, code, page, releases)
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
