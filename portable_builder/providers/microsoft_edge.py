import re

import requests

from ..versions import compare_versions


USER_AGENT = "Microsoft Edge Update/1.3.183.29;winhttp"
EDGE_UPDATE_API = "https://msedge.api.cdp.microsoft.com/api/v2/contents/Browser/namespaces/Default/names/{0}/versions/latest?action=select"
EDGE_DOWNLOAD_API = "https://msedge.api.cdp.microsoft.com/api/v1.1/internal/contents/Browser/namespaces/Default/names/{0}/versions/{1}/files?action=GenerateDownloadInfo"


def get_version_from_microsoft_api(config):
    app_id = config.get("app_id", "msedge-stable-win-x64")
    headers = {"User-Agent": config.get("user_agent", USER_AGENT)}
    data = {
        "targetingAttributes": {
            "IsInternalUser": True,
            "Updater": "MicrosoftEdgeUpdate",
            "UpdaterVersion": "1.3.183.29",
        }
    }
    response = requests.post(
        config.get("update_api", EDGE_UPDATE_API).format(app_id),
        json=data,
        headers=headers,
        verify=config.get("verify_ssl", False),
        timeout=60,
    )
    response.raise_for_status()
    content_id = response.json().get("ContentId")
    return content_id.get("Version") if content_id else None


def get_version_from_release_repo(config):
    repo = config.get("installer_repo")
    if not repo:
        return None

    response = requests.get(f"https://api.github.com/repos/{repo}/releases/latest", timeout=60)
    if response.status_code != 200:
        return None

    tag_name = response.json().get("tag_name", "")
    match = re.search(r"(\d+\.\d+\.\d+\.\d+)", tag_name)
    return match.group(1) if match else None


def get_download_info(config, version):
    app_id = config.get("app_id", "msedge-stable-win-x64")
    headers = {"User-Agent": config.get("user_agent", USER_AGENT)}
    response = requests.post(
        config.get("download_api", EDGE_DOWNLOAD_API).format(app_id, version),
        headers=headers,
        verify=config.get("verify_ssl", False),
        timeout=60,
    )
    response.raise_for_status()
    items = response.json()
    if not items:
        raise RuntimeError("Microsoft Edge download API returned no files.")

    items.sort(key=lambda item: item.get("SizeInBytes", 0), reverse=True)
    item = items[0]
    file_name = item.get("FileId") or "MicrosoftEdgeSetup.exe"
    if not file_name.lower().endswith(".exe"):
        file_name += ".exe"
    return {
        "url": item.get("Url"),
        "file_name": file_name,
        "size": item.get("SizeInBytes"),
    }


def get_package(config):
    if not config.get("verify_ssl", False):
        requests.packages.urllib3.disable_warnings()

    ms_version = None
    repo_version = None
    try:
        ms_version = get_version_from_microsoft_api(config)
        print(f"[INFO] Microsoft Edge API version: {ms_version}")
    except Exception as exc:
        print(f"[WARN] Microsoft Edge API failed: {exc}")

    try:
        repo_version = get_version_from_release_repo(config)
        if repo_version:
            print(f"[INFO] Installer repo version: {repo_version}")
    except Exception as exc:
        print(f"[WARN] Installer repo check failed: {exc}")

    version = ms_version or repo_version
    if ms_version and repo_version and compare_versions(repo_version, ms_version) > 0:
        version = repo_version

    if not version:
        raise RuntimeError("Unable to determine Microsoft Edge version.")

    info = get_download_info(config, version)
    return {
        "version": version,
        "url": info["url"],
        "file_name": info["file_name"],
        "verify_ssl": config.get("verify_ssl", False),
    }
