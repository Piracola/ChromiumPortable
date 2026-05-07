import argparse
import os
from pathlib import Path

from .builder import archive_target, build_target
from .config import get_target, load_config
from .multi import build_selected_targets, check_targets, render_multi_release, split_targets, update_multi_release
from .release import check_updates, render_release, update_release
from .tools import configure_stdout


def resolve_builder_dir(args_builder_dir):
    if args_builder_dir:
        path = Path(args_builder_dir)
        if not path.exists():
            raise FileNotFoundError(f"Builder directory not found: {path}")
        return str(path.resolve())

    for env_entry in os.getenv("PYTHONPATH", "").split(os.pathsep):
        entry = env_entry.strip()
        if entry:
            path = Path(entry)
            if path.exists() and (path / "setdll").exists():
                print(f"[INFO] Auto-detected builder directory from PYTHONPATH: {path}")
                return str(path.resolve())

    default_path = Path("_portable_builder")
    if default_path.exists() and (default_path / "setdll").exists():
        print(f"[INFO] Using default builder directory: {default_path}")
        return str(default_path.resolve())

    return None


def load_target(args):
    config = load_config(args.config)
    return get_target(config, args.target)


def main():
    configure_stdout()

    parser = argparse.ArgumentParser(description="Reusable portable Chromium browser builder")
    parser.add_argument("--config", default="browser.json", help="Path to browser config JSON/TOML/YAML")
    parser.add_argument("--target", required=True, help="Target name from config")
    parser.add_argument("--workdir", default=".", help="Caller repository working directory")
    parser.add_argument("--builder-dir", default=None, help="Path to builder repository (ChromiumPortable). Auto-detected from PYTHONPATH or _portable_builder if not set.")

    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("check", help="Check upstream and release versions")
    subparsers.add_parser("build", help="Build portable browser")
    subparsers.add_parser("archive", help="Archive build/release into build/assets")
    subparsers.add_parser("render-release", help="Render release title/tag/body")
    subparsers.add_parser("update-release", help="Update existing GitHub release metadata and remove old assets")
    subparsers.add_parser("check-targets", help="Check multiple comma-separated targets")
    subparsers.add_parser("build-targets", help="Build/archive updated comma-separated targets")
    subparsers.add_parser("render-release-targets", help="Render release metadata for multiple targets")
    subparsers.add_parser("update-release-targets", help="Update release for multiple targets")

    args = parser.parse_args()
    config = load_config(args.config)
    target = None if args.command.endswith("targets") else get_target(config, args.target)
    workdir = Path(args.workdir).resolve()
    builder_dir = resolve_builder_dir(args.builder_dir)

    if args.command == "check":
        check_updates(target, workdir)
    elif args.command == "build":
        build_target(target, workdir, builder_dir=builder_dir)
    elif args.command == "archive":
        archive_target(target, workdir)
    elif args.command == "render-release":
        render_release(target, workdir)
    elif args.command == "update-release":
        update_release(target, workdir)
    elif args.command == "check-targets":
        check_targets(config, split_targets(args.target), workdir)
    elif args.command == "build-targets":
        build_selected_targets(config, split_targets(args.target), workdir, builder_dir=builder_dir)
    elif args.command == "render-release-targets":
        render_multi_release(config, split_targets(args.target), workdir)
    elif args.command == "update-release-targets":
        update_multi_release(config, split_targets(args.target), workdir)
