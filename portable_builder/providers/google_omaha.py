import xml.etree.ElementTree as tree

import requests


DEFAULT_UPDATE_URL = "https://tools.google.com/service/update2"

CHANNELS = {
    "win_stable_x64": {
        "os": 'platform="win" version="10.0" sp="" arch="x64"',
        "app": 'appid="{8A69D345-D564-463C-AFF1-A69D9E530F96}" version="" nextversion="" lang="en" brand="" installage="-1" installdate="-1" iid="{11111111-1111-1111-1111-111111111111}"',
    },
    "win_beta_x64": {
        "os": 'platform="win" version="10.0" arch="x64"',
        "app": 'appid="{8A69D345-D564-463C-AFF1-A69D9E530F96}" ap="x64-beta-multi-chrome"',
    },
    "win_dev_x64": {
        "os": 'platform="win" version="10.0" arch="x64"',
        "app": 'appid="{8A69D345-D564-463C-AFF1-A69D9E530F96}" ap="x64-dev-multi-chrome"',
    },
    "win_canary_x64": {
        "os": 'platform="win" version="10.0" arch="x64"',
        "app": 'appid="{4EA16AC7-FD5A-47C3-875B-DBF4A2008C20}" ap="x64-canary"',
    },
}


def post_update(update_url, os_xml, app_xml):
    xml_data = f'''<?xml version="1.0" encoding="UTF-8"?>
<request protocol="3.0" updater="Omaha" updaterversion="1.3.36.372" shell_version="1.3.36.352" ismachine="0" sessionid="{{11111111-1111-1111-1111-111111111111}}" installsource="taggedmi" requestid="{{11111111-1111-1111-1111-111111111111}}" dedup="cr" domainjoined="0">
  <hw physmemory="16" sse="1" sse2="1" sse3="1" ssse3="1" sse41="1" sse42="1" avx="1"/>
  <os {os_xml}/>
  <app {app_xml}>
    <updatecheck/>
    <data name="install" index="empty"/>
  </app>
</request>'''
    response = requests.post(update_url, data=xml_data, timeout=60)
    response.raise_for_status()
    return response.text


def decode_response(text):
    root = tree.fromstring(text)
    manifest_node = root.find(".//manifest")
    if manifest_node is None:
        raise RuntimeError("No manifest found in Google Omaha response.")

    package_node = root.find(".//package")
    if package_node is None:
        raise RuntimeError("No package found in Google Omaha response.")

    package_name = package_node.get("name")
    urls = []
    for node in root.findall(".//url"):
        codebase = node.get("codebase")
        if codebase:
            urls.append(codebase + package_name)

    if not urls:
        raise RuntimeError("No download URL found in Google Omaha response.")

    return {
        "version": manifest_node.get("version"),
        "urls": urls,
        "file_name": package_name,
    }


def get_package(config):
    channel = config.get("channel", "win_stable_x64")
    channel_config = dict(CHANNELS.get(channel, {}))
    channel_config.update(config.get("request", {}))

    if not channel_config.get("os") or not channel_config.get("app"):
        raise KeyError(f"Google Omaha channel '{channel}' is not configured.")

    data = decode_response(post_update(config.get("update_url", DEFAULT_UPDATE_URL), channel_config["os"], channel_config["app"]))
    preferred_hosts = config.get("preferred_hosts", ["https://dl.google.com"])
    download_url = data["urls"][0]
    for candidate in data["urls"]:
        if any(candidate.startswith(host) for host in preferred_hosts):
            download_url = candidate
            break

    return {
        "version": data["version"],
        "url": download_url,
        "file_name": data["file_name"] or "chrome.7z.exe",
        "verify_ssl": config.get("verify_ssl", True),
    }
