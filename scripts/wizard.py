#!/usr/bin/env python3
"""Interactive local builder — pick options in the terminal, no long flags.

Double-click 开始构建.bat, or run:

    python scripts/wizard.py

UI language follows the system (zh-CN / en).
Override with environment variable WIZARD_LANG=zh-CN|en.

Underlying commands stay available for CI and advanced use.
Never installs or launches browsers; static extract + Chrome++ inject only.
"""

from __future__ import annotations

import json
import locale
import os
import shutil
import subprocess
import sys
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

INSTALLER_SUFFIXES = {".exe", ".msi", ".7z", ".zip", ".rar", ".cab"}
INSTALLER_DIR_NAME = "installers"
LOCALE_PATH = Path(__file__).resolve().parent / "locales" / "wizard.json"
DEFAULT_LANG = "zh-CN"

# LCID / language primary → wizard locale id
LANG_MAP = {
    "zh": "zh-CN",
    "zh-cn": "zh-CN",
    "zh-hans": "zh-CN",
    "zh-sg": "zh-CN",
    "zh-tw": "zh-CN",
    "zh-hk": "zh-CN",
    "zh-hant": "zh-CN",
    "en": "en",
}
LCID_MAP = {
    0x0804: "zh-CN",
    0x0404: "zh-CN",
    0x0C04: "zh-CN",
    0x1004: "zh-CN",
    0x1404: "zh-CN",
    0x0409: "en",
    0x0809: "en",
}

ONLINE_KEYS = [
    ("brave_stable", "online_brave"),
    ("vivaldi_stable", "online_vivaldi"),
    ("opera_stable", "online_opera"),
    ("thorium_stable", "online_thorium"),
    ("cse360_stable", "online_cse360"),
    ("chrome_stable", "online_chrome_stable"),
    ("chrome_beta", "online_chrome_beta"),
    ("edge_stable", "online_edge"),
    ("helium_stable", "online_helium"),
]


def _normalize_lang(raw: str | None) -> str | None:
    if not raw:
        return None
    text = raw.strip().replace("_", "-").lower()
    if text in LANG_MAP:
        return LANG_MAP[text]
    primary = text.split("-", 1)[0]
    return LANG_MAP.get(primary)


def detect_lang() -> str:
    override = os.environ.get("WIZARD_LANG") or os.environ.get("CHROMIUMPORTABLE_LANG")
    matched = _normalize_lang(override)
    if matched:
        return matched

    if sys.platform == "win32":
        try:
            import ctypes

            lcid = ctypes.windll.kernel32.GetUserDefaultUILanguage() & 0xFFFF
            if lcid in LCID_MAP:
                return LCID_MAP[lcid]
            # Follow user locale (date/format) as a fallback.
            lcid = ctypes.windll.kernel32.GetUserDefaultLangID() & 0xFFFF
            if lcid in LCID_MAP:
                return LCID_MAP[lcid]
        except Exception:
            pass

    for source in (
        os.environ.get("LC_ALL"),
        os.environ.get("LC_MESSAGES"),
        os.environ.get("LANG"),
        (locale.getlocale()[0] if hasattr(locale, "getlocale") else None),
    ):
        matched = _normalize_lang(source)
        if matched:
            return matched
    return DEFAULT_LANG


def load_strings(lang: str) -> dict[str, str]:
    try:
        data = json.loads(LOCALE_PATH.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}
    return dict(data.get(lang) or data.get(DEFAULT_LANG) or {})


LANG = detect_lang()
STRINGS = load_strings(LANG)


def t(key: str, **kwargs) -> str:
    template = STRINGS.get(key) or key
    if kwargs:
        try:
            return template.format(**kwargs)
        except (KeyError, ValueError):
            return template
    return template


ONLINE_LABEL_MAP = {
    "brave_stable": "online_brave",
    "vivaldi_stable": "online_vivaldi",
    "opera_stable": "online_opera",
    "thorium_stable": "online_thorium",
    "cse360_stable": "online_cse360",
    "chrome_stable": "online_chrome_stable",
    "chrome_beta": "online_chrome_beta",
    "edge_stable": "online_edge",
    "helium_stable": "online_helium",
}


def term_width() -> int:
    try:
        columns = shutil.get_terminal_size(fallback=(80, 24)).columns
    except Exception:
        columns = 80
    # Keep readable on ultra-wide and very narrow consoles.
    return max(56, min(int(columns), 100))


def cell_width(text: str) -> int:
    width = 0
    for char in text:
        if unicodedata.combining(char):
            continue
        width += 2 if unicodedata.east_asian_width(char) in {"W", "F"} else 1
    return width


def fit(text: str, width: int) -> str:
    """Pad/truncate by display cells so box borders stay aligned with CJK."""
    current = cell_width(text)
    if current <= width:
        return text + " " * (width - current)
    out = []
    used = 0
    for char in text:
        step = 2 if unicodedata.east_asian_width(char) in {"W", "F"} else 1
        if used + step > width - 1:
            out.append("…")
            break
        out.append(char)
        used += step
    body = "".join(out)
    return body + " " * max(0, width - cell_width(body))


def wrap_cells(text: str, width: int) -> list[str]:
    if width <= 4:
        return [text]
    lines: list[str] = []
    current = ""
    used = 0
    for char in text:
        step = 2 if unicodedata.east_asian_width(char) in {"W", "F"} else 1
        if used + step > width and current:
            lines.append(current)
            current = ""
            used = 0
        current += char
        used += step
    if current:
        lines.append(current)
    return lines or [""]


def clear_screen() -> None:
    os.system("cls" if os.name == "nt" else "clear")


def rule(char: str = "─") -> str:
    width = term_width()
    # Box-drawing CJK-safe fill to exact column count.
    if cell_width(char) == 2:
        return char * (width // 2)
    return char * width


def panel(lines: list[str], title: str | None = None) -> None:
    width = term_width()
    inner = max(10, width - 4)
    top = "┌─"
    if title:
        label = f" {title} "
        top += label + "─" * max(0, inner - cell_width(label))
    else:
        top += "─" * inner
    top += "─┐"
    print(top)
    for raw in lines:
        for piece in wrap_cells(raw, inner):
            print(f"│ {fit(piece, inner)} │")
    print("└" + "─" * (width - 2) + "┘")


def menu(options: list[tuple[str, str]]) -> None:
    width = term_width()
    inner = max(10, width - 6)
    for index, (_, label) in enumerate(options, 1):
        tag = f" {index} "
        body = f" {tag}  {label}"
        for offset, piece in enumerate(wrap_cells(body, inner)):
            if offset == 0:
                print(f"  {fit(piece, inner + 2)}")
            else:
                print(f"    {fit(piece, inner)}")
        print()


def step_header(step: int, total: int, title: str) -> None:
    clear_screen()
    print()
    panel(
        [
            t("tagline"),
            f"{t('drop_hint')}   ·   {t('output_hint')}",
            f"UI: {LANG}   ·   WIZARD_LANG=en|zh-CN",
        ],
        title=t("title"),
    )
    print()
    print(f"  [{step}/{total}]  {title}")
    print("  " + rule("─")[: max(10, term_width() - 4)])
    print()


def ask(text: str, default: str | None = None) -> str:
    suffix = f" [{default}]" if default not in (None, "") else ""
    try:
        raw = input(f"{text}{suffix}: ").strip()
    except EOFError:
        print()
        print(t("eof_error"))
        raise KeyboardInterrupt
    if not raw and default is not None:
        return default
    return raw


def ask_yes_no(text: str, default: bool = True) -> bool:
    hint = "Y/n" if default else "y/N"
    try:
        raw = input(f"{text} [{hint}]: ").strip().lower()
    except EOFError:
        print()
        print(t("eof_error"))
        raise KeyboardInterrupt
    if not raw:
        return default
    return raw in {"y", "yes", "是", "1"}


def ask_choice(title: str, options: list[tuple[str, str]], step: int = 1, total: int = 4) -> str:
    keys = {key.casefold(): key for key, _ in options}
    while True:
        step_header(step, total, title)
        menu(options)
        raw = ask(t("ask_number"), "1")
        if raw.isdigit() and 1 <= int(raw) <= len(options):
            return options[int(raw) - 1][0]
        matched = keys.get(raw.casefold())
        if matched:
            return matched
        # redraw with error on next loop; flash message briefly via footer
        print()
        print(f"  ! {t('invalid_number')}")
        input()


def ask_arch() -> str:
    return ask_choice(
        t("choose_arch"),
        [
            ("x64", t("arch_x64")),
            ("x86", t("arch_x86")),
            ("arm64", t("arch_arm64")),
        ],
        step=2,
        total=4,
    )


def installer_dir() -> Path:
    folder = ROOT / INSTALLER_DIR_NAME
    folder.mkdir(parents=True, exist_ok=True)
    readme = folder / "README.txt"
    if not readme.exists():
        readme.write_text(
            "把要构建的浏览器安装包放在这里。\n"
            "Put installer packages for portable builds in this folder.\n"
            "Supported: .exe .msi .7z .zip .rar .cab\n",
            encoding="utf-8",
        )
    return folder


def list_installers(folder: Path) -> list[Path]:
    return sorted(
        (
            path
            for path in folder.iterdir()
            if path.is_file() and path.suffix.casefold() in INSTALLER_SUFFIXES
        ),
        key=lambda path: path.name.casefold(),
    )


def explain_empty_folder(folder: Path) -> None:
    print()
    print(f"  [{folder}]{t('empty_folder_1')}")
    print(f"  {t('empty_folder_2')}")
    print(f"  {t('empty_folder_3')}")
    print(f"  {t('empty_folder_4')}")


def pick_installer() -> Path | None:
    default_dir = installer_dir()
    step_header(3, 4, t("ask_folder"))
    print(f"  {t('installer_dir_label')}{default_dir}")
    print()
    raw = ask(t("ask_folder"), str(default_dir))
    folder = Path(raw).expanduser()
    if not folder.is_absolute():
        folder = (ROOT / folder).resolve()
    if folder.is_file() and folder.suffix.casefold() in INSTALLER_SUFFIXES:
        return folder
    if not folder.is_dir():
        step_header(3, 4, t("ask_folder"))
        print(f"  {t('path_not_found')}{folder}")
        explain_empty_folder(default_dir)
        input()
        return None
    items = list_installers(folder)
    if not items:
        step_header(3, 4, t("ask_folder"))
        explain_empty_folder(folder)
        if folder != default_dir:
            print(f"  {t('alt_dir_note')}{default_dir}）")
        input()
        return None
    while True:
        step_header(3, 4, t("choose_installer"))
        menu([(str(i), p.name) for i, p in enumerate(items, 1)])
        raw = ask(t("ask_number"), "1")
        if raw.isdigit() and 1 <= int(raw) <= len(items):
            return items[int(raw) - 1]
        print(f"  ! {t('invalid_number')}")
        input()


def run(cmd: list[str]) -> int:
    print()
    print("→", " ".join(cmd))
    print()
    result = subprocess.run(cmd, cwd=ROOT)
    return result.returncode


def python_cmd() -> list[str]:
    return [sys.executable, "-m", "portable_builder", "--workdir", str(ROOT), "--builder-dir", str(ROOT)]


def build_local_package(installer: Path, architecture: str, archive: bool) -> int:
    cmd = python_cmd() + ["build-package", str(installer), "--architecture", architecture]
    if archive:
        cmd.append("--archive")
    return run(cmd)


def build_local_directory(folder: Path, architecture: str, archive: bool) -> int:
    cmd = python_cmd() + ["build-packages", str(folder), "--architecture", architecture]
    if archive:
        cmd.append("--archive")
    return run(cmd)


def build_online(target_id: str, architecture: str, archive: bool) -> int:
    prepare = [
        sys.executable,
        str(ROOT / "scripts" / "prepare_build_target.py"),
        "--browser",
        target_id,
        "--architecture",
        architecture,
        "--output",
        str(ROOT / "build" / "selected.browser.json"),
    ]
    code = run(prepare)
    if code != 0:
        return code
    config = str(ROOT / "build" / "selected.browser.json")
    base = python_cmd() + ["--config", config, "--target", target_id]
    code = run(base + ["build"])
    if code != 0:
        return code
    if not archive:
        return 0
    code = run(base + ["archive"])
    if code != 0:
        return code
    return run(base + ["verify", "--no-smoke"])


def main() -> int:
    mode = ask_choice(
        t("choose_action"),
        [
            ("one", t("action_one")),
            ("folder", t("action_folder")),
            ("online", t("action_online")),
        ],
        step=1,
        total=4,
    )

    architecture = ask_arch()
    step_header(3, 4, t("ask_archive"))
    archive = ask_yes_no(t("ask_archive"), default=True)

    if mode == "one":
        installer = pick_installer()
        if installer is None:
            return 1
        step_header(4, 4, t("ask_confirm"))
        print(f"  {t('about_to_one')}{installer.name}")
        print(f"  arch = {architecture} · archive = {archive}")
        print()
        if not ask_yes_no(t("ask_confirm"), default=True):
            print(t("cancelled"))
            return 0
        code = build_local_package(installer, architecture, archive)

    elif mode == "folder":
        default_dir = installer_dir()
        step_header(3, 4, t("ask_folder"))
        print(f"  {t('installer_dir_label')}{default_dir}")
        print()
        raw = ask(t("ask_folder"), str(default_dir))
        folder = Path(raw).expanduser()
        if not folder.is_absolute():
            folder = (ROOT / folder).resolve()
        if not folder.is_dir():
            step_header(3, 4, t("ask_folder"))
            print(f"  {t('path_not_found')}{folder}")
            explain_empty_folder(default_dir)
            return 1
        count = len(list_installers(folder))
        if count == 0:
            explain_empty_folder(folder)
            return 1
        step_header(4, 4, t("ask_confirm"))
        print(f"  {t('about_to_folder', count=count, arch=architecture)}")
        print()
        if not ask_yes_no(t("ask_confirm"), default=True):
            print(t("cancelled"))
            return 0
        code = build_local_directory(folder, architecture, archive)

    else:
        catalog_path = ROOT / "catalog" / "browser_catalog.json"
        catalog = json.loads(catalog_path.read_text(encoding="utf-8"))
        options = [
            (key, t(ONLINE_LABEL_MAP[key]))
            for key, _ in ONLINE_KEYS
            if key in catalog.get("targets", {})
        ]
        target_id = ask_choice(t("choose_online"), options, step=3, total=4)
        label = dict(options)[target_id]
        step_header(4, 4, t("ask_confirm"))
        print(f"  {t('about_to_online', label=label, arch=architecture)}")
        print()
        if not ask_yes_no(t("ask_confirm"), default=True):
            print(t("cancelled"))
            return 0
        code = build_online(target_id, architecture, archive)

    clear_screen()
    print()
    if code == 0:
        print(t("done_release"))
        if archive:
            print(t("done_archive"))
        print(t("done_eula"))
    else:
        print(t("failed", code=code))
        print(t("hint_token"))
        print(t("hint_local"))
    return code


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        print(f"\n{t('cancelled')}")
        raise SystemExit(130)
