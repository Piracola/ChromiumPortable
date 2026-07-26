import base64
import binascii
import hashlib
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path, PureWindowsPath

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
ABSOLUTE_VERSION_DLL = re.compile(rb"[A-Za-z]:\\[^\x00]*version\.dll", re.IGNORECASE)


def configure_stdout():
    if hasattr(sys.stdout, "reconfigure") and sys.stdout.encoding != "utf-8":
        sys.stdout.reconfigure(encoding="utf-8")


def human_size(size):
    """Format bytes the way Windows Explorer does (1024-based, labelled MB)."""
    if size in (None, ""):
        return ""
    value = float(size)
    for unit in ("B", "KB", "MB", "GB"):
        if value < 1024 or unit == "GB":
            return f"{value:.0f} {unit}" if unit == "B" else f"{value:.1f} {unit}"
        value /= 1024
    return f"{value:.1f} GB"


def sha256_file(path, chunk_size=1024 * 1024):
    digest = hashlib.sha256()
    with Path(path).open("rb") as file:
        for chunk in iter(lambda: file.read(chunk_size), b""):
            digest.update(chunk)
    return digest.hexdigest()


def normalize_sha256(value):
    """Accept hex, 'sha256:<hex>' and base64 digests; return lowercase hex."""
    if not value:
        return None

    text = str(value).strip()
    if ":" in text:
        text = text.split(":", 1)[1].strip()

    if len(text) == 64:
        try:
            return bytes.fromhex(text).hex()
        except ValueError:
            pass

    try:
        raw = base64.b64decode(text, validate=True)
    except (binascii.Error, ValueError) as exc:
        raise ValueError(f"Unrecognized SHA256 digest: {value!r}") from exc
    if len(raw) != 32:
        raise ValueError(f"Unrecognized SHA256 digest: {value!r}")
    return raw.hex()


def verify_file_digest(path, sha256=None, size=None):
    path = Path(path)
    if size is not None:
        actual_size = path.stat().st_size
        if int(size) != actual_size:
            raise RuntimeError(f"Size mismatch for {path.name}: expected {size} bytes, got {actual_size}")

    if sha256:
        expected = normalize_sha256(sha256)
        actual = sha256_file(path)
        if actual != expected:
            raise RuntimeError(f"SHA256 mismatch for {path.name}: expected {expected}, got {actual}")
        print(f"[INFO] SHA256 verified: {path.name} {actual}")


def download_file(url, path, verify_ssl=True, skip_existing=True, sha256=None, size=None, warn_unverified=True):
    path = Path(path)
    if skip_existing and path.exists():
        try:
            verify_file_digest(path, sha256=sha256, size=size)
        except RuntimeError as exc:
            print(f"[WARN] Cached download rejected, fetching again: {exc}")
            remove_path(path)
        else:
            print(f"[INFO] File exists, skipping download: {path}")
            return path

    path.parent.mkdir(parents=True, exist_ok=True)
    print(f"[INFO] Downloading {url}")
    digest = hashlib.sha256()
    downloaded = 0
    with requests.get(url, stream=True, verify=verify_ssl, timeout=120) as response:
        response.raise_for_status()
        with path.open("wb") as file:
            for chunk in response.iter_content(chunk_size=1024 * 1024):
                if chunk:
                    file.write(chunk)
                    digest.update(chunk)
                    downloaded += len(chunk)

    if size is not None and int(size) != downloaded:
        remove_path(path)
        raise RuntimeError(f"Size mismatch for {path.name}: expected {size} bytes, got {downloaded}")

    if sha256:
        expected = normalize_sha256(sha256)
        actual = digest.hexdigest()
        if actual != expected:
            remove_path(path)
            raise RuntimeError(f"SHA256 mismatch for {path.name}: expected {expected}, got {actual}")
        print(f"[INFO] SHA256 verified: {path.name} {actual}")
    elif warn_unverified:
        print(f"[WARN] Upstream provided no SHA256 for {path.name}; downloaded bytes are unverified.")

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
            download_file(url, local_7zr, skip_existing=False, warn_unverified=False)
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


def _section_for_rva(sections, rva):
    for section in sections:
        span = max(section["virtual_size"], section["raw_size"])
        if span and section["virtual_address"] <= rva < section["virtual_address"] + span:
            return section
    return None


def read_pe_sections(data):
    if data[:2] != b"MZ":
        raise RuntimeError("Not a PE image (missing MZ signature).")

    pe_offset = int.from_bytes(data[0x3C:0x40], "little")
    if data[pe_offset:pe_offset + 4] != b"PE\0\0":
        raise RuntimeError("Not a PE image (missing PE signature).")

    coff = pe_offset + 4
    section_count = int.from_bytes(data[coff + 2:coff + 4], "little")
    optional_size = int.from_bytes(data[coff + 16:coff + 18], "little")
    optional = coff + 20
    magic = int.from_bytes(data[optional:optional + 2], "little")
    if magic == 0x20B:
        data_directories = optional + 112
    elif magic == 0x10B:
        data_directories = optional + 96
    else:
        raise RuntimeError(f"Unsupported PE optional header magic 0x{magic:x}.")

    sections = []
    section_table = optional + optional_size
    for index in range(section_count):
        entry = section_table + index * 40
        sections.append({
            "name": data[entry:entry + 8].rstrip(b"\0").decode("ascii", errors="replace"),
            "virtual_size": int.from_bytes(data[entry + 8:entry + 12], "little"),
            "virtual_address": int.from_bytes(data[entry + 12:entry + 16], "little"),
            "raw_size": int.from_bytes(data[entry + 16:entry + 20], "little"),
            "raw_pointer": int.from_bytes(data[entry + 20:entry + 24], "little"),
        })

    # Data directory entry 1 is IMAGE_DIRECTORY_ENTRY_IMPORT.
    import_rva = int.from_bytes(data[data_directories + 8:data_directories + 12], "little")
    return sections, import_rva


def read_pe_import_names(path):
    """Return the DLL name strings from a PE image's import directory, in table order."""
    data = Path(path).read_bytes()
    sections, import_rva = read_pe_sections(data)
    if not import_rva:
        return [], None

    host_section = _section_for_rva(sections, import_rva)
    if host_section is None:
        raise RuntimeError(f"Import directory RVA 0x{import_rva:x} falls outside every section: {path}")

    names = []
    descriptor = host_section["raw_pointer"] + (import_rva - host_section["virtual_address"])
    while True:
        entry = data[descriptor:descriptor + 20]
        if len(entry) < 20 or entry == b"\0" * 20:
            break
        name_rva = int.from_bytes(entry[12:16], "little")
        name_section = _section_for_rva(sections, name_rva)
        if name_section is not None:
            offset = name_section["raw_pointer"] + (name_rva - name_section["virtual_address"])
            terminator = data.index(b"\0", offset)
            names.append(data[offset:terminator].decode("ascii", errors="replace"))
        descriptor += 20

    return names, host_section["name"]


def assert_portable_version_import(path, dll_name="version.dll"):
    """Assert the browser executable really loads our injected, relatively-referenced DLL.

    Checking only that some `version.dll` import exists is not enough: Chromium
    natively imports the system `VERSION.dll`, so that test passes even when
    injection silently did nothing. setdll (Detours) prepends the injected DLL so
    the loader resolves it first, so the first import descriptor is the signal.
    """
    path = Path(path)
    imports, import_section = read_pe_import_names(path)
    listed = ", ".join(imports) or "(none)"
    if not imports:
        raise RuntimeError(f"{path} has no import directory; it cannot load {dll_name}.")

    first = imports[0]
    if PureWindowsPath(first).name.lower() != dll_name.lower():
        raise RuntimeError(
            f"{path} does not load {dll_name} first, so DLL injection did not take effect. "
            f"Imports: {listed}"
        )

    if PureWindowsPath(first).drive or first.startswith(("\\", "/")):
        raise RuntimeError(f"{path} imports {dll_name} through a non-portable path: {first}")

    raw_matches = sorted(
        {
            match.group(0).decode("ascii", errors="replace")
            for match in ABSOLUTE_VERSION_DLL.finditer(path.read_bytes())
        }
    )
    if raw_matches:
        raise RuntimeError(f"{path} contains non-portable version.dll import: {raw_matches[0]}")

    print(f"[INFO] Import table verified (directory in '{import_section}'): {listed}")
    return imports
