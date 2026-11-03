"""AppImage payload regressions, independent of Rust and Linux build tools."""

import base64
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from check_appimage import (
    LOADER_CHECK_MARKER, check_appdir, check_appimage, check_filename, check_runtime_loader,
)

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "fix_appimage", ROOT / "packaging/linux/fix-appimage.py")
fixer = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = fixer
SPEC.loader.exec_module(fixer)


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
        (self.root / "AppRun").write_bytes(fixer.LAUNCHER.read_bytes())
        (self.root / "usr/lib").mkdir()
        (self.root / "usr/lib/libmiaominal_appimage_probe.so").write_bytes(b"fixture")
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

    def loader_output(self, root=None):
        root = root or self.root.resolve()
        return (f"{LOADER_CHECK_MARKER}{root / 'usr/bin/miaominal'}\n"
                f" libmiaominal_appimage_probe.so => "
                f"{root / 'usr/lib/libmiaominal_appimage_probe.so'} (0x00001234)\n")

    def extract(self, args, **kwargs):
        if len(args) > 1 and args[1] == "--appimage-extract":
            shutil.copytree(self.root, Path(kwargs["cwd"]) / "squashfs-root")
        output = ""
        if Path(args[0]).name == "AppRun":
            output = self.loader_output(Path(args[0]).parent)
        return subprocess.CompletedProcess(args, 0, stdout=output, stderr="")

    def test_final_image_is_extracted_before_desktop_validation(self):
        image = self.make_image()
        with patch("check_appimage.subprocess.run", side_effect=self.extract) as run:
            check_appimage(image)
        self.assertEqual(run.call_count, 3)
        self.assertEqual(run.call_args_list[0].args[0], [str(image), "--appimage-extract"])
        self.assertEqual(run.call_args_list[1].args[0][0], "desktop-file-validate")
        self.assertEqual(Path(run.call_args_list[2].args[0][0]).name, "AppRun")

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

    def test_runtime_loader_uses_only_bundled_library_search_path(self):
        completed = subprocess.CompletedProcess([], 0, stdout=self.loader_output(), stderr="")
        with patch.dict(os.environ, {"LD_LIBRARY_PATH": "/host/override",
                                     "LD_PRELOAD": "/host/preload.so",
                                     "LD_TRACE_LOADED_OBJECTS": "1"}), \
                patch("check_appimage.subprocess.run", return_value=completed) as run:
            check_runtime_loader(self.root)
        environment = run.call_args.kwargs["env"]
        self.assertNotIn("LD_LIBRARY_PATH", environment)
        self.assertNotIn("LD_PRELOAD", environment)
        self.assertNotIn("LD_TRACE_LOADED_OBJECTS", environment)
        self.assertEqual(environment["MIAOMINAL_APPIMAGE_CHECK_RUNTIME"], "1")
        self.assertEqual(environment["APPDIR"], str(self.root.resolve()))
        self.assertEqual(run.call_args.kwargs["timeout"], 30)

    def test_runtime_loader_rejects_missing_dependencies(self):
        output = self.loader_output() + " libgtk-3.so.0 => not found\n"
        completed = subprocess.CompletedProcess([], 0, stdout=output, stderr="")
        with patch("check_appimage.subprocess.run", return_value=completed):
            with self.assertRaisesRegex(ValueError, "unresolved shared libraries"):
                check_runtime_loader(self.root)

    def test_runtime_loader_rejects_host_libraries_masking_bundled_ones(self):
        output = self.loader_output().replace(
            str(self.root.resolve() / "usr/lib/libmiaominal_appimage_probe.so"),
            "/usr/lib/libmiaominal_appimage_probe.so")
        completed = subprocess.CompletedProcess([], 0, stdout=output, stderr="")
        with patch("check_appimage.subprocess.run", return_value=completed):
            with self.assertRaisesRegex(ValueError, "loaded outside the AppDir"):
                check_runtime_loader(self.root)

    def test_runtime_loader_rejects_tracing_the_launcher_shell(self):
        completed = subprocess.CompletedProcess(
            [], 0, stdout="libc.so.6 => /lib/libc.so.6 (0x1234)\n", stderr="")
        with patch("check_appimage.subprocess.run", return_value=completed):
            with self.assertRaisesRegex(ValueError, "did not trace the Miaominal"):
                check_runtime_loader(self.root)

    def test_runtime_loader_requires_at_least_one_bundled_resolution(self):
        completed = subprocess.CompletedProcess(
            [], 0, stdout=f"{LOADER_CHECK_MARKER}{self.root.resolve() / 'usr/bin/miaominal'}\n",
            stderr="")
        with patch("check_appimage.subprocess.run", return_value=completed):
            with self.assertRaisesRegex(ValueError, "any bundled shared library"):
                check_runtime_loader(self.root)

    def test_runtime_loader_rejects_old_launcher_before_execution(self):
        (self.root / "AppRun").write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        with patch("check_appimage.subprocess.run") as run:
            with self.assertRaisesRegex(ValueError, "library-loading launcher"):
                check_runtime_loader(self.root)
        run.assert_not_called()

    def test_install_launcher_unlinks_entry_without_overwriting_binary(self):
        binary = self.root / "usr/bin/miaominal"
        before = binary.read_bytes()
        (self.root / "AppRun").unlink()
        self.symlink("AppRun", "usr/bin/miaominal")
        fixer.install_launcher(self.root)
        self.assertFalse((self.root / "AppRun").is_symlink())
        self.assertEqual(binary.read_bytes(), before)
        self.assertEqual((self.root / "AppRun").read_bytes(), fixer.LAUNCHER.read_bytes())

    def make_repack_image(self):
        image = self.make_image()
        prefix = image.read_bytes() + bytes(128 - image.stat().st_size)
        image.write_bytes(prefix + b"hsqsold payload")
        return image, prefix

    def repack_operation(self, args, **kwargs):
        if len(args) > 1 and args[1] == "--appimage-offset":
            return subprocess.CompletedProcess(args, 0, stdout="128\n", stderr="")
        if len(args) > 1 and args[1] == "--appimage-extract":
            return self.extract(args, **kwargs)
        if args[0] == "mksquashfs":
            root = Path(args[1])
            self.assertEqual((root / "AppRun").read_bytes(), fixer.LAUNCHER.read_bytes())
            self.assertEqual((root / "usr/bin/miaominal").read_bytes(),
                             (self.root / "usr/bin/miaominal").read_bytes())
            Path(args[2]).write_bytes(b"hsqsfixed payload")
        return subprocess.CompletedProcess(args, 0, stdout="", stderr="")

    def test_repack_preserves_runtime_and_replaces_only_payload(self):
        image, prefix = self.make_repack_image()
        with patch("fix_appimage.subprocess.run", side_effect=self.repack_operation) as run:
            fixer.fix_appimage(image)
        self.assertEqual(image.read_bytes(), prefix + b"hsqsfixed payload")
        self.assertEqual(run.call_count, 3)
        self.assertEqual(run.call_args_list[2].args[0][0], "mksquashfs")
        self.assertIn("-noappend", run.call_args_list[2].args[0])

    def test_repack_rejects_invalid_runtime_offset_before_extracting(self):
        image, _ = self.make_repack_image()
        before = image.read_bytes()
        for offset in ("not a number\n", "0\n", "999999\n", "127\n"):
            completed = subprocess.CompletedProcess([], 0, stdout=offset, stderr="")
            with self.subTest(offset=offset), \
                    patch("fix_appimage.subprocess.run", return_value=completed) as run:
                with self.assertRaises(ValueError):
                    fixer.fix_appimage(image)
                self.assertEqual(run.call_count, 1)
                self.assertEqual(image.read_bytes(), before)

    def test_repack_failure_leaves_original_image_untouched(self):
        image, _ = self.make_repack_image()
        before = image.read_bytes()

        def fail_pack(args, **kwargs):
            if args[0] == "mksquashfs":
                raise subprocess.CalledProcessError(1, args)
            return self.repack_operation(args, **kwargs)

        with patch("fix_appimage.subprocess.run", side_effect=fail_pack):
            with self.assertRaises(subprocess.CalledProcessError):
                fixer.fix_appimage(image)
        self.assertEqual(image.read_bytes(), before)

    @unittest.skipUnless(sys.platform.startswith("linux") and shutil.which("cc"),
                         "native ELF loader/argument smoke test runs on Linux CI")
    def test_native_launcher_loads_bundled_library_and_preserves_arguments_and_environment(self):
        # No RPATH and a private SONAME: the bare executable cannot start using
        # host libraries. This catches launchers that merely look correct.
        root = self.root.with_name("AppDir with spaces")
        shutil.copytree(self.root, root)
        library = root / "usr/lib/libmiaominal_appimage_probe.so"
        library_source = Path(self.directory.name) / "library.c"
        library_source.write_text("int appimage_probe(void) { return 42; }\n", encoding="utf-8")
        main_source = Path(self.directory.name) / "main.c"
        main_source.write_text(
            '#include <stdio.h>\n#include <stdlib.h>\n'
            'int appimage_probe(void);\n'
            'int main(int argc, char **argv) {\n'
            ' if (appimage_probe() != 42) return 99;\n'
            ' for (int i = 1; i < argc; ++i) puts(argv[i]);\n'
            ' puts(getenv("APPIMAGE")); puts(getenv("APPDIR")); return 7;\n}\n',
            encoding="utf-8")
        subprocess.run(["cc", "-shared", "-fPIC", str(library_source),
                        "-Wl,-soname,libmiaominal_appimage_probe.so", "-o", str(library)],
                       check=True, capture_output=True)
        binary = root / "usr/bin/miaominal"
        subprocess.run(["cc", str(main_source), "-L", str(root / "usr/lib"),
                        "-lmiaominal_appimage_probe", "-o", str(binary)],
                       check=True, capture_output=True)
        environment = os.environ.copy()
        for name in ("LD_LIBRARY_PATH", "LD_PRELOAD", "LD_AUDIT", "LD_TRACE_LOADED_OBJECTS"):
            environment.pop(name, None)
        bare = subprocess.run([str(binary)], env=environment, capture_output=True, text=True)
        self.assertNotEqual(bare.returncode, 0)
        self.assertIn("libmiaominal_appimage_probe.so", bare.stderr)
        check_runtime_loader(root)
        environment["APPIMAGE"] = "/original/image with spaces.AppImage"
        environment["APPDIR"] = "/runtime/supplied-appdir"
        arguments = ["one argument", "", "$HOME;*"]
        launched = subprocess.run([str(root / "AppRun"), *arguments], env=environment,
                                  capture_output=True, text=True)
        self.assertEqual(launched.returncode, 7, launched.stderr)
        self.assertEqual(launched.stdout.splitlines(),
                         [*arguments, environment["APPIMAGE"], environment["APPDIR"]])


if __name__ == "__main__":
    unittest.main()
