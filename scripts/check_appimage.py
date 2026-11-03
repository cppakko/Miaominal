#!/usr/bin/env python3
"""Check the final Miaominal AppImage without FUSE or a graphical session.

Required AppDir entries: https://docs.appimage.org/reference/appdir.html
Use --appdir to check an already extracted payload instead.
"""

import argparse
import configparser
import os
from pathlib import Path
import re
import struct
import subprocess
import sys
import tempfile


def check_filename(name: str) -> None:
    if "linux" in name.lower():
        raise ValueError("AppImage filenames must not contain 'linux'")
    if not re.fullmatch(r"Miaominal-.+-(x86_64|aarch64)\.AppImage", name):
        raise ValueError("expected Miaominal-<version>-<x86_64|aarch64>.AppImage")


def require_file(root: Path, relative: str, *, executable: bool = False) -> Path:
    path = root / relative
    if not path.is_file():
        raise ValueError(f"missing file or dangling symlink: {relative}")
    if not path.resolve().is_relative_to(root):
        raise ValueError(f"file resolves outside the AppDir: {relative}")
    if executable and not os.access(path, os.X_OK):
        raise ValueError(f"file is not executable: {relative}")
    return path


def check_appdir(root: Path) -> Path:
    """Validate the required entries, returning the single root desktop file."""
    root = root.resolve(strict=True)
    require_file(root, "AppRun", executable=True)
    require_file(root, "usr/bin/miaominal", executable=True)
    directory_icon = require_file(root, ".DirIcon")
    with directory_icon.open("rb") as stream:
        header = stream.read(24)
    if (len(header) != 24 or header[:8] != b"\x89PNG\r\n\x1a\n"
            or header[8:16] != b"\x00\x00\x00\rIHDR"
            or 0 in struct.unpack(">II", header[16:24])):
        raise ValueError(".DirIcon must be a PNG with nonzero dimensions")

    desktop_files = sorted(root.glob("*.desktop"))
    if len(desktop_files) != 1:
        raise ValueError("AppDir must contain exactly one root .desktop file")
    desktop = require_file(root, desktop_files[0].name)
    config = configparser.ConfigParser(interpolation=None, delimiters=("=",))
    config.optionxform = str
    config.read_string(desktop.read_text(encoding="utf-8"))
    if not config.has_section("Desktop Entry"):
        raise ValueError("desktop file is missing [Desktop Entry]")
    entry = config["Desktop Entry"]
    for key in ("Name", "Exec", "Icon"):
        if not entry.get(key, "").strip():
            raise ValueError(f"desktop file is missing {key}=")
    if entry.get("Type") != "Application":
        raise ValueError("desktop file must declare Type=Application")
    icon = entry["Icon"]
    if not re.fullmatch(r"[\w-]+", icon):
        raise ValueError("Icon= must be a basename without a path or extension")
    icons = [root / f"{icon}{suffix}" for suffix in (".png", ".svg")]
    if not any(path.is_file() for path in icons):
        raise ValueError(f"missing root icon matching Icon={icon}")
    for path in icons:
        if path.is_file():
            require_file(root, path.name)
    return desktop


LOADER_CHECK_MARKER = "MIAOMINAL_APPIMAGE_LOADER_CHECK="


def check_runtime_loader(root: Path) -> None:
    """Run AppRun's headless ELF-loader check, not the application GUI.

    Clear host loader overrides, then check that dependencies shipped in usr/lib
    actually resolve there. A build runner's GTK installation must not hide a
    broken AppRun, and tracing /bin/sh instead of Miaominal must not pass.
    """
    root = root.resolve(strict=True)
    launcher = require_file(root, "AppRun", executable=True)
    if launcher.is_symlink() or b"MIAOMINAL_APPIMAGE_CHECK_RUNTIME" not in launcher.read_bytes():
        raise ValueError("AppRun must be a library-loading launcher with a runtime check")
    environment = os.environ.copy()
    for name in ("LD_LIBRARY_PATH", "LD_PRELOAD", "LD_AUDIT", "LD_TRACE_LOADED_OBJECTS"):
        environment.pop(name, None)
    environment["APPDIR"] = str(root)
    environment["MIAOMINAL_APPIMAGE_CHECK_RUNTIME"] = "1"
    try:
        result = subprocess.run([str(launcher)], check=True, capture_output=True,
                                text=True, env=environment, timeout=30)
    except subprocess.CalledProcessError as error:
        raise ValueError(f"application loader check failed: {error.stderr or error}") from error
    lines = result.stdout.splitlines()
    marker = f"{LOADER_CHECK_MARKER}{root / 'usr/bin/miaominal'}"
    if marker not in lines:
        raise ValueError("AppRun did not trace the Miaominal application loader")
    if any(re.search(r"=>\s*not found(?:\s|$)", line) for line in lines):
        raise ValueError(f"application has unresolved shared libraries:\n{result.stdout}")
    bundled = 0
    for line in lines:
        match = re.fullmatch(r"\s*(\S+)\s+=>\s+(.+?)\s+\(0x[0-9a-fA-F]+\)\s*", line)
        if not match:
            continue
        name, resolved = match.groups()
        candidate = root / "usr/lib" / name
        if not candidate.is_file():
            continue
        expected = require_file(root, f"usr/lib/{name}").resolve()
        if Path(resolved).resolve() != expected:
            raise ValueError(f"bundled library {name} loaded outside the AppDir: {resolved}")
        bundled += 1
    if not bundled:
        raise ValueError("application loader did not load any bundled shared library")


def check_appimage(image: Path) -> None:
    check_filename(image.name)
    image = image.resolve(strict=True)
    if not os.access(image, os.X_OK):
        raise ValueError("AppImage must be executable")
    with image.open("rb") as stream:
        header = stream.read(11)
    if header[:4] != b"\x7fELF" or header[8:11] != b"AI\x02":
        raise ValueError("expected an ELF type-2 AppImage")
    with tempfile.TemporaryDirectory(prefix="miaominal-appimage-") as directory:
        # Native runtime extraction needs neither FUSE nor DISPLAY and checks
        # the published payload, not just cargo-bundle's staging directory.
        subprocess.run([str(image), "--appimage-extract"], cwd=directory,
                       check=True, stdout=subprocess.DEVNULL, timeout=120)
        root = Path(directory) / "squashfs-root"
        desktop = check_appdir(root)
        subprocess.run(["desktop-file-validate", str(desktop)], check=True, timeout=30)
        check_runtime_loader(root)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path)
    parser.add_argument("--appdir", action="store_true",
                        help="check an extracted AppDir instead of an AppImage")
    args = parser.parse_args()
    try:
        if args.appdir:
            check_appdir(args.path)
            check_runtime_loader(args.path)
        else:
            check_appimage(args.path)
    except (OSError, ValueError, configparser.Error, subprocess.SubprocessError) as error:
        print(f"AppImage validation failed: {error}", file=sys.stderr)
        return 1
    print(f"AppImage validation passed: {args.path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
