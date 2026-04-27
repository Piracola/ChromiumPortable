import argparse
from pathlib import Path

from .builder import archive_target, build_target
from .config import get_target, load_config
from .release import check_updates, render_release, update_release
from .tools import configure_stdout


def load_target(args):
    config = load_config(args.config)
    return get_target(config, args.target)


def main():
    configure_stdout()

    parser = argparse.ArgumentParser(description="Reusable portable Chromium browser builder")
    parser.add_argument("--config", default="browser.json", help="Path to browser config JSON/TOML/YAML")
    parser.add_argument("--target", required=True, help="Target name from config")
    parser.add_argument("--workdir", default=".", help="Caller repository working directory")

    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("check", help="Check upstream and release versions")
    subparsers.add_parser("build", help="Build portable browser")
    subparsers.add_parser("archive", help="Archive build/release into build/assets")
    subparsers.add_parser("render-release", help="Render release title/tag/body")
    subparsers.add_parser("update-release", help="Update existing GitHub release metadata and remove old assets")

    args = parser.parse_args()
    target = load_target(args)
    workdir = Path(args.workdir).resolve()

    if args.command == "check":
        check_updates(target, workdir)
    elif args.command == "build":
        build_target(target, workdir)
    elif args.command == "archive":
        archive_target(target, workdir)
    elif args.command == "render-release":
        render_release(target, workdir)
    elif args.command == "update-release":
        update_release(target, workdir)
