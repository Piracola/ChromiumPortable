import re


def get_package(config):
    url = config.get("url")
    path = config.get("path") or config.get("installer_path")
    if not url and not path:
        raise ValueError("direct provider requires 'url' or 'path'")

    version = config.get("version")
    version_regex = config.get("version_regex")
    if not version and version_regex and url:
        match = re.search(version_regex, url)
        if match:
            version = match.group(1)

    if not version:
        version = "0.0.0.0"

    package = {
        "version": version,
        "file_name": config.get("file_name") or (url.rstrip("/").split("/")[-1] if url else None) or "browser-installer.exe",
        "verify_ssl": config.get("verify_ssl", True),
        "sha256": config.get("sha256"),
        "size": config.get("size"),
    }
    if path:
        package["path"] = path
    else:
        package["url"] = url
    return package
