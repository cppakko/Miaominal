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
                       check=True, stdout=subprocess.DEVNULL)
        desktop = check_appdir(Path(directory) / "squashfs-root")
        subprocess.run(["desktop-file-validate", str(desktop)], check=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path)
    parser.add_argument("--appdir", action="store_true",
                        help="check an extracted AppDir instead of an AppImage")
    args = parser.parse_args()
    try:
        if args.appdir:
            check_appdir(args.path)
        else:
            check_appimage(args.path)
    except (OSError, ValueError, configparser.Error, subprocess.CalledProcessError) as error:
        print(f"AppImage validation failed: {error}", file=sys.stderr)
        return 1
    print(f"AppImage validation passed: {args.path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
