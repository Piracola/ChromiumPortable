import json
import re
import subprocess
from pathlib import Path

from .brave_bundle import extract_brave_metainstaller
from .pe import read_pe_machine, read_version_info
from .tools import extract_with_7z, read_pe_import_names, remove_path


VERSION_DIR = re.compile(r"^\d+(?:\.\d+){1,5}$")
ARCHIVE_SUFFIXES = {".7z", ".zip", ".rar", ".cab", ".msi"}
KNOWN_BROWSER_EXES = {
    "360chromex.exe",
    "brave.exe",
    "chrome.exe",
    "chromium.exe",
    "helium.exe",
    "msedge.exe",
    "opera.exe",
    "thorium.exe",
    "vivaldi.exe",
}
REJECT_NAME_TOKENS = {
    "broker",
    "crash",
    "driver",
    "elevation",
    "helper",
    "install",
    "launcher",
    "notification",
    "ondemand",
    "proxy",
    "register",
    "report",
    "service",
    "setup",
    "shell",
    "unins",
    "update",
    "vpn",
    "wer",
    "wireguard",
}
REJECT_PATH_TOKENS = {
    "installer",
    "update",
    "win_clang_x86",
}
CHROMIUM_MARKERS = {
    "chrome.dll",
    "chrome_elf.dll",
    "icudtl.dat",
    "opera_elf.dll",
    "resources.pak",
    "vivaldi_elf.dll",
}


def _is_pe(path):
    try:
        with Path(path).open("rb") as file:
            return file.read(2) == b"MZ"
    except OSError:
        return False


def _safe_name(path):
    return re.sub(r"[^A-Za-z0-9._-]+", "_", Path(path).name)[:80]


def _is_brave_metainstaller(path):
    if not _is_pe(path):
        return False
    info = read_version_info(path)
    product = info.get("ProductName", "").casefold()
    original = info.get("OriginalFilename", "").casefold()
    return "brave" in product and "update" in product and "updatesetup" in original


def _seven_zip_can_open(path, seven_zip):
    result = subprocess.run(
        [str(seven_zip), "l", str(path)],
        capture_output=True,
        text=True,
        errors="replace",
    )
    return result.returncode == 0 and "Type =" in result.stdout


def _nested_archive_candidates(root, seven_zip):
    root = Path(root)
    likely = []
    for path in root.rglob("*"):
        if not path.is_file() or path.stat().st_size < 64 * 1024:
            continue
        relative = path.relative_to(root)
        if len(relative.parts) > 3:
            continue
        suffix = path.suffix.casefold()
        name = path.name.casefold()
        if suffix in ARCHIVE_SUFFIXES:
            likely.append((0 if name in {"chrome.7z", "vivaldi.7z", "browser.7z"} else 1, path))
            continue
        if _is_pe(path) and any(token in name for token in ("installer", "setup", "package")):
            info = read_version_info(path)
            description = " ".join(info.values()).casefold()
            if "installer" in description or "setup" in description:
                likely.append((2, path))

    opened = []
    for _, path in sorted(likely, key=lambda item: (item[0], len(item[1].parts), item[1].name.casefold())):
        if _seven_zip_can_open(path, seven_zip):
            opened.append(path)
    return opened


def _direct_names(directory):
    try:
        return {item.name.casefold() for item in Path(directory).iterdir()}
    except OSError:
        return set()


def _numeric_children(directory):
    try:
        return [item for item in Path(directory).iterdir() if item.is_dir() and VERSION_DIR.fullmatch(item.name)]
    except OSError:
        return []


def _descendant_marker_names(directory, numeric_children):
    markers = set()
    for child in numeric_children[:4]:
        markers.update(_direct_names(child) & CHROMIUM_MARKERS)
    return markers


def _score_candidate(path, desired_arch="x64"):
    path = Path(path)
    name = path.name.casefold()
    stem = path.stem.casefold()
    info = read_version_info(path)
    metadata = " ".join(info.values()).casefold()
    reasons = []
    score = 0

    try:
        architecture = read_pe_machine(path)
    except RuntimeError:
        return None
    if architecture == desired_arch:
        score += 15
        reasons.append(f"架构为 {architecture}")
    elif desired_arch and architecture != desired_arch:
        score -= 100
        reasons.append(f"架构为 {architecture}，目标为 {desired_arch}")

    if name in KNOWN_BROWSER_EXES:
        score += 120
        reasons.append("文件名是常见浏览器主程序")
    if any(token in stem for token in REJECT_NAME_TOKENS):
        score -= 180
        reasons.append("文件名像安装器、更新器或辅助程序")
    path_tokens = {part.casefold() for part in path.parent.parts[-3:]}
    if path_tokens & REJECT_PATH_TOKENS:
        score -= 70
        reasons.append("位于安装器或兼容组件目录")

    product_text = " ".join(
        info.get(field, "")
        for field in ("ProductName", "FileDescription", "OriginalFilename")
    ).casefold()
    if any(token in product_text for token in ("installer", "setup", "update", "helper", "crash", "service")):
        score -= 140
        reasons.append("文件说明表明它不是浏览器主程序")
    if any(token in metadata for token in ("browser", "chromium", "chrome", "vivaldi", "opera", "brave", "thorium")):
        score += 25
        reasons.append("产品信息表明它属于 Chromium 浏览器")
    original = Path(info.get("OriginalFilename", "")).name.casefold()
    if original and original == name:
        score += 15
        reasons.append("原始文件名与当前文件一致")

    direct = _direct_names(path.parent)
    numeric_children = _numeric_children(path.parent)
    child_markers = _descendant_marker_names(path.parent, numeric_children)
    direct_markers = direct & CHROMIUM_MARKERS
    if direct_markers:
        score += 35
        reasons.append("同目录存在 Chromium 核心文件")
    if numeric_children and child_markers:
        score += 45
        reasons.append("同目录存在版本资源目录")
    if VERSION_DIR.fullmatch(path.parent.name) and direct_markers:
        score += 20
        reasons.append("位于完整的版本资源目录")

    try:
        imports, _ = read_pe_import_names(path)
    except (OSError, RuntimeError, ValueError):
        imports = []
    lowered_imports = {item.casefold() for item in imports}
    elf_imports = lowered_imports & {"chrome_elf.dll", "vivaldi_elf.dll", "opera_elf.dll"}
    if elf_imports:
        score += 55
        reasons.append("直接加载浏览器 ELF 组件")

    if not imports:
        score -= 15
        reasons.append("没有可用的 PE 导入表")

    version = info.get("ProductVersion") or info.get("FileVersion") or ""
    product = info.get("ProductName") or info.get("FileDescription") or path.stem
    return {
        "path": path,
        "score": score,
        "reasons": reasons,
        "architecture": architecture,
        "version": version,
        "product": product,
        "version_info": info,
        "imports": imports,
        "has_chromium_evidence": bool(direct_markers or child_markers or elf_imports),
    }


def find_browser_candidates(root, desired_arch="x64"):
    root = Path(root)
    candidates = []
    for path in root.rglob("*"):
        if not path.is_file() or path.suffix.casefold() != ".exe" or not _is_pe(path):
            continue
        candidate = _score_candidate(path, desired_arch=desired_arch)
        if candidate is not None:
            candidates.append(candidate)
    return sorted(candidates, key=lambda item: (-item["score"], len(item["path"].parts), item["path"].name.casefold()))


def select_browser_candidate(candidates, minimum_score=80, ambiguity_margin=15):
    if not candidates or candidates[0]["score"] < minimum_score or not candidates[0]["has_chromium_evidence"]:
        summary = ", ".join(f"{item['path'].name} ({item['score']})" for item in candidates[:8]) or "没有 EXE 候选"
        raise RuntimeError(f"没有找到可信的浏览器主程序：{summary}")
    best = candidates[0]
    if len(candidates) > 1 and candidates[1]["score"] >= best["score"] - ambiguity_margin:
        tied = ", ".join(f"{item['path']} ({item['score']})" for item in candidates[:5] if item["score"] >= best["score"] - ambiguity_margin)
        raise RuntimeError(f"存在多个接近的主程序候选，已停止自动选择：{tied}")
    return best


def extract_package_layers(package_path, extraction_root, seven_zip, desired_arch="x64", max_depth=4, max_archives=16):
    package_path = Path(package_path)
    extraction_root = Path(extraction_root)

    if package_path.is_dir():
        return [{"source": package_path, "root": package_path, "depth": 0, "kind": "directory"}]

    remove_path(extraction_root)
    extraction_root.mkdir(parents=True, exist_ok=True)

    first_root = extraction_root / "layer-00-package"
    if _is_brave_metainstaller(package_path):
        print(f"[INFO] Statically extracting Brave/Omaha bundle: {package_path.name}")
        extract_brave_metainstaller(package_path, first_root)
        first_kind = "brave_omaha"
    else:
        extract_with_7z(package_path, first_root, seven_zip)
        first_kind = "archive"

    layers = [{"source": package_path, "root": first_root, "depth": 0, "kind": first_kind}]
    queue = [layers[0]]
    processed = set()
    sequence = 1
    while queue and sequence <= max_archives:
        layer = queue.pop(0)
        if layer["depth"] >= max_depth:
            continue
        for archive in _nested_archive_candidates(layer["root"], seven_zip):
            key = str(archive.resolve()).casefold()
            if key in processed:
                continue
            processed.add(key)
            child_root = extraction_root / f"layer-{sequence:02d}-{_safe_name(archive)}"
            sequence += 1
            print(f"[INFO] Recursively extracting nested package: {archive.name}")
            extract_with_7z(archive, child_root, seven_zip)
            child = {
                "source": archive,
                "root": child_root,
                "depth": layer["depth"] + 1,
                "kind": "nested_archive",
            }
            layers.append(child)
            queue.append(child)
            if sequence > max_archives:
                break
    return layers


def _infer_version_dir(app_root, executable, version):
    app_root = Path(app_root)
    executable = Path(executable)
    if VERSION_DIR.fullmatch(executable.parent.name):
        return executable.parent
    if version and (app_root / version).is_dir():
        return app_root / version
    numeric = _numeric_children(app_root)
    if numeric:
        return sorted(numeric, key=lambda path: path.name, reverse=True)[0]
    return app_root


def analyze_package(package_path, extraction_root, seven_zip, desired_arch="x64"):
    layers = extract_package_layers(
        package_path,
        extraction_root,
        seven_zip,
        desired_arch=desired_arch,
    )
    candidates = []
    for layer in layers:
        for candidate in find_browser_candidates(layer["root"], desired_arch=desired_arch):
            candidate["layer_root"] = layer["root"]
            candidate["layer_source"] = layer["source"]
            candidates.append(candidate)
    candidates.sort(key=lambda item: (-item["score"], len(item["path"].parts), item["path"].name.casefold()))
    selected = select_browser_candidate(candidates)
    app_root = selected["path"].parent
    version = selected["version"] or "0.0.0.0"
    version_dir = _infer_version_dir(app_root, selected["path"], version)
    return {
        "package": Path(package_path),
        "layers": layers,
        "candidates": candidates,
        "selected": selected,
        "app_root": app_root,
        "executable": selected["path"],
        "executable_relative": selected["path"].relative_to(app_root),
        "version_dir": version_dir,
        "version": version,
        "product": selected["product"],
        "architecture": selected["architecture"],
    }


def analyze_extracted_app(app_root, desired_arch="x64"):
    app_root = Path(app_root)
    candidates = find_browser_candidates(app_root, desired_arch=desired_arch)
    selected = select_browser_candidate(candidates)
    version = selected["version"] or "0.0.0.0"
    return {
        "candidates": candidates,
        "selected": selected,
        "app_root": app_root,
        "executable": selected["path"],
        "executable_relative": selected["path"].relative_to(app_root),
        "version_dir": _infer_version_dir(app_root, selected["path"], version),
        "version": version,
        "product": selected["product"],
        "architecture": selected["architecture"],
    }


def public_report(result):
    package = result.get("package")
    selected = result["selected"]
    layer_root = selected.get("layer_root")
    selected_path = selected["path"]
    if layer_root:
        selected_display = selected_path.relative_to(layer_root).as_posix()
    else:
        selected_display = selected_path.relative_to(result["app_root"]).as_posix()
    return {
        "package": str(package) if package else None,
        "product": result["product"],
        "version": result["version"],
        "architecture": result["architecture"],
        "executable": selected_display,
        "score": selected["score"],
        "reasons": selected["reasons"],
        "layers": [
            {
                "depth": layer["depth"],
                "kind": layer["kind"],
                "source": Path(layer["source"]).name,
            }
            for layer in result.get("layers", [])
        ],
        "alternatives": [
            {
                "path": str(item["path"]),
                "score": item["score"],
                "product": item["product"],
            }
            for item in result["candidates"][1:6]
        ],
    }


def print_report(result, as_json=False):
    report = public_report(result)
    if as_json:
        print(json.dumps(report, ensure_ascii=False, indent=2))
        return
    print(f"[OK] 浏览器：{report['product']}")
    print(f"[OK] 版本：{report['version']} ({report['architecture']})")
    print(f"[OK] 主程序：{report['executable']}")
    print(f"[OK] 可信分：{report['score']}")
    print("[OK] 判断依据：" + "；".join(report["reasons"]))
