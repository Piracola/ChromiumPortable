import os
import re
from datetime import datetime
from pathlib import Path

import requests

from .builder import build_context, format_value, get_version_info
from .github_env import write_env
from .versions import is_major_update, is_minor_update, is_upgrade


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


def check_updates(target):
    event_name = os.getenv("GITHUB_EVENT_NAME", "")
    force_build = event_name == "workflow_dispatch"

    package = get_version_info(target)
    upstream_version = package["version"]
    release = latest_release()
    current_version = release_version(release, target) if release else None
    release_id = release.get("id") if release else None
    release_tag = release.get("tag_name") if release else None

    print(f"[INFO] Upstream version: {upstream_version}")
    print(f"[INFO] Current release version: {current_version}")

    if force_build:
        update_needed = True
        print("[INFO] Manual dispatch detected; forcing build.")
    elif not current_version:
        update_needed = True
        print("[INFO] No existing release version found; build is needed.")
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


def delete_assets_by_pattern(release_id, pattern):
    deleted = 0
    for asset in get_release_assets(release_id):
        name = asset.get("name", "")
        if pattern.lower() in name.lower():
            print(f"[INFO] Deleting old asset: {name}")
            delete_release_asset(release_id, asset["id"])
            deleted += 1
    print(f"[INFO] Deleted {deleted} old assets.")


def update_release(target, workdir):
    release_id = os.getenv("RELEASE_ID")
    if not release_id:
        print("[INFO] No RELEASE_ID set; create release step will handle publishing.")
        return

    release_info = render_release(target, workdir)
    release_config = target.get("release", {})
    asset_match = release_config.get("asset_match") or target.get("output_dir", target.get("name", "")).lower()
    if asset_match:
        delete_assets_by_pattern(release_id, asset_match)

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
