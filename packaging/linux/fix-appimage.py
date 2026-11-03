#!/usr/bin/env python3
"""Install Miaominal's AppRun and repack a cargo-bundle 0.12 type-2 image.

The original architecture-specific runtime is preserved byte for byte. Only the
SquashFS payload is rebuilt, using squashfs-tools already installed by release CI.
No appimagetool download or undocumented cargo-bundle staging hook is required.
"""

import argparse
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from check_appimage import check_appdir


LAUNCHER = Path(__file__).resolve().with_name("AppRun")


def install_launcher(root: Path) -> None:
    root = root.resolve(strict=True)
    check_appdir(root)
    launcher = root / "AppRun"
    # Check the parent, not the leaf: cargo-bundle's AppRun is a symlink to
    # usr/bin/miaominal. Unlink that entry without following it into the binary.
    if launcher.parent.resolve(strict=True) != root:
        raise ValueError("AppRun is outside the extracted AppDir")
    launcher.unlink()
    launcher.write_bytes(LAUNCHER.read_bytes())
    launcher.chmod(0o755)


def runtime_offset(image: Path) -> int:
    with image.open("rb") as stream:
        header = stream.read(11)
    if header[:4] != b"\x7fELF" or header[8:11] != b"AI\x02":
        raise ValueError("expected an ELF type-2 AppImage")
    result = subprocess.run([str(image), "--appimage-offset"], check=True,
                            capture_output=True, text=True, timeout=30)
    value = result.stdout.strip()
    if not value.isdecimal():
        raise ValueError("AppImage runtime did not return a numeric payload offset")
    offset = int(value)
    if offset < len(header) or offset >= image.stat().st_size:
        raise ValueError("AppImage payload offset is outside the image")
    with image.open("rb") as stream:
        stream.seek(offset)
        if stream.read(4) != b"hsqs":
            raise ValueError("AppImage payload offset does not point to SquashFS")
    return offset


def fix_appimage(image: Path) -> None:
    if image.is_symlink():
        raise ValueError("refusing to replace an AppImage symlink")
    image = image.resolve(strict=True)
    offset = runtime_offset(image)
    mode = stat.S_IMODE(image.stat().st_mode) | 0o111
    with tempfile.TemporaryDirectory(prefix="miaominal-appimage-repack-") as directory:
        stage = Path(directory).resolve(strict=True)
        subprocess.run([str(image), "--appimage-extract"], cwd=stage, check=True,
                       stdout=subprocess.DEVNULL, timeout=120)
        root = stage / "squashfs-root"
        if root.is_symlink() or root.resolve(strict=True) != root:
            raise ValueError("extracted AppDir is outside the temporary staging directory")
        install_launcher(root)
        payload = stage / "payload.squashfs"
        subprocess.run(["mksquashfs", str(root), str(payload), "-noappend",
                        "-all-root", "-comp", "gzip", "-no-xattrs", "-no-progress"],
                       check=True, timeout=300)
        with payload.open("rb") as stream:
            if stream.read(4) != b"hsqs":
                raise ValueError("mksquashfs did not produce a SquashFS payload")
        replacement = None
        try:
            with tempfile.NamedTemporaryFile(prefix=f".{image.name}.",
                                             dir=image.parent, delete=False) as output:
                replacement = Path(output.name).resolve(strict=True)
                with image.open("rb") as original:
                    remaining = offset
                    while remaining:
                        block = original.read(min(remaining, 1024 * 1024))
                        if not block:
                            raise ValueError("AppImage runtime was truncated during repacking")
                        output.write(block)
                        remaining -= len(block)
                with payload.open("rb") as stream:
                    shutil.copyfileobj(stream, output)
                output.flush()
                os.fsync(output.fileno())
            replacement.chmod(mode)
            # Replacement is atomic and only targets the exact resolved input;
            # a failed extraction/pack never damages the original release image.
            if (image.resolve(strict=True) != image
                    or replacement.parent.resolve(strict=True) != image.parent):
                raise ValueError("AppImage replacement paths changed during repacking")
            os.replace(replacement, image)
        finally:
            if replacement is not None and replacement.exists():
                if replacement.parent.resolve(strict=True) != image.parent:
                    raise ValueError("unexpected temporary AppImage cleanup path")
                replacement.unlink()
    print(f"Installed runtime library launcher in {image}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    args = parser.parse_args()
    try:
        fix_appimage(args.image)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"AppImage repack failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
