import argparse
import json
import os
from pathlib import Path

from .builder import archive_target, build_package_file, build_target
from .config import get_target, load_config
from .discovery import analyze_package, print_report, public_report
from .multi import build_selected_targets, check_targets, render_multi_release, split_targets, update_multi_release
from .release import check_updates, render_release, update_release
from .tools import configure_stdout, find_7z_tool, remove_path
from .verify import verify_target, verify_targets


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

    package_root = Path(__file__).resolve().parent.parent
    if (package_root / "setdll").exists():
        print(f"[INFO] Using builder files beside the Python package: {package_root}")
        return str(package_root)

    return None


def load_target(args):
    config = load_config(args.config)
    return get_target(config, args.target)


def main():
    configure_stdout()

    parser = argparse.ArgumentParser(description="Reusable portable Chromium browser builder")
    parser.add_argument("--config", default="browser.json", help="Path to browser config JSON/TOML/YAML")
    parser.add_argument("--target", default=None, help="Target name from config")
    parser.add_argument("--workdir", default=".", help="Caller repository working directory")
    parser.add_argument("--builder-dir", default=None, help="Path to builder repository (ChromiumPortable). Auto-detected from PYTHONPATH or _portable_builder if not set.")

    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("check", help="Check upstream and release versions")
    subparsers.add_parser("build", help="Build portable browser")
    subparsers.add_parser("archive", help="Archive build/release into build/assets")
    inspect_parser = subparsers.add_parser("inspect-package", help="Statically inspect an installer and locate its browser executable")
    inspect_parser.add_argument("package", help="Installer file or an already extracted directory")
    inspect_parser.add_argument("--architecture", default="x64", choices=("x86", "x64", "arm64"), help="Expected browser architecture")
    inspect_parser.add_argument("--json", action="store_true", help="Print the result as JSON")
    inspect_parser.add_argument("--keep-extracted", action="store_true", help="Keep the temporary extracted files for debugging")
    build_package_parser = subparsers.add_parser("build-package", help="Automatically locate, patch, and build a portable browser from an installer or extracted package")
    build_package_parser.add_argument("package", help="Installer file or extracted browser package directory")
    build_package_parser.add_argument("--architecture", default="x64", choices=("x86", "x64", "arm64"), help="Expected browser architecture")
    build_package_parser.add_argument("--output-dir", default=None, help="Portable browser directory name")
    build_package_parser.add_argument("--archive", action="store_true", help="Also create a final 7z archive")
    build_packages_parser = subparsers.add_parser(
        "build-packages",
        help="Extract, locate, and statically inject Chrome++ into every installer in a directory (one auto-layout path for all Chromium structures)",
    )
    build_packages_parser.add_argument("directory", nargs="?", default="bin", help="Directory containing installer packages")
    build_packages_parser.add_argument("--architecture", default="x64", choices=("x86", "x64", "arm64"), help="Expected browser architecture")
    build_packages_parser.add_argument("--archive", action="store_true", help="Also create final 7z archives and run static verify")
    research_parser = subparsers.add_parser("research-packages", help="Statically inspect every installer in a directory")
    research_parser.add_argument("directory", nargs="?", default="bin", help="Directory containing installer samples")
    research_parser.add_argument("--architecture", default="x64", choices=("x86", "x64", "arm64"), help="Expected browser architecture")
    research_parser.add_argument("--json", action="store_true", help="Print the combined result as JSON")
    research_parser.add_argument("--keep-extracted", action="store_true", help="Keep temporary extracted files for debugging")
    verify_parser = subparsers.add_parser("verify", help="Extract the built archive and verify injection plus portability")
    verify_parser.add_argument("--archive", default=None, help="Archive path (default: newest match in build/assets)")
    verify_parser.add_argument("--no-smoke", action="store_true", help="Skip launching the browser; check the import table only")

    verify_targets_parser = subparsers.add_parser("verify-targets", help="Verify archives for multiple comma-separated targets")
    verify_targets_parser.add_argument("--no-smoke", action="store_true", help="Skip launching the browser; check the import table only")
    subparsers.add_parser("render-release", help="Render release title/tag/body")
    subparsers.add_parser("update-release", help="Update existing GitHub release metadata and remove old assets")
    subparsers.add_parser("check-targets", help="Check multiple comma-separated targets")
    subparsers.add_parser("build-targets", help="Build/archive updated comma-separated targets")
    subparsers.add_parser("render-release-targets", help="Render release metadata for multiple targets")
    subparsers.add_parser("update-release-targets", help="Update release for multiple targets")

    args = parser.parse_args()
    workdir = Path(args.workdir).resolve()
    if args.command == "inspect-package":
        package = Path(args.package)
        if not package.is_absolute():
            package = workdir / package
        if not package.exists():
            raise FileNotFoundError(f"Package not found: {package}")
        inspect_root = workdir / "build" / "inspect-package" / package.stem
        try:
            seven_zip = None
            if not package.is_dir():
                seven_zip = find_7z_tool(workdir, allow_download=False, allow_system_install=False)
            result = analyze_package(
                package,
                inspect_root,
                seven_zip,
                desired_arch=args.architecture,
            )
            print_report(result, as_json=args.json)
        finally:
            if not args.keep_extracted:
                remove_path(inspect_root)
        return

    if args.command == "build-package":
        package = Path(args.package)
        if not package.is_absolute():
            package = workdir / package
        builder_dir = resolve_builder_dir(args.builder_dir)
        result = build_package_file(
            package,
            workdir,
            builder_dir=builder_dir,
            architecture=args.architecture,
            output_dir=args.output_dir,
        )
        if args.archive:
            archive = archive_target(
                result["target_config"],
                workdir,
                version=result["version"],
                package_version=result["package_version"],
                source_dir=result["output_dir"],
            )
            # Static archive verification deliberately avoids launching a
            # browser or installer on the local machine.
            verify_target(result["target_config"], workdir, archive=archive["path"], smoke=False)
        return

    if args.command == "build-packages":
        sample_dir = Path(args.directory)
        if not sample_dir.is_absolute():
            sample_dir = workdir / sample_dir
        if not sample_dir.is_dir():
            raise FileNotFoundError(f"Package directory not found: {sample_dir}")
        suffixes = {".exe", ".msi", ".7z", ".zip", ".rar", ".cab"}
        packages = sorted(
            (path for path in sample_dir.iterdir() if path.is_file() and path.suffix.casefold() in suffixes),
            key=lambda path: path.name.casefold(),
        )
        if not packages:
            raise FileNotFoundError(f"No installer packages found in: {sample_dir}")
        builder_dir = resolve_builder_dir(args.builder_dir)
        built = []
        failures = []
        for index, package in enumerate(packages, 1):
            print(f"\n[{index}/{len(packages)}] {package.name}")
            try:
                result = build_package_file(
                    package,
                    workdir,
                    builder_dir=builder_dir,
                    architecture=args.architecture,
                )
                entry = {
                    "package": package.name,
                    "product": result["target_config"].get("display_name") or result["target_config"].get("name"),
                    "version": result["version"],
                    "output_dir": result["output_dir"],
                }
                if args.archive:
                    archive = archive_target(
                        result["target_config"],
                        workdir,
                        version=result["version"],
                        package_version=result["package_version"],
                        source_dir=result["output_dir"],
                    )
                    verify_target(result["target_config"], workdir, archive=archive["path"], smoke=False)
                    entry["archive"] = archive["name"]
                    entry["sha256"] = archive["sha256"]
                built.append(entry)
                print(f"[OK] {package.name} -> {entry['product']} {entry['version']}")
            except Exception as exc:
                failures.append({"package": package.name, "error": str(exc)})
                print(f"[FAIL] {package.name}: {exc}")
        print(f"\n[DONE] built={len(built)} failed={len(failures)}")
        for item in built:
            print(f"  OK  {item['package']}: {item['product']} {item['version']}")
        for item in failures:
            print(f"  ERR {item['package']}: {item['error']}")
        if failures:
            raise RuntimeError(f"{len(failures)} package(s) failed to build")
        return

    if args.command == "research-packages":
        sample_dir = Path(args.directory)
        if not sample_dir.is_absolute():
            sample_dir = workdir / sample_dir
        if not sample_dir.is_dir():
            raise FileNotFoundError(f"Sample directory not found: {sample_dir}")
        suffixes = {".exe", ".msi", ".7z", ".zip", ".rar", ".cab"}
        packages = sorted(
            (path for path in sample_dir.iterdir() if path.is_file() and path.suffix.casefold() in suffixes),
            key=lambda path: path.name.casefold(),
        )
        if not packages:
            raise FileNotFoundError(f"No installer samples found in: {sample_dir}")
        reports = []
        failures = []
        seven_zip = find_7z_tool(workdir, allow_download=False, allow_system_install=False)
        for index, package in enumerate(packages, 1):
            inspect_root = workdir / "build" / "research-packages" / f"{index:02d}-{package.stem}"
            try:
                result = analyze_package(
                    package,
                    inspect_root,
                    seven_zip,
                    desired_arch=args.architecture,
                )
                report = public_report(result)
                reports.append(report)
                if not args.json:
                    print(f"\n[{index}/{len(packages)}] {package.name}")
                    print_report(result)
            except Exception as exc:
                failure = {"package": str(package), "error": str(exc)}
                failures.append(failure)
                if not args.json:
                    print(f"\n[FAIL] {package.name}: {exc}")
            finally:
                if not args.keep_extracted:
                    remove_path(inspect_root)
        combined = {"recognized": reports, "failed": failures}
        if args.json:
            print(json.dumps(combined, ensure_ascii=False, indent=2))
        if failures:
            raise RuntimeError(f"{len(failures)} package(s) could not be identified")
        return

    if not args.target:
        parser.error("--target is required for this command")
    config = load_config(args.config)
    target = None if args.command.endswith("targets") else get_target(config, args.target)
    builder_dir = resolve_builder_dir(args.builder_dir)

    if args.command == "check":
        check_updates(target, workdir)
    elif args.command == "build":
        build_target(target, workdir, builder_dir=builder_dir)
    elif args.command == "archive":
        archive_target(target, workdir)
    elif args.command == "verify":
        verify_target(target, workdir, archive=args.archive, smoke=not args.no_smoke)
    elif args.command == "verify-targets":
        verify_targets(config, split_targets(args.target), workdir, smoke=not args.no_smoke)
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
