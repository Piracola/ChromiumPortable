import os
import shutil
import subprocess
from datetime import datetime
from pathlib import Path

from .github_env import write_env
from .providers import get_package
from .tools import (
    download_file,
    extract_with_7z,
    find_7z_tool,
    find_child_dir,
    find_child_file,
    find_version_dir,
    remove_path,
)


def format_value(template, context):
    return str(template).format(**context)


def build_context(target, version=None, date=None):
    return {
        "target": target.get("target", ""),
        "name": target.get("name", target.get("display_name", "")),
        "display_name": target.get("display_name", target.get("name", "")),
        "output_dir": target.get("output_dir", target.get("name", "Browser")),
        "version": version or "",
        "date": date or datetime.now().strftime("%Y-%m-%d"),
        "arch": target.get("architecture", "x64"),
    }


def get_version_info(target, workdir=None):
    provider_config = dict(target.get("provider", {}))
    if workdir is not None:
        provider_config["_workdir"] = str(Path(workdir))
    return get_package(provider_config)


def prepare_package(target, workdir, package):
    workdir = Path(workdir)
    temp_dir = workdir / "build" / "temp" / target["target"]
    remove_path(temp_dir)
    temp_dir.mkdir(parents=True, exist_ok=True)

    seven_zip = find_7z_tool(workdir)
    source_path = package.get("path") or package.get("installer_path")
    if source_path:
        source_path = Path(source_path)
        if not source_path.is_absolute():
            source_path = workdir / source_path
        if not source_path.exists():
            raise FileNotFoundError(f"Installer path returned by provider does not exist: {source_path}")
        installer_path = temp_dir / source_path.name
        if source_path.resolve() != installer_path.resolve():
            shutil.copy2(source_path, installer_path)
    else:
        installer_path = temp_dir / package["file_name"]
        download_file(package["url"], installer_path, verify_ssl=package.get("verify_ssl", True))
    extract_with_7z(installer_path, temp_dir, seven_zip)

    inner_archive = target.get("inner_archive")
    if inner_archive:
        inner_archive_path = find_child_file(temp_dir, inner_archive)
        if not inner_archive_path:
            raise FileNotFoundError(f"Inner archive not found: {inner_archive}")
        extract_with_7z(inner_archive_path, temp_dir, seven_zip)

    version_root_name = target.get("version_root", "Chrome-bin")
    version_root = find_child_dir(temp_dir, version_root_name)
    if not version_root:
        raise FileNotFoundError(f"Version root not found after extraction: {version_root_name}")

    version_dir = find_version_dir(version_root, package.get("version"))
    if not version_dir:
        raise FileNotFoundError(f"Version directory not found in {version_root}")

    return {
        "temp_dir": temp_dir,
        "version_root": version_root,
        "version_dir_name": version_dir.name,
        "version": version_dir.name or package["version"],
    }


def stage_app(target, workdir, prepared):
    workdir = Path(workdir)
    stage_dir = workdir / "build" / "stage" / target["target"]
    remove_path(stage_dir)
    stage_dir.mkdir(parents=True, exist_ok=True)

    output_dir_name = target.get("output_dir", target.get("name", "Browser"))
    app_root = stage_dir / output_dir_name
    layout = target.get("layout", "move_version_root")

    if layout == "move_version_dir":
        app_root.mkdir(parents=True, exist_ok=True)
        source = prepared["version_root"] / prepared["version_dir_name"]
        shutil.move(str(source), app_root / prepared["version_dir_name"])
    elif layout == "copy_version_root":
        shutil.copytree(prepared["version_root"], app_root)
    else:
        shutil.move(str(prepared["version_root"]), app_root)

    version_dir = app_root / prepared["version_dir_name"]
    if not version_dir.exists():
        raise FileNotFoundError(f"Staged version directory not found: {version_dir}")

    return {
        "stage_dir": stage_dir,
        "app_root": app_root,
        "version_dir": version_dir,
        "version": prepared["version"],
    }


def copy_chrome_plus(target, workdir, staged):
    workdir = Path(workdir)
    chrome_plus_dir = workdir / target.get("chrome_plus_dir", "chrome++")
    if not chrome_plus_dir.exists():
        raise FileNotFoundError(f"chrome++ directory not found: {chrome_plus_dir}")

    arch = target.get("architecture", "x64")
    version_dll_name = target.get("version_dll_name", f"version-{arch}.dll")
    setdll_name = target.get("setdll_name", f"setdll-{arch}.exe")

    version_dll_src = chrome_plus_dir / version_dll_name
    setdll_src = chrome_plus_dir / setdll_name
    if not version_dll_src.exists():
        raise FileNotFoundError(f"Required DLL not found: {version_dll_src}")
    if not setdll_src.exists():
        raise FileNotFoundError(f"Required setdll tool not found: {setdll_src}")

    dll_location = target.get("version_dll_location", "app_root")
    dll_dir = staged["version_dir"] if dll_location == "version_dir" else staged["app_root"]
    dll_path = dll_dir / "version.dll"
    shutil.copy(version_dll_src, dll_path)

    setdll_path = staged["app_root"] / setdll_name
    shutil.copy(setdll_src, setdll_path)

    ini_src = chrome_plus_dir / target.get("ini_name", "chrome++.ini")
    if not ini_src.exists() and (workdir / target.get("ini_name", "chrome++.ini")).exists():
        ini_src = workdir / target.get("ini_name", "chrome++.ini")

    if ini_src.exists():
        ini_location = target.get("ini_location", "app_root")
        ini_dir = staged["version_dir"] if ini_location == "version_dir" else staged["app_root"]
        shutil.copy(ini_src, ini_dir / "chrome++.ini")
    else:
        print("[WARN] chrome++.ini not found; continuing without it.")

    staged["version_dll"] = dll_path
    staged["setdll"] = setdll_path
    return staged


def inject_dll(target, staged):
    exe_name = target.get("exe_name")
    if not exe_name:
        raise ValueError("Target config requires exe_name")

    target_exe = staged["version_dir"] / exe_name
    if not target_exe.exists():
        matches = [path for path in staged["app_root"].rglob(Path(exe_name).name) if path.is_file()]
        if not matches:
            raise FileNotFoundError(f"Browser executable not found: {target_exe}")
        target_exe = min(matches, key=lambda path: len(path.relative_to(staged["app_root"]).parts))
        print(f"[INFO] Browser executable found at {target_exe}")

    setdll = staged["setdll"]
    version_dll = staged["version_dll"]
    if target.get("verify_architecture", True):
        verify_cmd = [str(setdll.resolve()), "/t", str(target_exe.resolve())]
        result = subprocess.run(verify_cmd, capture_output=True, text=True)
        print(result.stdout)

    if target.get("version_dll_location") == "version_dir":
        dll_arg = "/d:version.dll"
        cwd = target_exe.parent
    else:
        dll_arg = f"/d:{version_dll.resolve()}"
        cwd = staged["app_root"]

    cmd = [str(setdll.resolve()), dll_arg, str(target_exe.resolve())]
    print(f"[INFO] Injecting version.dll into {target_exe}")
    result = subprocess.run(cmd, capture_output=True, text=True, cwd=cwd)
    if result.stdout:
        print(result.stdout)
    if result.returncode != 0:
        if result.stderr:
            print(result.stderr)
        raise RuntimeError("DLL injection failed.")

    if target.get("remove_setdll", True):
        remove_path(setdll)


def finalize(target, workdir, staged):
    workdir = Path(workdir)
    release_dir = workdir / "build" / "release"
    output_dir_name = target.get("output_dir", target.get("name", "Browser"))
    final_app_dir = release_dir / output_dir_name

    release_dir.mkdir(parents=True, exist_ok=True)
    remove_path(final_app_dir)
    shutil.move(str(staged["app_root"]), final_app_dir)

    (final_app_dir / "version.txt").write_text(staged["version"], encoding="utf-8")

    start_script = target.get("start_script", "开始.bat")
    start_path = workdir / start_script
    if start_path.exists():
        shutil.copy(start_path, release_dir / start_path.name)

    return final_app_dir


def build_target(target, workdir):
    package = get_version_info(target, workdir)
    print(f"[INFO] Upstream version: {package['version']}")

    prepared = prepare_package(target, workdir, package)
    if prepared["version"] != package["version"]:
        print(f"[WARN] Package version {package['version']} differs from directory version {prepared['version']}; using directory version.")

    staged = stage_app(target, workdir, prepared)
    copy_chrome_plus(target, workdir, staged)
    inject_dll(target, staged)
    final_app_dir = finalize(target, workdir, staged)

    env_values = {
        "BUILT_VERSION": staged["version"],
        "BROWSER_VERSION": staged["version"],
        "OUTPUT_DIR": target.get("output_dir", target.get("name", "Browser")),
    }
    write_env(env_values)
    print(f"[INFO] Build completed: {final_app_dir}")
    return {
        "version": staged["version"],
        "output_dir": str(final_app_dir),
    }


def archive_target(target, workdir, version=None, build_date=None):
    workdir = Path(workdir)
    seven_zip = find_7z_tool(workdir)
    version = version or os.getenv("BUILT_VERSION") or os.getenv("BROWSER_VERSION")
    build_date = build_date or os.getenv("BUILD_DATE") or datetime.now().strftime("%Y-%m-%d")
    context = build_context(target, version=version, date=build_date)

    archive_template = target.get("archive_name", "{display_name}_Portable_{version}_{date}.7z")
    archive_name = format_value(archive_template, context)
    assets_dir = workdir / "build" / "assets"
    assets_dir.mkdir(parents=True, exist_ok=True)
    archive_path = assets_dir / archive_name
    remove_path(archive_path)

    release_dir = workdir / "build" / "release"
    if not release_dir.exists():
        raise FileNotFoundError(f"Release directory not found: {release_dir}")

    cmd = [str(seven_zip), "a", "-t7z", "-mx=9", str(archive_path.resolve()), "*"]
    print(f"[INFO] Creating archive: {archive_path}")
    result = subprocess.run(cmd, cwd=release_dir, capture_output=True, text=True)
    if result.stdout:
        print(result.stdout)
    if result.returncode != 0:
        if result.stderr:
            print(result.stderr)
        raise RuntimeError("Archive creation failed.")

    write_env({"ARCHIVE_NAME": archive_name, "ASSET_PATH": str(archive_path)})
    return archive_path
