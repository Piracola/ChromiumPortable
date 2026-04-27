import os
import re
from datetime import datetime
from pathlib import Path

from .builder import archive_target, build_target, format_value, get_version_info
from .config import get_target
from .github_env import write_env
from .release import (
    delete_assets_by_pattern,
    github_headers,
    latest_release,
    render_release,
    update_release,
)
from .versions import is_major_update, is_minor_update, is_upgrade

import requests


def env_name(value):
    return re.sub(r"[^A-Za-z0-9]+", "_", value).strip("_").upper()


def split_targets(targets):
    return [target.strip() for target in targets.split(",") if target.strip()]


def extract_with_pattern(text, pattern):
    if not text or not pattern:
        return None
    match = re.search(pattern, text, re.IGNORECASE)
    return match.group(1) if match else None


def build_flat_context(config, target_names, packages, build_date):
    context = {"date": build_date}
    for target_name in target_names:
        package = packages[target_name]
        prefix = env_name(target_name).lower()
        context[f"{prefix}_version"] = package["version"]
    return context


def check_targets(config, target_names, workdir):
    event_name = os.getenv("GITHUB_EVENT_NAME", "")
    force_build = event_name == "workflow_dispatch"
    release = latest_release()
    release_body = release.get("body", "") if release else ""
    release_id = release.get("id") if release else None
    release_tag = release.get("tag_name") if release else None

    packages = {}
    updates = {}
    current_versions = {}
    for target_name in target_names:
        target = get_target(config, target_name)
        package = get_version_info(target, workdir)
        packages[target_name] = package

        release_config = target.get("release", {})
        current = extract_with_pattern(release_body, release_config.get("version_pattern"))
        current_versions[target_name] = current
        update = force_build or not current or (package["version"] != current and is_upgrade(package["version"], current))
        updates[target_name] = update
        print(f"[INFO] {target_name}: upstream={package['version']} current={current} update={update}")

    tag_target = config.get("release", {}).get("tag_target", target_names[0])
    create_new_release = not release_id
    if release_id and updates.get(tag_target):
        current = current_versions.get(tag_target)
        upstream = packages[tag_target]["version"]
        create_new_release = bool(current and is_major_update(upstream, current))

    minor_update = False
    if release_id and updates.get(tag_target):
        current = current_versions.get(tag_target)
        upstream = packages[tag_target]["version"]
        minor_update = bool(current and is_minor_update(upstream, current))

    values = {
        "UPDATE_NEEDED": str(any(updates.values())).lower(),
        "CREATE_NEW_RELEASE": str(create_new_release).lower(),
        "MINOR_UPDATE": str(minor_update).lower(),
    }
    if release_id:
        values["RELEASE_ID"] = release_id
    if release_tag:
        values["RELEASE_TAG"] = release_tag

    for target_name, package in packages.items():
        target = get_target(config, target_name)
        prefix = target.get("env_prefix") or env_name(target_name)
        values[f"{prefix}_UPDATE"] = str(updates[target_name]).lower()
        values[f"UPSTREAM_{prefix}"] = package["version"]

    write_env(values)
    return values


def build_selected_targets(config, target_names, workdir):
    built = {}
    for target_name in target_names:
        target = get_target(config, target_name)
        prefix = target.get("env_prefix") or env_name(target_name)
        should_build = os.getenv(f"{prefix}_UPDATE", "false").lower() == "true"
        if os.getenv("GITHUB_EVENT_NAME") == "workflow_dispatch":
            should_build = True
        if not should_build:
            print(f"[INFO] Skipping {target_name}; no update required.")
            continue

        result = build_target(target, workdir)
        archive_target(target, workdir, version=result["version"])
        built[target_name] = result["version"]
        write_env({f"{prefix}_VERSION": result["version"]})
    return built


def render_multi_release(config, target_names, workdir):
    release_config = config.get("release")
    if not release_config:
        return render_release(get_target(config, target_names[0]), workdir)

    build_date = os.getenv("BUILD_DATE") or datetime.now().strftime("%Y-%m-%d")
    packages = {}
    for target_name in target_names:
        target = get_target(config, target_name)
        prefix = target.get("env_prefix") or env_name(target_name)
        version = os.getenv(f"{prefix}_VERSION") or os.getenv(f"UPSTREAM_{prefix}")
        packages[target_name] = {"version": version}

    context = build_flat_context(config, target_names, packages, build_date)
    tag = format_value(release_config.get("tag", "v{date}"), context)
    title = format_value(release_config.get("title", "Portable Browser {date}"), context)
    body = format_value(release_config.get("body", ""), context)

    body_path = Path(workdir) / "build" / "release_body.md"
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


def update_multi_release(config, target_names, workdir):
    release_id = os.getenv("RELEASE_ID")
    if not release_id:
        print("[INFO] No RELEASE_ID set; create release step will handle publishing.")
        return

    release_info = render_multi_release(config, target_names, workdir)
    for target_name in target_names:
        target = get_target(config, target_name)
        prefix = target.get("env_prefix") or env_name(target_name)
        if os.getenv(f"{prefix}_UPDATE", "false").lower() == "true":
            asset_match = target.get("release", {}).get("asset_match")
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
