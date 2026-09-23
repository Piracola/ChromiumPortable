import tempfile
import unittest
import sys
from pathlib import Path
from unittest.mock import patch

from portable_builder.builder import prepare_package
from portable_builder.cli import main
from portable_builder.discovery import (
    extract_package_layers,
    find_browser_candidates,
    select_browser_candidate,
)
from portable_builder.brave_bundle import decode_bcj2_container


def write_fake_pe(path, size=128):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"MZ" + b"\0" * max(0, size - 2))


class DiscoveryTests(unittest.TestCase):
    def test_360_does_not_choose_larger_compatibility_executable(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "Chrome-bin"
            main = root / "360chromex.exe"
            compatibility = root / "win_clang_x86" / "360chromeie.exe"
            write_fake_pe(main, 1024)
            write_fake_pe(compatibility, 1024 * 1024)
            version_dir = root / "23.1.1253.64"
            version_dir.mkdir(parents=True)
            (version_dir / "chrome.dll").write_bytes(b"")

            def version_info(path):
                if Path(path).name.casefold() == "360chromex.exe":
                    return {
                        "ProductName": "360极速浏览器X",
                        "ProductVersion": "23.1.1253.64",
                        "OriginalFilename": "360chromex.exe",
                    }
                return {"ProductName": "360 Chrome compatibility component"}

            def machine(path):
                return "x86" if Path(path).name.casefold() == "360chromeie.exe" else "x64"

            with patch("portable_builder.discovery.read_version_info", side_effect=version_info), patch(
                "portable_builder.discovery.read_pe_machine", side_effect=machine
            ), patch("portable_builder.discovery.read_pe_import_names", return_value=(["VERSION.dll"], ".rdata")):
                selected = select_browser_candidate(find_browser_candidates(root))

            self.assertEqual(main, selected["path"])

    def test_unknown_chromium_browser_can_be_identified_by_structure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "Acme-bin"
            executable = root / "Acme.exe"
            write_fake_pe(executable)
            (root / "chrome_elf.dll").write_bytes(b"")
            (root / "resources.pak").write_bytes(b"")

            with patch(
                "portable_builder.discovery.read_version_info",
                return_value={
                    "ProductName": "Acme Browser",
                    "ProductVersion": "1.2.3.4",
                    "OriginalFilename": "Acme.exe",
                },
            ), patch("portable_builder.discovery.read_pe_machine", return_value="x64"), patch(
                "portable_builder.discovery.read_pe_import_names",
                return_value=(["chrome_elf.dll", "KERNEL32.dll"], ".rdata"),
            ):
                selected = select_browser_candidate(find_browser_candidates(root))

            self.assertEqual(executable, selected["path"])
            self.assertGreaterEqual(selected["score"], 80)

    def test_ambiguous_high_confidence_candidates_stop_instead_of_guessing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write_fake_pe(root / "chrome.exe")
            write_fake_pe(root / "brave.exe")
            (root / "chrome_elf.dll").write_bytes(b"")

            def version_info(path):
                return {
                    "ProductName": "Chromium Browser",
                    "ProductVersion": "1.2.3.4",
                    "OriginalFilename": Path(path).name,
                }

            with patch("portable_builder.discovery.read_version_info", side_effect=version_info), patch(
                "portable_builder.discovery.read_pe_machine", return_value="x64"
            ), patch(
                "portable_builder.discovery.read_pe_import_names",
                return_value=(["chrome_elf.dll"], ".rdata"),
            ):
                candidates = find_browser_candidates(root)
                with self.assertRaisesRegex(RuntimeError, "多个接近"):
                    select_browser_candidate(candidates)

    def test_browser_like_file_without_chromium_files_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            executable = Path(directory) / "chrome.exe"
            write_fake_pe(executable)

            with patch(
                "portable_builder.discovery.read_version_info",
                return_value={"ProductName": "Chrome Browser", "OriginalFilename": "chrome.exe"},
            ), patch("portable_builder.discovery.read_pe_machine", return_value="x64"), patch(
                "portable_builder.discovery.read_pe_import_names",
                return_value=(["KERNEL32.dll"], ".rdata"),
            ):
                with self.assertRaisesRegex(RuntimeError, "没有找到"):
                    select_browser_candidate(find_browser_candidates(Path(directory)))

    def test_brave_bcj2_container_without_branches(self):
        content = b"plain payload"
        header = b"".join(
            value.to_bytes(4, "little")
            for value in (len(content), len(content), 0, 0, 5)
        )
        container = header + content + b"\0" * 5
        self.assertEqual(content, decode_bcj2_container(container))

    def test_existing_directory_is_inspected_without_touching_the_temp_root(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package_dir = root / "already-extracted"
            package_dir.mkdir()
            extraction_root = root / "temporary-output"
            extraction_root.mkdir()
            marker = extraction_root / "keep.txt"
            marker.write_text("keep", encoding="utf-8")

            layers = extract_package_layers(package_dir, extraction_root, seven_zip=None)

            self.assertEqual(package_dir, layers[0]["root"])
            self.assertTrue(marker.exists())

    def test_preparing_an_existing_directory_does_not_require_an_extractor(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package_dir = root / "already-extracted"
            executable = package_dir / "Browser.exe"
            write_fake_pe(executable)
            discovery = {
                "app_root": package_dir,
                "executable": executable,
                "executable_relative": Path("Browser.exe"),
                "version_dir": package_dir,
                "version": "1.2.3.4",
            }
            target = {"target": "existing_directory", "layout": "auto", "architecture": "x64"}

            with patch("portable_builder.builder.find_7z_tool") as find_7z, patch(
                "portable_builder.builder.analyze_package", return_value=discovery
            ) as analyze:
                prepared = prepare_package(target, root, {"path": str(package_dir), "version": "0.0.0.0"})

            find_7z.assert_not_called()
            analyze.assert_called_once_with(
                package_dir,
                root / "build" / "temp" / "existing_directory" / "auto-extracted",
                None,
                desired_arch="x64",
            )
            self.assertEqual(package_dir, prepared["app_source"])

    def test_inspect_command_skips_extractor_for_an_existing_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package_dir = root / "already-extracted"
            package_dir.mkdir()

            with patch("portable_builder.cli.configure_stdout"), patch(
                "portable_builder.cli.find_7z_tool", side_effect=AssertionError("must not be called")
            ) as find_7z, patch("portable_builder.cli.analyze_package", return_value={}) as analyze, patch(
                "portable_builder.cli.print_report"
            ), patch.object(sys, "argv", [
                "portable_builder",
                "--workdir",
                str(root),
                "inspect-package",
                str(package_dir),
            ]):
                main()

            find_7z.assert_not_called()
            analyze.assert_called_once_with(
                package_dir,
                root / "build" / "inspect-package" / package_dir.stem,
                None,
                desired_arch="x64",
            )


if __name__ == "__main__":
    unittest.main()
