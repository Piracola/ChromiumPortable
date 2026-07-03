import filecmp
import json
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
SETDLL_DIR = REPO_ROOT / "setdll"
UPSTREAM_API = "https://api.github.com/repos/Bush2021/chrome_plus/releases/latest"
REQUIRED_FILES = ("version-x64.dll", "setdll-x64.exe", "README.md", "chrome++.ini")
REBUILD_TRIGGER_FILES = {"version-x64.dll", "setdll-x64.exe"}


def github_headers():
    headers = {
        "Accept": "application/vnd.github+json",
        "User-Agent": "ChromiumPortable chrome++ updater",
    }
    token = os.getenv("GITHUB_TOKEN")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    return headers


def request_json(url):
    request = urllib.request.Request(url, headers=github_headers())
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.loads(response.read().decode("utf-8"))


def download(url, destination):
    request = urllib.request.Request(url, headers=github_headers())
    with urllib.request.urlopen(request, timeout=120) as response:
        destination.write_bytes(response.read())


def find_7z():
    configured = os.getenv("SEVEN_ZIP")
    if configured:
        return configured

    local_7zr = REPO_ROOT / "7zr.exe"
    if local_7zr.exists():
        return str(local_7zr)

    for name in ("7z", "7zz", "7za"):
        found = shutil.which(name)
        if found:
            return found

    raise RuntimeError("7-Zip not found. Install p7zip-full or provide SEVEN_ZIP.")


def extract_archive(archive_path, output_dir):
    seven_zip = find_7z()
    command = [seven_zip, "x", str(archive_path), "-y", f"-o{output_dir}"]
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode != 0:
        if result.stdout:
            print(result.stdout)
        if result.stderr:
            print(result.stderr, file=sys.stderr)
        raise RuntimeError(f"Failed to extract {archive_path}")


def find_required_file(root, name):
    matches = [path for path in root.rglob(name) if path.is_file()]
    if not matches:
        raise FileNotFoundError(f"{name} was not found in the upstream setdll archive.")
    return min(matches, key=lambda path: len(path.parts))


def set_output(name, value):
    output_path = os.getenv("GITHUB_OUTPUT")
    if output_path:
        with Path(output_path).open("a", encoding="utf-8") as file:
            file.write(f"{name}={value}\n")


def main():
    release = request_json(UPSTREAM_API)
    version = release.get("tag_name") or release.get("name") or ""
    assets = release.get("assets", [])
    asset = next((item for item in assets if item.get("name") == "setdll.7z"), None)
    if not asset:
        names = ", ".join(item.get("name", "") for item in assets)
        raise RuntimeError(f"setdll.7z was not found in chrome_plus {version}. Assets: {names}")

    SETDLL_DIR.mkdir(parents=True, exist_ok=True)
    changed_files = []

    with tempfile.TemporaryDirectory(prefix="chrome-plus-") as temp_name:
        temp_dir = Path(temp_name)
        archive_path = temp_dir / "setdll.7z"
        extract_dir = temp_dir / "extract"

        print(f"[INFO] Latest chrome++ release: {version}")
        print(f"[INFO] Downloading {asset['browser_download_url']}")
        download(asset["browser_download_url"], archive_path)
        extract_archive(archive_path, extract_dir)

        for name in REQUIRED_FILES:
            source = find_required_file(extract_dir, name)
            destination = SETDLL_DIR / name
            if not destination.exists() or not filecmp.cmp(source, destination, shallow=False):
                changed_files.append(name)
                shutil.copy2(source, destination)
                print(f"[INFO] Updated {destination.relative_to(REPO_ROOT)}")
            else:
                print(f"[INFO] Unchanged {destination.relative_to(REPO_ROOT)}")

    updated = bool(changed_files)
    rebuild_needed = any(name in REBUILD_TRIGGER_FILES for name in changed_files)
    set_output("updated", str(updated).lower())
    set_output("rebuild_needed", str(rebuild_needed).lower())
    set_output("version", version)
    set_output("changed_files", ",".join(changed_files))

    if updated:
        print(f"[INFO] chrome++ files changed: {', '.join(changed_files)}")
        if rebuild_needed:
            print("[INFO] Rebuild is needed because chrome++ binary files changed.")
        else:
            print("[INFO] Rebuild is not needed; only non-runtime files changed.")
    else:
        print("[INFO] chrome++ files are already up to date.")


if __name__ == "__main__":
    main()
