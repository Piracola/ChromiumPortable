#!/usr/bin/env python3
"""Resolve a browser's latest upstream installer to the script-provider JSON.

Prints one JSON object on stdout:
  {"version", "url", "file_name", "sha256", "size", "verify_ssl"}

Used by catalog targets whose package source is not a built-in provider.
Never runs installers or browsers.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import xml.etree.ElementTree as tree
from pathlib import Path

import requests

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from portable_builder.tools import normalize_sha256  # noqa: E402

UA = {"User-Agent": "ChromiumPortable-UpstreamResolver/1.0"}
WIN_EXE_SUFFIXES = (".exe", ".msi", ".7z", ".zip")


def github_headers():
    headers = dict(UA)
    headers["Accept"] = "application/vnd.github+json"
    token = os.getenv("GITHUB_TOKEN") or os.getenv("GH_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    return headers


def get_json(url, headers=None):
    response = requests.get(url, headers=headers or UA, timeout=60)
    if response.status_code == 403 and "rate limit" in response.text.casefold():
        raise RuntimeError(
            "GitHub API rate limit exceeded (unauthenticated). "
            "Set GITHUB_TOKEN and retry, or build from a local installer (向导选项 1)."
        )
    response.raise_for_status()
    return response.json()


def emit(data: dict) -> None:
    if data.get("sha256"):
        data["sha256"] = normalize_sha256(data["sha256"])
    data.setdefault("verify_ssl", True)
    print(json.dumps(data, ensure_ascii=False))


def pick_github_asset(assets, patterns):
    for pattern in patterns:
        regex = re.compile(pattern, re.IGNORECASE)
        for asset in assets:
            name = asset.get("name") or ""
            if regex.search(name):
                return asset
    return None


def from_github_release(repo, patterns, prerelease=False):
    url = f"https://api.github.com/repos/{repo}/releases?per_page=15"
    try:
        releases = get_json(url, headers=github_headers())
    except RuntimeError:
        raise
    except requests.RequestException as exc:
        raise RuntimeError(f"GitHub request failed for {repo}: {exc}") from exc
    for release in releases:
        if release.get("draft"):
            continue
        if release.get("prerelease") and not prerelease:
            continue
        assets = release.get("assets") or []
        asset = pick_github_asset(assets, patterns)
        if not asset:
            continue
        tag = release.get("tag_name") or ""
        match = re.search(r"(\d+(?:\.\d+)+)", tag)
        version = match.group(1) if match else "0.0.0.0"
        return {
            "version": version,
            "url": asset["browser_download_url"],
            "file_name": asset["name"],
            "size": asset.get("size"),
            "sha256": (asset.get("digest") or "").replace("sha256:", "") or None,
        }
    raise RuntimeError(f"No matching Windows asset in {repo} releases.")


def resolve_thorium():
    # Windows installers live on the active fork's release, not Alex313031/thorium notes.
    for repo in ("gz83/thorium", "Alex313031/thorium", "Alex313031/Thorium-Build"):
        try:
            return from_github_release(
                repo,
                [
                    r"thorium_AVX2_mini_installer\.exe$",
                    r"thorium_.*_mini_installer\.exe$",
                    r"thorium.*_installer\.exe$",
                    r"mini_installer.*\.exe$",
                    r"thorium.*\.exe$",
                ],
            )
        except RuntimeError as exc:
            if "rate limit" in str(exc).casefold():
                raise
            continue
    raise RuntimeError("No Thorium Windows installer asset found in known repos.")


def resolve_helium():
    # Upstream tags exist, but Windows binaries are often absent from GitHub assets.
    rate_limited = False
    for patterns in (
        [r"helium.*win.*\.(exe|msi|7z|zip)$", r"win.*\.(exe|msi|7z|zip)$", r"\.(exe|msi)$"],
        [r"Helium.*\.(exe|msi|7z|zip)$"],
    ):
        try:
            return from_github_release("imputnet/helium", patterns)
        except RuntimeError as exc:
            if "rate limit" in str(exc).casefold():
                rate_limited = True
                break
            continue
    if rate_limited:
        raise RuntimeError(
            "GitHub API rate limit exceeded while resolving Helium. "
            "Set GITHUB_TOKEN and retry, or use the wizard's local-installer mode."
        )
    raise RuntimeError(
        "imputnet/helium has no pickable Windows installer on GitHub Releases. "
        "Download a Helium Windows package yourself and use the wizard's "
        "'构建单个安装包' mode (or workflow input package_url)."
    )


def _brave_version_hint(text):
    match = re.search(r"(\d+\.\d+\.\d+(?:\.\d+)?)", text or "")
    return match.group(1) if match else "0.0.0.0"


def resolve_brave():
    # Full standalone installer (not the ~1MB online stub). Prefer explicit file URL.
    candidates = (
        "https://referrals.brave.com/latest/brave_installer-x64.exe",
        "https://referrals.brave.com/latest/BraveBrowserStandaloneSetup.exe",
        "https://laptop-updates.brave.com/latest/winx64",
    )
    for url in candidates:
        try:
            response = requests.get(url, stream=True, allow_redirects=True, timeout=60, headers=UA)
            final = response.url or url
            length = int(response.headers.get("Content-Length") or 0)
            disposition = response.headers.get("Content-Disposition") or ""
            response.close()
            # Skip online-bootstrapper stubs; static extract needs the full payload.
            if length and length < 5 * 1024 * 1024:
                continue
            name = re.search(r'filename="?([^";]+)"?', disposition)
            file_name = name.group(1) if name else (final.rstrip("/").split("/")[-1] or "brave_installer-x64.exe")
            version = _brave_version_hint(disposition) if disposition else "0.0.0.0"
            return {
                "version": version,
                "url": final,
                "file_name": file_name,
                "size": length or None,
                "verify_ssl": True,
            }
        except requests.RequestException:
            continue

    update_url = "https://updates.bravesoftware.com/service/update2"
    app_id = "{AFE6A462-C574-4B8A-AF43-4CC60DF4563B}"
    xml_data = f'''<?xml version="1.0" encoding="UTF-8"?>
<request protocol="3.0" updater="Omaha" updaterversion="1.3.36.372" shell_version="1.3.36.352" ismachine="0" sessionid="{{11111111-1111-1111-1111-111111111111}}" installsource="taggedmi" requestid="{{11111111-1111-1111-1111-111111111111}}" dedup="cr" domainjoined="0">
  <hw physmemory="16" sse="1" sse2="1" sse3="1" ssse3="1" sse41="1" sse42="1" avx="1"/>
  <os platform="win" version="10.0" sp="" arch="x64"/>
  <app appid="{app_id}" version="" nextversion="" lang="en" brand="" installage="-1" installdate="-1" iid="{{11111111-1111-1111-1111-111111111111}}">
    <updatecheck/>
    <data name="install" index="empty"/>
  </app>
</request>'''
    response = requests.post(update_url, data=xml_data, headers=UA, timeout=60)
    response.raise_for_status()
    root = tree.fromstring(response.text)
    manifest = root.find(".//manifest")
    package = root.find(".//package")
    if manifest is None or package is None:
        raise RuntimeError("Brave update response had no manifest/package.")
    package_name = package.get("name") or "brave_installer.exe"
    urls = []
    for node in root.findall(".//url"):
        codebase = node.get("codebase")
        if codebase:
            urls.append(codebase + package_name)
    if not urls:
        raise RuntimeError("Brave update response had no download URL.")
    size = package.get("size")
    return {
        "version": manifest.get("version") or "0.0.0.0",
        "url": urls[0],
        "file_name": package_name,
        "sha256": package.get("hash_sha256"),
        "size": int(size) if size and str(size).isdigit() else None,
    }


def resolve_vivaldi():
    # Official download page lists versioned CDN links; pick newest win-x64 installer.
    response = requests.get("https://vivaldi.com/download/", headers=UA, timeout=60)
    response.raise_for_status()
    links = re.findall(r'https://downloads\.vivaldi\.com/stable/Vivaldi\.[^"\']+?\.x64\.exe', response.text)
    if not links:
        links = re.findall(r'https://downloads\.vivaldi\.com/stable/Vivaldi\.[^"\']+?\.exe', response.text)
    if not links:
        raise RuntimeError("No Vivaldi Windows installer link found on vivaldi.com/download.")

    def version_of(url):
        match = re.search(r"(\d+(?:\.\d+)+)", url)
        return match.group(1) if match else "0"

    best = sorted(set(links), key=version_of, reverse=True)[0]
    version = version_of(best)
    return {
        "version": version,
        "url": best,
        "file_name": best.rstrip("/").split("/")[-1],
        "verify_ssl": True,
    }


def resolve_opera():
    # Official desktop FTP lists versioned dirs; win/ holds Setup_x64.exe + sha256sum.
    base = "https://ftp.opera.com/pub/opera/desktop/"
    listing = requests.get(base, headers=UA, timeout=60)
    listing.raise_for_status()
    versions = re.findall(r'href="(\d+\.\d+\.\d+\.\d+)/"', listing.text)
    if not versions:
        versions = re.findall(r"\b(\d+\.\d+\.\d+\.\d+)/\b", listing.text)
    if not versions:
        raise RuntimeError("Opera FTP listing had no version directories.")
    versions.sort(key=lambda value: tuple(int(part) for part in value.split(".")))
    version = versions[-1]
    file_name = f"Opera_{version}_Setup_x64.exe"
    url = f"{base}{version}/win/{file_name}"
    sha_url = url + ".sha256sum"
    sha256 = None
    try:
        digest_response = requests.get(sha_url, headers=UA, timeout=30)
        if digest_response.status_code == 200:
            match = re.search(r"\b([0-9a-fA-F]{64})\b", digest_response.text)
            if match:
                sha256 = match.group(1)
    except requests.RequestException:
        pass
    return {
        "version": version,
        "url": url,
        "file_name": file_name,
        "sha256": sha256,
    }


def resolve_github_alias(key):
    mapping = {
        "thorium": (resolve_thorium, ()),
        "helium": (resolve_helium, ()),
    }
    if key not in mapping:
        raise KeyError(key)
    func, args = mapping[key]
    return func(*args)


def resolve_cse360():
    """360 极速浏览器 X (360csex) via browser.360.cn download_link.js — not 360cse / 360se."""
    source = "https://browser.360.cn/browser_download_link.js"
    response = requests.get(source, headers=UA, timeout=60)
    response.raise_for_status()
    text = response.text
    # Prefer the versioned X installer (EEX); never pick CSE_32 / SE_* / SET_LINK.
    url = None
    for key in ("EEX_64_UNIVERSAL", "EEX", "CSEX_64"):
        match = re.search(rf"{key}\s*:\s*['\"]([^'\"]+)['\"]", text)
        if match and re.search(r"360csex", match.group(1), re.I) and not match.group(1).rstrip("/").endswith("_setup.exe"):
            url = match.group(1)
            break
    if not url:
        match = re.search(r"['\"](https?://[^'\"]*360csex_[\d.]+[^'\"]*\.exe)['\"]", text, re.I)
        if match:
            url = match.group(1)
    if not url:
        # Last resort: unversioned official setup link on chromex site pattern.
        match = re.search(r"['\"](https?://[^'\"]*360csex_setup\.exe)['\"]", text, re.I)
        if match:
            url = match.group(1)
    if not url:
        raise RuntimeError("No 360csex (极速X) installer URL in browser_download_link.js")

    version_match = re.search(r"(\d+(?:\.\d+)+)", url)
    version = version_match.group(1) if version_match else "0.0.0.0"
    return {
        "version": version,
        "url": url,
        "file_name": url.rstrip("/").split("/")[-1],
        "verify_ssl": True,
    }


RESOLVERS = {
    "brave": resolve_brave,
    "vivaldi": resolve_vivaldi,
    "opera": resolve_opera,
    "thorium": resolve_thorium,
    "helium": resolve_helium,
    "cse360": resolve_cse360,
}


def main(argv=None):
    parser = argparse.ArgumentParser(description="Resolve latest upstream installer")
    parser.add_argument(
        "--browser",
        required=True,
        choices=sorted(RESOLVERS),
        help="Browser key",
    )
    args = parser.parse_args(argv)

    data = RESOLVERS[args.browser]()
    emit(data)


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"[FAIL] {exc}", file=sys.stderr)
        raise SystemExit(1)
