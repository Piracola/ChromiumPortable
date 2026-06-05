import os
import re
from datetime import datetime
from pathlib import Path
from string import Formatter

import requests

from .builder import build_context, format_value, get_version_info
from .github_env import write_env
from .versions import is_major_update, is_minor_update, is_upgrade

DEFAULT_SAMPLE_VERSION = "123.456.789.0"
DEFAULT_SAMPLE_DATE = "2099-12-31"
PLACEHOLDER_PATTERNS = {
    "version": r"\d+(?:\.\d+)+",
    "package_version": r"\d+(?:\.\d+)+",
    "date": r"\d{4}-\d{2}-\d{2}",
    "arch": r"[A-Za-z0-9][A-Za-z0-9.-]*",
}


def github_headers():
    headers = {"Accept": "application/vnd.github.v3+json"}
    token = os.getenv("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"token {token}"
    return headers


def latest_release():
    repo = os.getenv("GITHUB_REPOSITORY")
    if not repo:
        print("[INFO] GITHUB_REPOSITORY is not set; assuming first local run.")
        return None

    response = requests.get(f"https://api.github.com/repos/{repo}/releases/latest", headers=github_headers(), timeout=60)
    if response.status_code == 404:
        return None
    response.raise_for_status()
    return response.json()


def extract_version(text):
    if not text:
        return None
    match = re.search(r"(\d+\.\d+\.\d+\.\d+)", text)
    return match.group(1) if match else None


def asset_context(target, version=DEFAULT_SAMPLE_VERSION, date=DEFAULT_SAMPLE_DATE, package_version=None, fill_defaults=True):
    if fill_defaults:
        return build_context(target, version=version, date=date, package_version=package_version or version)

    resolved_version = "" if version is None else version
    resolved_package_version = resolved_version if package_version is None else package_version
    return {
        "target": target.get("target", ""),
        "name": target.get("name", target.get("display_name", "")),
        "display_name": target.get("display_name", target.get("name", "")),
        "output_dir": target.get("output_dir", target.get("name", "Browser")),
        "version": resolved_version,
        "package_version": resolved_package_version,
        "date": "" if date is None else date,
        "arch": target.get("architecture", "x64"),
    }


def asset_name_template(target):
    return target.get("archive_name", "{display_name}_Portable_{version}_{date}.7z")


def render_asset_name(target, version=DEFAULT_SAMPLE_VERSION, date=DEFAULT_SAMPLE_DATE, package_version=None):
    context = asset_context(target, version=version, date=date, package_version=package_version)
    return format_value(asset_name_template(target), context)


def archive_name_regex(target):
    template = asset_name_template(target)
    context = asset_context(target, version=None, date=None, package_version=None, fill_defaults=False)
    parts = ["^"]
    for literal_text, field_name, format_spec, conversion in Formatter().parse(template):
        del format_spec, conversion
        parts.append(re.escape(literal_text))
        if field_name is None:
            continue

        value = context.get(field_name)
        if value not in (None, ""):
            parts.append(re.escape(str(value)))
            continue

        parts.append(PLACEHOLDER_PATTERNS.get(field_name, r"[^/]+?"))

    parts.append("$")
    return re.compile("".join(parts), re.IGNORECASE)


def asset_matchers(target):
    release_config = target.get("release", {})
    matchers = []

    asset_regex = release_config.get("asset_regex")
    if asset_regex:
        compiled = re.compile(asset_regex, re.IGNORECASE)
        matchers.append(("release.asset_regex", lambda name, regex=compiled: bool(regex.search(name))))

    if target.get("archive_name"):
        compiled = archive_name_regex(target)
        matchers.append((f"archive_name:{asset_name_template(target)}", lambda name, regex=compiled: bool(regex.fullmatch(name))))

    asset_match = release_config.get("asset_match")
    if asset_match:
        mode = str(release_config.get("asset_match_mode", "contains")).lower()
        needle = str(asset_match)
        if mode == "exact":
            matchers.append((f"asset_match(exact):{needle}", lambda name, value=needle: name.lower() == value.lower()))
        elif mode == "prefix":
            matchers.append((f"asset_match(prefix):{needle}", lambda name, value=needle: name.lower().startswith(value.lower())))
        elif mode == "suffix":
            matchers.append((f"asset_match(suffix):{needle}", lambda name, value=needle: name.lower().endswith(value.lower())))
        elif mode == "regex":
            compiled = re.compile(needle, re.IGNORECASE)
            matchers.append((f"asset_match(regex):{needle}", lambda name, regex=compiled: bool(regex.search(name))))
        else:
            matchers.append((f"asset_match(contains):{needle}", lambda name, value=needle: value.lower() in name.lower()))

    if not matchers:
        fallback = str(target.get("output_dir", target.get("name", "")))
        if fallback:
            matchers.append((f"default(contains):{fallback}", lambda name, value=fallback: value.lower() in name.lower()))

    return matchers


def matching_assets_for_target(assets, target):
    for description, matcher in asset_matchers(target):
        matches = [asset for asset in assets if matcher(asset.get("name", ""))]
        if matches:
            return matches, description
    return [], None


def asset_matches_target(target, asset_name):
    return any(matcher(asset_name) for _, matcher in asset_matchers(target))


def target_match_description(target):
    return ", ".join(description for description, _ in asset_matchers(target)) or "(none)"


def release_version(release, target):
    if not release:
        return None

    release_config = target.get("release", {})
    body = release.get("body", "")
    tag = release.get("tag_name", "")
    pattern = release_config.get("version_pattern")
    if pattern:
        match = re.search(pattern, body, re.IGNORECASE)
        if match:
            return match.group(1)

    return extract_version(tag) or extract_version(body)


def check_updates(target, workdir="."):
    event_name = os.getenv("GITHUB_EVENT_NAME", "")
    force_build = event_name == "workflow_dispatch"

    package = get_version_info(target, workdir)
    upstream_version = package["version"]
    release = latest_release()
    current_version = release_version(release, target) if release else None
    release_id = release.get("id") if release else None
    release_tag = release.get("tag_name") if release else None
    asset_present = bool(find_matching_release_assets(release_id, target)) if release_id else False

    print(f"[INFO] Upstream version: {upstream_version}")
    print(f"[INFO] Current release version: {current_version}")
    print(f"[INFO] Matching release asset present: {asset_present}")

    if force_build:
        update_needed = True
        print("[INFO] Manual dispatch detected; forcing build.")
    elif not current_version:
        update_needed = True
        print("[INFO] No existing release version found; build is needed.")
    elif not asset_present:
        update_needed = True
        print("[INFO] Release asset is missing; rebuild is needed.")
    elif upstream_version != current_version and is_upgrade(upstream_version, current_version):
        update_needed = True
        print(f"[INFO] Version upgrade detected: {current_version} -> {upstream_version}")
    else:
        update_needed = False
        print("[INFO] No newer upstream version detected.")

    create_new_release = not release_id or (update_needed and current_version and is_major_update(upstream_version, current_version))
    minor_update = bool(update_needed and current_version and is_minor_update(upstream_version, current_version))

    values = {
        "UPDATE_NEEDED": str(update_needed).lower(),
        "UPSTREAM_VERSION": upstream_version or "",
        "CREATE_NEW_RELEASE": str(create_new_release).lower(),
        "MINOR_UPDATE": str(minor_update).lower(),
    }
    if release_id:
        values["RELEASE_ID"] = release_id
    if release_tag:
        values["RELEASE_TAG"] = release_tag
    write_env(values)
    return values


def render_release(target, workdir, version=None, build_date=None):
    workdir = Path(workdir)
    version = version or os.getenv("BUILT_VERSION") or os.getenv("BROWSER_VERSION") or os.getenv("UPSTREAM_VERSION")
    build_date = build_date or os.getenv("BUILD_DATE") or datetime.now().strftime("%Y-%m-%d")
    context = build_context(target, version=version, date=build_date)
    release_config = target.get("release", {})

    tag_template = release_config.get("tag", "v{version}")
    title_template = release_config.get("title", "{display_name} Portable v{version}")
    body_template = release_config.get(
        "body",
        "自动构建的 {display_name} 便携版\n\n构建时间: {date}\n{display_name} 版本: {version}\n",
    )

    tag = format_value(tag_template, context)
    title = format_value(title_template, context)
    body = format_value(body_template, context)

    body_path = workdir / "build" / "release_body.md"
    body_path.parent.mkdir(parents=True, exist_ok=True)
    body_path.write_text(body, encoding="utf-8")

    write_env({
        "RELEASE_TAG": tag,
        "RELEASE_TITLE": title,
        "RELEASE_BODY_PATH": str(body_path),
    })
    return {
        "tag": tag,
        "title": title,
        "body_path": str(body_path),
    }


def delete_release_asset(release_id, asset_id):
    repo = os.getenv("GITHUB_REPOSITORY")
    response = requests.delete(
        f"https://api.github.com/repos/{repo}/releases/assets/{asset_id}",
        headers=github_headers(),
        timeout=60,
    )
    if response.status_code != 204:
        raise RuntimeError(f"Failed to delete asset {asset_id}: {response.status_code} {response.text}")


def get_release_assets(release_id):
    repo = os.getenv("GITHUB_REPOSITORY")
    response = requests.get(
        f"https://api.github.com/repos/{repo}/releases/{release_id}",
        headers=github_headers(),
        timeout=60,
    )
    response.raise_for_status()
    return response.json().get("assets", [])


def find_matching_release_assets(release_id, target):
    assets = get_release_assets(release_id)
    matches, _ = matching_assets_for_target(assets, target)
    return matches


def find_latest_target_asset(release_id, target):
    matches = find_matching_release_assets(release_id, target)
    if not matches:
        return None

    def sort_key(asset):
        return (
            asset.get("updated_at")
            or asset.get("updatedAt")
            or asset.get("created_at")
            or asset.get("createdAt")
            or ""
        )

    return sorted(matches, key=sort_key, reverse=True)[0]


def download_release_asset(asset, destination):
    destination = Path(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    download_url = asset.get("browser_download_url") or asset.get("url")
    if not download_url:
        raise RuntimeError(f"Release asset is missing a download URL: {asset.get('name', '(unknown)')}")

    headers = github_headers()
    if not asset.get("browser_download_url") and asset.get("url"):
        headers = dict(headers)
        headers["Accept"] = "application/octet-stream"

    with requests.get(download_url, headers=headers, stream=True, timeout=120) as response:
        response.raise_for_status()
        with destination.open("wb") as file:
            for chunk in response.iter_content(chunk_size=1024 * 1024):
                if chunk:
                    file.write(chunk)

    return destination


def delete_assets_by_pattern(release_id, pattern):
    deleted = 0
    for asset in get_release_assets(release_id):
        name = asset.get("name", "")
        if pattern.lower() in name.lower():
            print(f"[INFO] Deleting old asset: {name}")
            delete_release_asset(release_id, asset["id"])
            deleted += 1
    print(f"[INFO] Deleted {deleted} old assets.")


def delete_target_assets(release_id, target):
    assets = get_release_assets(release_id)
    matches, description = matching_assets_for_target(assets, target)
    deleted = 0
    for asset in matches:
        name = asset.get("name", "")
        print(f"[INFO] Deleting old asset: {name}")
        delete_release_asset(release_id, asset["id"])
        deleted += 1

    if deleted == 0:
        print(f"[INFO] Deleted 0 old assets for {target.get('target', target.get('name', 'target'))} using {description or 'no matcher'}.")
    else:
        print(f"[INFO] Deleted {deleted} old assets for {target.get('target', target.get('name', 'target'))}.")


def update_release(target, workdir):
    release_id = os.getenv("RELEASE_ID")
    if not release_id:
        print("[INFO] No RELEASE_ID set; create release step will handle publishing.")
        return

    release_info = render_release(target, workdir)
    delete_target_assets(release_id, target)

    repo = os.getenv("GITHUB_REPOSITORY")
    data = {
        "body": Path(release_info["body_path"]).read_text(encoding="utf-8"),
    }
    if os.getenv("MINOR_UPDATE", "false").lower() == "true":
        data["name"] = release_info["title"]
        data["tag_name"] = release_info["tag"]

    response = requests.patch(
        f"https://api.github.com/repos/{repo}/releases/{release_id}",
        headers=github_headers(),
        json=data,
        timeout=60,
    )
    response.raise_for_status()
    print(f"[INFO] Updated release {release_id}.")
