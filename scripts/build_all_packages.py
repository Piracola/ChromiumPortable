#!/usr/bin/env python3
"""One entrypoint that adapts every Chromium-family installer structure.

Usage (from this repository root):

    python scripts/build_all_packages.py
        Interactive wizard (same as 开始构建.bat / scripts/wizard.py).

    python scripts/build_all_packages.py [directory] [--archive]
        Non-interactive batch build for every installer in the directory.

For each installer this runs the auto-layout pipeline:
static extract -> structure-aware main-exe discovery -> Chrome++ injection.
It never runs installers or browsers.
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))


if __name__ == "__main__":
    args = sys.argv[1:]
    if not args or args[0] in {"--wizard", "-i", "--interactive"}:
        scripts_dir = Path(__file__).resolve().parent
        if str(scripts_dir) not in sys.path:
            sys.path.insert(0, str(scripts_dir))
        from wizard import main as wizard_main

        raise SystemExit(wizard_main())

    from portable_builder.cli import main

    directory = "bin"
    if not args[0].startswith("-"):
        directory = args[0]
        args = args[1:]
    sys.argv = [
        "portable_builder",
        "--workdir",
        str(ROOT),
        "--builder-dir",
        str(ROOT),
        "build-packages",
        directory,
        *args,
    ]
    main()
