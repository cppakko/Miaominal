#!/usr/bin/env python3
"""Collect the release artifacts and write a build manifest.

Copies the APKs, R8 mapping, R8 retrace-free native symbols and the unstripped
Rust libraries into ``android/build/p7-artifacts`` (a non-committed build
directory) and writes ``build-manifest.txt`` plus ``SHA256SUMS`` describing the
exact toolchain, workspace state and artifact fingerprints.

Usage (from the repository root or anywhere):
    python android/scripts/write_build_manifest.py
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

ANDROID_DIR_DEFAULT = Path(__file__).resolve().parent.parent
OUT_DIR_DEFAULT = ANDROID_DIR_DEFAULT / "build" / "p7-artifacts"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run(command: list[str], cwd: Path | None = None) -> str:
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            capture_output=True,
            text=True,
            errors="replace",
            timeout=120,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        return f"<unavailable: {error}>"
    output = (result.stdout or "").strip()
    if not output or result.returncode != 0:
        output = (output + "\n" + (result.stderr or "").strip()).strip()
    if result.returncode != 0:
        return f"<exit {result.returncode}> {output}"
    return output or "<empty>"


def first_line(text: str) -> str:
    return text.splitlines()[0] if text.splitlines() else text


def parse_workspace_version(android_dir: Path) -> str:
    manifest = android_dir.parent / "Cargo.toml"
    for line in manifest.read_text(encoding="utf-8").splitlines():
        if line.startswith("version = "):
            return line.split('"')[1]
    return "<unknown>"


def apk_abis(apk: Path) -> str:
    if not apk.is_file():
        return "<missing>"
    with zipfile.ZipFile(apk) as archive:
        abis = sorted(
            {
                entry.split("/")[1]
                for entry in archive.namelist()
                if entry.startswith("lib/") and entry.count("/") >= 2
            }
        )
    return ",".join(abis) or "<none>"


def parse_abi_contract(android_dir: Path) -> str:
    source = (android_dir.parent / "crates/miaominal-android/src/lib.rs").read_text(
        encoding="utf-8"
    )
    for line in source.splitlines():
        if "ABI_VERSION" in line and "=" in line:
            return line.split('"')[1]
    return "<unknown>"


def parse_ndk_revision(android_dir: Path) -> str:
    sdk = os.environ.get("ANDROID_SDK_ROOT") or os.environ.get("ANDROID_HOME")
    candidates = []
    if sdk:
        candidates.append(Path(sdk) / "ndk")
    candidates.append(
        Path(os.environ.get("LOCALAPPDATA", "~")).expanduser() / "Android/Sdk/ndk"
    )
    for root in candidates:
        if not root.is_dir():
            continue
        for revision in sorted(root.iterdir(), reverse=True):
            source = revision / "source.properties"
            if source.is_file():
                for line in source.read_text(encoding="utf-8").splitlines():
                    if line.startswith("Pkg.Revision"):
                        return line.split("=", 1)[1].strip()
    return "<not found>"


def parse_output_metadata(android_dir: Path, build_type: str) -> dict:
    metadata = (
        android_dir / "app/build/outputs/apk" / build_type / "output-metadata.json"
    )
    if not metadata.is_file():
        return {}
    try:
        elements = json.loads(metadata.read_text(encoding="utf-8")).get("elements", [])
    except json.JSONDecodeError as error:
        return {"error": str(error)}
    return elements[0] if elements else {}


def collect_artifacts(android_dir: Path) -> list[tuple[str, Path]]:
    """Returns (relative destination path, source path) pairs.

    Labels are always explicit relative *file* paths: staged APKs keep their
    original file name, so no suffix heuristics are involved.
    """
    artifacts: list[tuple[str, Path]] = []

    def add(label: str, path: Path) -> None:
        if path.is_file():
            artifacts.append((label, path))

    apk_root = android_dir / "app/build/outputs/apk"
    add("apk/debug/app-debug.apk", apk_root / "debug/app-debug.apk")
    add("apk/release/app-release.apk", apk_root / "release/app-release.apk")
    # Variant copies staged by the release runbook (distinct ABIs overwrite the
    # single outputs/apk path, so each variant is copied aside after building).
    staging = android_dir / "build/p7-staging"
    if staging.is_dir():
        for staged in sorted(staging.glob("*.apk")):
            add(f"apk/{staged.name}", staged)
    add(
        "mapping/release/mapping.txt",
        android_dir / "app/build/outputs/mapping/release/mapping.txt",
    )
    add(
        "native-debug-symbols/release/native-debug-symbols.zip",
        android_dir / "app/build/outputs/native-debug-symbols/release/native-debug-symbols.zip",
    )
    for profile in ("debug", "release"):
        rust_root = android_dir / "native/build/rustJniLibs" / profile
        if rust_root.is_dir():
            for abi_dir in sorted(rust_root.iterdir()):
                if abi_dir.is_dir():
                    add(
                        f"native-symbols/{profile}/{abi_dir.name}/libmiaominal_android.so",
                        abi_dir / "libmiaominal_android.so",
                    )
    return artifacts


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--android-dir",
        type=Path,
        default=ANDROID_DIR_DEFAULT,
        help="android/ module directory",
    )
    parser.add_argument("--out-dir", type=Path, default=None, help="artifact output directory")
    args = parser.parse_args(argv)

    android_dir: Path = args.android_dir.resolve()
    out_dir: Path = (args.out_dir or OUT_DIR_DEFAULT).resolve()
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True)

    release_metadata = parse_output_metadata(android_dir, "release")
    debug_metadata = parse_output_metadata(android_dir, "debug")

    copied: list[tuple[str, Path, Path]] = []
    for label, source in collect_artifacts(android_dir):
        destination = out_dir / Path(label)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
        copied.append((label, source, destination))

    repository_root = android_dir.parent
    feature_tree = run(
        [
            "cargo",
            "tree",
            "-p",
            "miaominal-android",
            "-e",
            "features",
            "--target",
            "aarch64-linux-android",
            "--prefix",
            "none",
            "--depth",
            "1",
        ],
        cwd=repository_root,
    )
    mobile_features = [
        line.replace(' feature "', "=").replace('"', "")
        for line in feature_tree.splitlines()
        if line.startswith("miaominal-") and ' feature "' in line
    ]
    git_commit = run(["git", "rev-parse", "HEAD"], cwd=repository_root)
    git_status = run(["git", "status", "--porcelain"], cwd=repository_root)
    dirty_lines = [line for line in git_status.splitlines() if line.strip()]
    if git_status.startswith("<"):
        workspace_state = git_status
    elif dirty_lines:
        workspace_state = f"dirty ({len(dirty_lines)} changed/untracked paths)"
    else:
        workspace_state = "clean"

    cargo_lock = repository_root / "Cargo.lock"
    java_home = os.environ.get("JAVA_HOME", "<unset>")
    java_command = "java"
    java_candidate = Path(java_home) / "bin" / ("java.exe" if os.name == "nt" else "java")
    if java_candidate.is_file():
        java_command = str(java_candidate)

    lines = [
        "Miaominal Android build manifest",
        f"generated_at_utc = {dt.datetime.now(dt.timezone.utc).isoformat(timespec='seconds')}",
        "",
        "[version]",
        f"workspace_version = {parse_workspace_version(android_dir)}",
        f"release_versionName = {release_metadata.get('versionName', '<unknown>')}",
        f"release_versionCode = {release_metadata.get('versionCode', '<unknown>')}",
        "release_abis = "
        + apk_abis(android_dir / "app/build/outputs/apk/release/app-release.apk"),
        f"debug_versionName = {debug_metadata.get('versionName', '<unknown>')}",
        f"debug_versionCode = {debug_metadata.get('versionCode', '<unknown>')}",
        "debug_abis = "
        + apk_abis(android_dir / "app/build/outputs/apk/debug/app-debug.apk"),
        f"abi_contract = {parse_abi_contract(android_dir)}",
        "",
        "[workspace]",
        f"git_commit = {first_line(git_commit)}",
        f"workspace_state = {workspace_state}",
        f"cargo_lock_sha256 = {sha256(cargo_lock) if cargo_lock.is_file() else '<missing>'}",
        "",
        "[toolchain]",
        f"java_home = {java_home}",
        f"java = {first_line(run([java_command, '-version']))}",
        f"rustc = {first_line(run(['rustc', '--version']))}",
        f"cargo = {first_line(run(['cargo', '--version']))}",
        f"cargo_ndk = {first_line(run(['cargo', 'ndk', '--version']))}",
        f"ndk_revision = {parse_ndk_revision(android_dir)}",
        "bridge_package = miaominal-android",
        "bridge_profile_release = android-release (inherits release, debug=1)",
        "locked_build = cargo ndk ... build --locked",
        f"mobile_features = {', '.join(mobile_features) or '<unavailable>'}",
        "",
        "[artifacts]",
    ]
    for label, source, destination in copied:
        detail = f"{destination.relative_to(out_dir).as_posix()} "
        if destination.suffix == ".apk":
            detail += f"[abis {apk_abis(destination)}] "
        detail += f"({destination.stat().st_size} bytes, sha256 {sha256(destination)})"
        lines.append(f"{label} = {detail}")

    missing = [
        label
        for label, path in [
            ("apk/debug", android_dir / "app/build/outputs/apk/debug/app-debug.apk"),
            ("apk/release", android_dir / "app/build/outputs/apk/release/app-release.apk"),
            (
                "mapping/release",
                android_dir / "app/build/outputs/mapping/release/mapping.txt",
            ),
        ]
        if not path.is_file()
    ]
    if missing:
        lines.append(f"missing_expected_artifacts = {', '.join(missing)}")

    manifest_path = out_dir / "build-manifest.txt"
    manifest_path.write_text("\n".join(lines) + "\n", encoding="utf-8")

    checksums = out_dir / "SHA256SUMS"
    entries = []
    for _label, _source, destination in copied:
        entries.append(f"{sha256(destination)}  {destination.relative_to(out_dir).as_posix()}")
    entries.append(f"{sha256(manifest_path)}  build-manifest.txt")
    checksums.write_text("\n".join(entries) + "\n", encoding="utf-8")

    print(f"manifest: {manifest_path}")
    print(f"checksums: {checksums}")
    for label, _source, destination in copied:
        print(f"  {label}: {destination}")
    if missing:
        print(f"WARNING missing expected artifacts: {', '.join(missing)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
