"""AppImage payload regressions, independent of Rust and Linux build tools."""

import base64
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_appimage import check_appdir, check_appimage, check_filename


PNG = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a9V8AAAAASUVORK5CYII="
)
DESKTOP = """[Desktop Entry]
Type=Application
Name=Miaominal
Exec=miaominal
Icon=miaominal
Terminal=false
Categories=Development;
"""


class AppImageFilenameTests(unittest.TestCase):
    def test_release_architectures(self):
        for arch in ("x86_64", "aarch64"):
            check_filename(f"Miaominal-0.2.4-{arch}.AppImage")

    def test_prerelease_version(self):
        check_filename("Miaominal-0.3.0-rc.1-x86_64.AppImage")

    def test_catalog_name_warning_is_rejected(self):
        for name in ("Miaominal-linux-x86_64-0.2.4.AppImage",
                     "Miaominal-0.2.4-Linux-x86_64.AppImage"):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "linux"):
                check_filename(name)

    def test_wrong_architecture_or_extension(self):
        for name in ("Miaominal-0.2.4-arm64.AppImage", "Miaominal-0.2.4-x86_64.zip"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                check_filename(name)


class AppDirTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name) / "AppDir"
        (self.root / "usr/bin").mkdir(parents=True)
        for name in ("AppRun", "usr/bin/miaominal"):
            path = self.root / name
            path.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            path.chmod(0o755)
        (self.root / ".DirIcon").write_bytes(PNG)
        (self.root / "miaominal.png").write_bytes(PNG)
        self.desktop = self.root / "miaominal.desktop"
        self.desktop.write_text(DESKTOP, encoding="utf-8")

    def symlink(self, link, target):
        try:
            (self.root / link).symlink_to(target)
        except OSError as error:
            if os.name == "nt" and getattr(error, "winerror", None) == 1314:
                self.skipTest("Windows symlink privilege unavailable; covered on Linux CI")
            raise

    def test_complete_appdir(self):
        self.assertEqual(check_appdir(self.root), self.desktop.resolve())

    def test_png_only_bundle_must_still_include_diricon(self):
        # cargo-bundle 0.11 generated this incomplete layout for PNG-only icons.
        (self.root / ".DirIcon").unlink()
        with self.assertRaisesRegex(ValueError, r"\.DirIcon"):
            check_appdir(self.root)

    def test_diricon_must_not_be_svg_or_empty(self):
        for content in (b"", b"<svg></svg>", PNG[:20]):
            (self.root / ".DirIcon").write_bytes(content)
            with self.subTest(content=content), self.assertRaisesRegex(ValueError, "PNG"):
                check_appdir(self.root)

    def test_zero_size_diricon_is_rejected(self):
        (self.root / ".DirIcon").write_bytes(PNG[:16] + bytes(8) + PNG[24:])
        with self.assertRaisesRegex(ValueError, "nonzero"):
            check_appdir(self.root)

    def test_missing_apprun(self):
        (self.root / "AppRun").unlink()
        with self.assertRaisesRegex(ValueError, "AppRun"):
            check_appdir(self.root)

    @unittest.skipIf(os.name == "nt", "Windows does not expose POSIX executable bits")
    def test_nonexecutable_apprun(self):
        (self.root / "AppRun").chmod(0o644)
        with self.assertRaisesRegex(ValueError, "not executable"):
            check_appdir(self.root)

    def test_missing_application_binary(self):
        (self.root / "usr/bin/miaominal").unlink()
        with self.assertRaisesRegex(ValueError, "usr/bin/miaominal"):
            check_appdir(self.root)

    def test_exactly_one_root_desktop_file(self):
        self.desktop.unlink()
        with self.assertRaisesRegex(ValueError, "exactly one"):
            check_appdir(self.root)
        self.desktop.write_text(DESKTOP, encoding="utf-8")
        (self.root / "extra.desktop").write_text(DESKTOP, encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "exactly one"):
            check_appdir(self.root)

    def test_required_desktop_keys(self):
        for line in ("Name=Miaominal", "Exec=miaominal", "Icon=miaominal"):
            self.desktop.write_text(DESKTOP.replace(line, ""), encoding="utf-8")
            with self.subTest(line=line), self.assertRaisesRegex(ValueError, "missing"):
                check_appdir(self.root)

    def test_desktop_entry_group_and_type(self):
        for content in (DESKTOP.replace("Desktop Entry", "Other"),
                        DESKTOP.replace("Type=Application", "Type=Link")):
            self.desktop.write_text(content, encoding="utf-8")
            with self.subTest(content=content), self.assertRaises(ValueError):
                check_appdir(self.root)

    def test_exec_field_codes_are_not_config_interpolation(self):
        self.desktop.write_text(DESKTOP.replace("Exec=miaominal", "Exec=miaominal %U"),
                                encoding="utf-8")
        check_appdir(self.root)

    def test_icon_must_match_root_file(self):
        (self.root / "miaominal.png").unlink()
        with self.assertRaisesRegex(ValueError, "missing root icon"):
            check_appdir(self.root)
        (self.root / "miaominal.svg").write_text("<svg/>", encoding="utf-8")
        check_appdir(self.root)

    def test_icon_must_be_a_basename_without_extension(self):
        for icon in ("miaominal.png", "/usr/share/icons/miaominal", "../miaominal"):
            self.desktop.write_text(DESKTOP.replace("Icon=miaominal", f"Icon={icon}"),
                                    encoding="utf-8")
            with self.subTest(icon=icon), self.assertRaisesRegex(ValueError, "basename"):
                check_appdir(self.root)

    def test_valid_relative_symlinks(self):
        (self.root / "AppRun").unlink()
        self.symlink("AppRun", "usr/bin/miaominal")
        (self.root / ".DirIcon").unlink()
        self.symlink(".DirIcon", "miaominal.png")
        check_appdir(self.root)

    def test_dangling_diricon_symlink(self):
        (self.root / ".DirIcon").unlink()
        self.symlink(".DirIcon", "missing.png")
        with self.assertRaisesRegex(ValueError, "dangling"):
            check_appdir(self.root)

    def test_absolute_host_icon_symlink_is_rejected(self):
        outside = Path(self.directory.name) / "outside.png"
        outside.write_bytes(PNG)
        (self.root / ".DirIcon").unlink()
        self.symlink(".DirIcon", outside)
        with self.assertRaisesRegex(ValueError, "outside"):
            check_appdir(self.root)

    def make_image(self):
        image = Path(self.directory.name) / "Miaominal-0.2.4-x86_64.AppImage"
        image.write_bytes(b"\x7fELF" + bytes(4) + b"AI\x02")
        image.chmod(0o755)
        return image

    def extract(self, args, **kwargs):
        if args[1] == "--appimage-extract":
            shutil.copytree(self.root, Path(kwargs["cwd"]) / "squashfs-root")

    def test_final_image_is_extracted_before_desktop_validation(self):
        image = self.make_image()
        with patch("check_appimage.subprocess.run", side_effect=self.extract) as run:
            check_appimage(image)
        self.assertEqual(run.call_count, 2)
        self.assertEqual(run.call_args_list[0].args[0], [str(image), "--appimage-extract"])
        self.assertEqual(run.call_args_list[1].args[0][0], "desktop-file-validate")

    def test_final_payload_missing_diricon_fails_before_desktop_validation(self):
        image = self.make_image()
        (self.root / ".DirIcon").unlink()
        with patch("check_appimage.subprocess.run", side_effect=self.extract) as run:
            with self.assertRaisesRegex(ValueError, r"\.DirIcon"):
                check_appimage(image)
        self.assertEqual(run.call_count, 1)

    def test_not_an_appimage_is_rejected_before_execution(self):
        image = self.make_image()
        image.write_bytes(b"not an AppImage")
        with patch("check_appimage.subprocess.run") as run:
            with self.assertRaisesRegex(ValueError, "type-2"):
                check_appimage(image)
        run.assert_not_called()

    def test_extraction_failure_is_not_ignored(self):
        image = self.make_image()
        with patch("check_appimage.subprocess.run",
                   side_effect=subprocess.CalledProcessError(1, "extract")):
            with self.assertRaises(subprocess.CalledProcessError):
                check_appimage(image)


if __name__ == "__main__":
    unittest.main()
