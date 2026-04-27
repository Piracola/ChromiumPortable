import os
import shutil
import subprocess
import sys
from pathlib import Path

import requests


SEVEN_ZIP_URLS = (
    "https://www.7-zip.org/a/7zr.exe",
    "https://raw.githubusercontent.com/develar/7zip-bin/master/win/x64/7za.exe",
)
LOCAL_7ZR = "7zr.exe"
SYSTEM_7Z_PATHS = (
    r"C:\Program Files\7-Zip\7z.exe",
    r"C:\Program Files (x86)\7-Zip\7z.exe",
)


def configure_stdout():
    if hasattr(sys.stdout, "reconfigure") and sys.stdout.encoding != "utf-8":
        sys.stdout.reconfigure(encoding="utf-8")


def download_file(url, path, verify_ssl=True, skip_existing=True):
    path = Path(path)
    if skip_existing and path.exists():
        print(f"[INFO] File exists, skipping download: {path}")
        return path

    path.parent.mkdir(parents=True, exist_ok=True)
    print(f"[INFO] Downloading {url}")
    with requests.get(url, stream=True, verify=verify_ssl, timeout=120) as response:
        response.raise_for_status()
        with path.open("wb") as file:
            for chunk in response.iter_content(chunk_size=1024 * 1024):
                if chunk:
                    file.write(chunk)
    return path


def install_7z_with_chocolatey():
    if not shutil.which("choco"):
        return None

    print("[INFO] Trying to install 7-Zip with Chocolatey.")
    result = subprocess.run(
        ["choco", "install", "7zip", "-y", "--no-progress"],
        capture_output=True,
        text=True,
    )
    if result.stdout:
        print(result.stdout)
    if result.returncode != 0:
        if result.stderr:
            print(result.stderr)
        return None

    for path in SYSTEM_7Z_PATHS:
        if Path(path).exists():
            return path
    if shutil.which("7z"):
        return "7z"
    return None


def find_7z_tool(workdir):
    for path in SYSTEM_7Z_PATHS:
        if Path(path).exists():
            print(f"[INFO] Using system 7-Zip: {path}")
            return path

    local_7zr = Path(workdir) / LOCAL_7ZR
    if local_7zr.exists():
        print(f"[INFO] Using local 7zr.exe: {local_7zr}")
        return str(local_7zr)

    if shutil.which("7z"):
        print("[INFO] Using 7z from PATH")
        return "7z"

    print("[INFO] 7-Zip not found. Downloading standalone extractor.")
    last_error = None
    for url in SEVEN_ZIP_URLS:
        try:
            download_file(url, local_7zr, skip_existing=False)
            return str(local_7zr)
        except Exception as exc:
            last_error = exc
            print(f"[WARN] Failed to download 7-Zip from {url}: {exc}")
            remove_path(local_7zr)

    chocolatey_7z = install_7z_with_chocolatey()
    if chocolatey_7z:
        print(f"[INFO] Using Chocolatey-installed 7-Zip: {chocolatey_7z}")
        return chocolatey_7z

    raise RuntimeError(f"Unable to locate or install 7-Zip. Last download error: {last_error}")


def extract_with_7z(archive, output_dir, seven_zip_path):
    output_dir = Path(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    cmd = [str(seven_zip_path), "x", str(archive), "-y", f"-o{output_dir}"]
    print(f"[INFO] Extracting {archive}")
    result = subprocess.run(cmd, capture_output=True, text=True)
    if result.returncode != 0:
        print(result.stdout)
        print(result.stderr)
        raise RuntimeError(f"Extraction failed: {archive}")


def find_child_dir(root, name):
    root = Path(root)
    direct = root / name
    if direct.exists():
        return direct

    name_lower = name.lower()
    for item in root.rglob("*"):
        if item.is_dir() and item.name.lower() == name_lower:
            return item
    return None


def find_child_file(root, name):
    root = Path(root)
    direct = root / name
    if direct.exists():
        return direct

    name_lower = name.lower()
    for item in root.rglob("*"):
        if item.is_file() and item.name.lower() == name_lower:
            return item
    return None


def find_version_dir(root, preferred_version=None):
    root = Path(root)
    if preferred_version and (root / preferred_version).is_dir():
        return root / preferred_version

    for item in root.iterdir():
        if item.is_dir() and item.name and item.name[0].isdigit():
            if all(char.isdigit() or char == "." for char in item.name):
                return item
    return None


def remove_path(path):
    path = Path(path)
    if path.is_dir():
        shutil.rmtree(path)
    elif path.exists():
        path.unlink()
