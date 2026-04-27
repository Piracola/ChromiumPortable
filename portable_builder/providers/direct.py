import re


def get_package(config):
    url = config.get("url")
    if not url:
        raise ValueError("direct provider requires 'url'")

    version = config.get("version")
    version_regex = config.get("version_regex")
    if not version and version_regex:
        match = re.search(version_regex, url)
        if match:
            version = match.group(1)

    if not version:
        version = "0.0.0.0"

    return {
        "version": version,
        "url": url,
        "file_name": config.get("file_name") or url.rstrip("/").split("/")[-1] or "browser-installer.exe",
        "verify_ssl": config.get("verify_ssl", True),
    }
