import os
import re
from datetime import datetime
from pathlib import Path

from .builder import archive_target, build_target, format_value, get_version_info
from .config import get_target
from .github_env import write_env
from .release import (
    delete_target_assets,
    download_release_asset,
    find_latest_target_asset,
    github_headers,
    latest_release,
    render_release,
    target_match_description,
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


def release_asset_present(config, target_name, release_id):
    if not release_id:
        return False
    target = get_target(config, target_name)
    return bool(find_latest_target_asset(release_id, target))


def validate_target_asset_matchers(config, target_names):
    sample_names = {}
    for target_name in target_names:
        target = get_target(config, target_name)
        archive_name = target.get("archive_name")
        if not archive_name:
            continue
        sample_names[target_name] = format_value(
            archive_name,
            {
                "target": target_name,
                "name": target.get("name", target.get("display_name", "")),
                "display_name": target.get("display_name", target.get("name", "")),
                "output_dir": target.get("output_dir", target.get("name", "Browser")),
                "version": "123.456.789.0",
                "package_version": "123.456.789.0",
                "date": "2099-12-31",
                "arch": target.get("architecture", "x64"),
            },
        )

    conflicts = []
    for owner_name in target_names:
        owner = get_target(config, owner_name)
        for sample_name, produced_name in sample_names.items():
            if owner_name == sample_name:
                continue
            if any(matcher(produced_name) for _, matcher in target_matchers(owner)):
                conflicts.append(
                    f"{owner_name} matcher ({target_match_description(owner)}) also matches {sample_name} archive '{produced_name}'"
                )

    if conflicts:
        joined = "; ".join(conflicts)
        raise RuntimeError(f"Overlapping release asset matchers detected: {joined}")


def target_matchers(target):
    from .release import asset_matchers

    return asset_matchers(target)


def check_targets(config, target_names, workdir):
    event_name = os.getenv("GITHUB_EVENT_NAME", "")
    force_build = event_name == "workflow_dispatch"
    release = latest_release()
    release_body = release.get("body", "") if release else ""
    release_id = release.get("id") if release else None
    release_tag = release.get("tag_name") if release else None

    validate_target_asset_matchers(config, target_names)

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
        asset_present = release_asset_present(config, target_name, release_id)
        update = (
            force_build
            or not current
            or not asset_present
            or (package["version"] != current and is_upgrade(package["version"], current))
        )
        updates[target_name] = update
        print(f"[INFO] {target_name}: upstream={package['version']} current={current} asset_present={asset_present} update={update}")

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


def build_selected_targets(config, target_names, workdir, builder_dir=None):
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

        result = build_target(target, workdir, builder_dir=builder_dir)
        archive_target(target, workdir, version=result["package_version"], package_version=result["package_version"])
        built[target_name] = result["package_version"]
        write_env({
            f"{prefix}_VERSION": result["package_version"],
            f"{prefix}_BUILD_VERSION": result["version"],
            f"{prefix}_PACKAGE_VERSION": result["package_version"],
        })
    ensure_shared_release_assets(config, target_names, workdir)
    return built


def ensure_shared_release_assets(config, target_names, workdir):
    release_id = os.getenv("RELEASE_ID")
    create_new_release = os.getenv("CREATE_NEW_RELEASE", "false").lower() == "true"
    if not release_id or not create_new_release:
        return []

    assets_dir = Path(workdir) / "build" / "assets"
    assets_dir.mkdir(parents=True, exist_ok=True)
    carried = []
    for target_name in target_names:
        target = get_target(config, target_name)
        prefix = target.get("env_prefix") or env_name(target_name)
        should_build = os.getenv(f"{prefix}_UPDATE", "false").lower() == "true"
        if should_build:
            continue

        previous_asset = find_latest_target_asset(release_id, target)
        if not previous_asset:
            print(f"[WARN] No previous release asset found for {target_name}; shared release will not carry one forward.")
            continue

        destination = assets_dir / previous_asset["name"]
        if destination.exists():
            print(f"[INFO] Reusing carried asset already present: {destination.name}")
        else:
            print(f"[INFO] Carrying forward previous asset for {target_name}: {previous_asset['name']}")
            download_release_asset(previous_asset, destination)
        carried.append(str(destination))

    return carried


def render_multi_release(config, target_names, workdir):
    release_config = config.get("release")
    if not release_config:
        return render_release(get_target(config, target_names[0]), workdir)

    build_date = os.getenv("BUILD_DATE") or datetime.now().strftime("%Y-%m-%d")
    packages = {}
    for target_name in target_names:
        target = get_target(config, target_name)
        prefix = target.get("env_prefix") or env_name(target_name)
        version = (
            os.getenv(f"{prefix}_PACKAGE_VERSION")
            or os.getenv(f"{prefix}_VERSION")
            or os.getenv(f"UPSTREAM_{prefix}")
        )
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
