"""Post-archive verification: check the artifact users actually download.

The build stage already asserts the injected import is portable, but that only
covers the staging tree. This module re-checks the finished .7z end to end:
extract it, re-run the import assertion, then actually launch the browser and
confirm Chrome++ redirected its profile into the portable data directory. That
last step is the only automated proof that "portable" is true.
"""

import os
import subprocess
import time
from pathlib import Path

from .config import get_target
from .github_env import write_env
from .multi import env_name
from .release import archive_name_regex
from .tools import (
    assert_portable_version_import,
    extract_with_7z,
    find_7z_tool,
    find_version_dir,
    remove_path,
    sha256_file,
)

DEFAULT_SMOKE_ARGS = (
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    "--no-default-browser-check",
    "--dump-dom",
    "about:blank",
)


def find_target_archive(target, workdir):
    assets_dir = Path(workdir) / "build" / "assets"
    if not assets_dir.exists():
        raise FileNotFoundError(f"Assets directory not found: {assets_dir}")

    regex = archive_name_regex(target)
    matches = [path for path in assets_dir.glob("*.7z") if regex.fullmatch(path.name)]
    if not matches:
        available = ", ".join(sorted(path.name for path in assets_dir.glob("*.7z"))) or "(none)"
        raise FileNotFoundError(
            f"No archive matching {target.get('archive_name')} in {assets_dir}. Present: {available}"
        )

    return max(matches, key=lambda path: path.stat().st_mtime)


def locate_executable(target, app_root):
    version_dir = find_version_dir(app_root)
    if version_dir is None:
        raise FileNotFoundError(f"No version directory found under {app_root}")

    exe_name = target.get("exe_name")
    if not exe_name:
        raise ValueError("Target config requires exe_name")

    executable = Path(os.path.normpath(version_dir / exe_name))
    if not executable.exists():
        raise FileNotFoundError(f"Browser executable not found in archive: {executable}")
    return executable


def run_browser(executable, args, timeout, cwd):
    command = [str(executable), *args]
    print(f"[INFO] Running {' '.join(command)}")
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=timeout, cwd=str(cwd))
    except subprocess.TimeoutExpired as exc:
        raise RuntimeError(f"Browser did not exit within {timeout}s: {' '.join(command)}") from exc

    if result.returncode != 0:
        if result.stdout:
            print(result.stdout[:2000])
        if result.stderr:
            print(result.stderr[:2000])
        raise RuntimeError(f"Browser exited with code {result.returncode}: {' '.join(command)}")
    return result


def assert_no_stray_backups(extracted_root):
    """setdll leaves '<exe>~' backups; they must never reach an archive."""
    leftovers = sorted(path.relative_to(extracted_root).as_posix() for path in extracted_root.rglob("*~") if path.is_file())
    if leftovers:
        raise RuntimeError(f"Archive contains leftover backup files: {', '.join(leftovers)}")


def smoke_test(target, extracted_root, app_root, executable):
    data_dir_name = target.get("smoke_data_dir", "Data")
    data_dir = extracted_root / data_dir_name
    if data_dir.exists():
        raise RuntimeError(f"Archive already ships a '{data_dir_name}' directory: {data_dir}")

    version_file = app_root / "version.txt"
    if version_file.exists():
        # Not asserted against the browser's own reported version: Helium's
        # package version and its bundled Chromium version differ by design.
        print(f"[INFO] version.txt records: {version_file.read_text(encoding='utf-8').strip()}")

    # Chromium is a GUI-subsystem binary, so a captured pipe stays empty; the
    # exit code is the signal that the patched PE loaded our DLL successfully.
    run_browser(executable, ["--version"], target.get("smoke_timeout", 120), app_root)

    args = list(target.get("smoke_args", DEFAULT_SMOKE_ARGS))
    run_browser(executable, args, target.get("smoke_timeout", 180), app_root)

    if not data_dir.is_dir():
        present = ", ".join(sorted(item.name for item in extracted_root.iterdir())) or "(empty)"
        raise RuntimeError(
            f"Chrome++ did not create the portable data directory '{data_dir_name}', so the profile "
            f"went to the user profile instead. Archive root contains: {present}"
        )
    print(f"[INFO] Portable data directory created inside the archive: {data_dir}")


def cleanup(extracted_root, attempts=4, delay=3):
    """Browser subprocesses outlive the parent briefly and hold DLL handles."""
    for attempt in range(attempts):
        try:
            remove_path(extracted_root)
            return True
        except OSError as exc:
            if attempt == attempts - 1:
                print(f"[WARN] Could not clean up {extracted_root}: {exc}")
                return False
            time.sleep(delay)
    return False


def verify_target(target, workdir, archive=None, smoke=True):
    workdir = Path(workdir)
    archive = Path(archive) if archive else find_target_archive(target, workdir)
    print(f"[INFO] Verifying archive: {archive.name} ({archive.stat().st_size} bytes)")
    print(f"[INFO] Archive SHA256: {sha256_file(archive)}")

    extracted_root = workdir / "build" / "verify" / target["target"]
    remove_path(extracted_root)
    extracted_root.mkdir(parents=True, exist_ok=True)
    extract_with_7z(archive, extracted_root, find_7z_tool(workdir))

    output_dir_name = target.get("output_dir", target.get("name", "Browser"))
    app_root = extracted_root / output_dir_name
    if not app_root.is_dir():
        present = ", ".join(sorted(item.name for item in extracted_root.iterdir())) or "(empty)"
        raise FileNotFoundError(f"Archive does not contain '{output_dir_name}'. Root contains: {present}")

    executable = locate_executable(target, app_root)
    assert_portable_version_import(executable)
    assert_no_stray_backups(extracted_root)

    if smoke:
        smoke_test(target, extracted_root, app_root, executable)
    else:
        print("[INFO] Smoke launch disabled; import table check only.")

    cleanup(extracted_root)

    print(f"[INFO] Verification passed: {archive.name}")
    return {"archive": str(archive), "executable": str(executable)}


def verify_targets(config, target_names, workdir, smoke=True):
    verified = {}
    for target_name in target_names:
        target = get_target(config, target_name)
        prefix = target.get("env_prefix") or env_name(target_name)
        updated = os.getenv(f"{prefix}_UPDATE", "").lower() == "true"
        forced = os.getenv("GITHUB_EVENT_NAME") == "workflow_dispatch"
        if os.getenv(f"{prefix}_UPDATE") is not None and not (updated or forced):
            print(f"[INFO] Skipping {target_name}; it was not rebuilt in this run.")
            continue

        verified[target_name] = verify_target(target, workdir, smoke=smoke)

    if verified:
        write_env({"VERIFIED_TARGETS": ",".join(sorted(verified))})
    return verified
