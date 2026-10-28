#!/usr/bin/env python3
"""Collect licenses embedded in all supported desktop distributions."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import importlib.util
import json
import re
import subprocess
import threading
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "assets/licenses"
spec = importlib.util.spec_from_file_location("notice_collector", ROOT / "android/scripts/generate_third_party_notices.py")
collector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(collector)
collector.CACHE = ROOT / "target/third-party-license-cache"
collector.LEGAL_NAME = re.compile(r"^(?:LICEN[CS]E|COPYING|NOTICES?|COPYRIGHT|OFL)(?:[-._]|$)", re.I)
TARGETS = (
    "x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc",
    "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu",
    "aarch64-apple-darwin", "x86_64-apple-darwin",
)
LUCIDE_LICENSE_REVISION = "a04f228cd01185e09c188b7227b9600c08c565ec"
REPOSITORY_LOCKS: dict[str, threading.Lock] = {}
LOCK = threading.Lock()
STANDARD_LICENSE_EXCEPTIONS = {
    ("block", "0.1.6"): "MIT",
    ("malloc_buf", "0.0.6"): "MIT",
    ("fax", "0.2.6"): "MIT",
    ("fax_derive", "0.2.0"): "MIT",
    ("seahash", "4.1.0"): "MIT",
    ("hexf-parse", "0.2.1"): "CC0-1.0",
    ("eventsource-stream", "0.2.3"): "Apache-2.0",
    ("mac", "0.1.1"): "Apache-2.0",
    ("pathfinder_geometry", "0.5.1"): "Apache-2.0",
    ("leak", "0.1.2"): "Apache-2.0",
    ("leaky-cow", "0.1.1"): "Apache-2.0",
}
SPDX_REVISION = "60e0de29cbcf3e83bf9d5e299972a1969516d918"  # v3.27.0
ORIGINAL_DOWNLOAD = collector.download


def git_source(repository: str, revision: str) -> Path:
    checkout = collector.CACHE / "git" / hashlib.sha256(repository.encode()).hexdigest()
    with LOCK:
        lock = REPOSITORY_LOCKS.setdefault(repository, threading.Lock())
    with lock:
        if not checkout.exists():
            checkout.parent.mkdir(parents=True, exist_ok=True)
            collector.run(["git", "init", "--bare", str(checkout)])
        exists = subprocess.run(["git", "cat-file", "-e", revision + "^{commit}"], cwd=checkout, capture_output=True).returncode == 0
        if not exists:
            result = subprocess.run(["git", "fetch", "--quiet", "--depth=1", repository, revision], cwd=checkout,
                                    capture_output=True, text=True, encoding="utf-8", timeout=90)
            if result.returncode:
                raise ValueError(f"Cannot retrieve {repository} at {revision}: {result.stderr.strip()}")
    return checkout


def download_evidence(url: str) -> bytes:
    """Read GitHub documents from pinned Git objects, avoiding raw-host routing."""
    path = collector.CACHE / hashlib.sha256(url.encode()).hexdigest()
    if path.is_file():
        return path.read_bytes()
    if not url.startswith("https://raw.githubusercontent.com/"):
        return ORIGINAL_DOWNLOAD(url)
    owner, repo, revision, file = url.removeprefix("https://raw.githubusercontent.com/").split("/", 3)
    checkout = git_source(f"https://github.com/{owner}/{repo}", revision)
    data = collector.run(["git", "show", revision + ":" + file], checkout).encode("utf-8")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)
    return data


def standard_notices(pkg: dict) -> list[dict]:
    license_id = STANDARD_LICENSE_EXCEPTIONS[(pkg["name"], pkg["version"])]
    if license_id not in pkg["license"]:
        raise ValueError(f"Selected license no longer matches {pkg['name']} declaration")
    directory = Path(pkg["manifest_path"]).parent
    declaration = directory / "Cargo.toml.orig"
    if not declaration.exists():
        declaration = directory / "Cargo.toml"
    url = f"https://raw.githubusercontent.com/spdx/license-list-data/{SPDX_REVISION}/text/{license_id}.txt"
    content = collector.text(collector.download(url))
    if license_id == "MIT":
        # SPDX's copyright line is a template, not an upstream copyright
        # notice. Preserve actual notices separately without inventing years.
        content = content.replace("Copyright (c) <year> <copyright holders>\n\n", "")
    return [collector.document("License declaration in published source archive", f"https://crates.io/api/v1/crates/{pkg['name']}/{pkg['version']}/download!/{declaration.name}", collector.text(declaration.read_bytes())),
            collector.document(f"Standard {license_id} text (no standalone license in source archive)", url, content)]


def upstream_notices(pkg: dict) -> list[dict]:
    directory = Path(pkg["manifest_path"]).parent
    if (pkg["name"], pkg["version"]) in STANDARD_LICENSE_EXCEPTIONS:
        return standard_notices(pkg)
    if pkg["name"] == "libappindicator-sys":
        parent = next(p for p in collector.METADATA["packages"] if p["name"] == "libappindicator" and p["version"] == pkg["version"])
        return collector.local_crate_notices(parent)
    repository = (pkg.get("repository") or "").replace("http://github.com/", "https://github.com/")
    if pkg["name"].startswith("zune-"):
        repository = "https://github.com/etemesi254/zune-image"
    if pkg["name"] == "tree-sitter-diff":
        repository = "https://github.com/tree-sitter-grammars/tree-sitter-diff"
    repository = repository.split("/tree/")[0].removesuffix(".git").rstrip("/")
    vcs_file = directory / ".cargo_vcs_info.json"
    if vcs_file.exists():
        vcs = json.loads(vcs_file.read_text())
        revision, subdir = vcs["git"]["sha1"], vcs.get("path_in_vcs", "")
    else:
        return ORIGINAL_MISSING_NOTICES(pkg)
    if not repository:
        raise ValueError(f"No upstream repository for {pkg['name']}")
    checkout = git_source(repository, revision)
    files = collector.run(["git", "ls-tree", "-r", "--name-only", revision], checkout).splitlines()
    ancestors = {str(p).replace("\\", "/").strip(".") for p in [Path(subdir), *Path(subdir).parents, Path("")]}
    paths = [p for p in files if collector.LEGAL_NAME.match(Path(p).name) and Path(p).parent.as_posix().strip(".") in ancestors]
    result = []
    for path in sorted(paths):
        if repository.startswith("https://github.com/"):
            origin = repository.replace("https://github.com/", "https://raw.githubusercontent.com/") + "/" + revision + "/" + path
        else:
            origin = repository + "/-/raw/" + revision + "/" + path
        result.append(collector.document(path, origin, collector.run(["git", "show", revision + ":" + path], checkout)))
    if not result:
        raise ValueError(f"No upstream license found for {pkg['name']} {pkg['version']}")
    return result


ORIGINAL_MISSING_NOTICES = collector.missing_crate_notices
collector.missing_crate_notices = upstream_notices
collector.download = download_evidence


def generate() -> dict[str, str]:
    collector.DOCUMENTS.clear()
    collector.METADATA = json.loads(collector.run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"]))
    selected: dict[tuple[str, str], list[str]] = {}
    for target in TARGETS:
        tree = collector.run(["cargo", "tree", "-p", "miaominal", "--target", target, "--edges", "normal", "--prefix", "none", "--format", "{p}", "--locked", "--offline"])
        for line in tree.splitlines():
            match = re.match(r"^([^ ]+) v([^ |]+)", line)
            if match:
                targets = selected.setdefault((match[1], match[2]), [])
                if target not in targets:
                    targets.append(target)
    packages = [p for p in collector.METADATA["packages"] if p["source"] and (p["name"], p["version"]) in selected]
    print(f"Collecting {len(packages)} desktop dependency versions across {len(TARGETS)} targets.", flush=True)
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        futures = {pool.submit(collector.native_component, p, selected[(p["name"], p["version"])]): p for p in packages}
        components, errors = [], []
        for future in concurrent.futures.as_completed(futures):
            pkg = futures[future]
            try:
                components.append(future.result())
            except Exception as error:
                errors.append(f"{pkg['name']} {pkg['version']}: {error}")
        if errors:
            raise ValueError("Missing license evidence:\n" + "\n".join(sorted(errors)))
    # two-face's syntax/theme licenses are embedded in a binary archive, so the
    # ordinary license-file scan cannot read them. Include its pinned full list.
    for pkg in packages:
        if pkg["name"] == "two-face":
            directory = Path(pkg["manifest_path"]).parent
            revision = json.loads((directory / ".cargo_vcs_info.json").read_text())["git"]["sha1"]
            origin = f"https://raw.githubusercontent.com/CosmicHorrorDev/two-face/{revision}/generated/acknowledgements_full.md"
            component = next(row for row in components if row["name"] == f"two-face@{pkg['version']}")
            component["documents"].append(collector.document("Embedded syntax and theme licenses", origin, collector.text(collector.download(origin))))
    lucide = "https://raw.githubusercontent.com/lucide-icons/lucide/" + LUCIDE_LICENSE_REVISION + "/LICENSE"
    components.append({"name": "Lucide SVG icons (bundled files)", "kind": "bundled-assets", "declared_license": "ISC; original Feather portions MIT",
                       "source": lucide, "copyrights": [], "documents": [collector.document("Lucide and Feather icon licenses", lucide, collector.text(collector.download(lucide)))]})
    sqlite = next(p for p in packages if p["name"] == "libsqlite3-sys")
    sqlite_source = Path(sqlite["manifest_path"]).parent / "sqlite3/sqlite3.c"
    dedication = collector.text(sqlite_source.read_bytes()).split("*/", 1)[0] + "*/\n"
    components.append({"name": "SQLite (bundled by libsqlite3-sys " + sqlite["version"] + ")", "kind": "embedded-native", "declared_license": "Public domain",
                       "source": "https://www.sqlite.org/copyright.html", "copyrights": [],
                       "documents": [collector.document("SQLite public-domain dedication", sqlite["repository"] + "!/sqlite3/sqlite3.c", dedication)]})
    components.sort(key=lambda row: (row["kind"], row["name"]))
    for row in components:
        row["documents"] = sorted({(d["sha256"], d["title"]): d for d in row["documents"]}.values(), key=lambda d: (d["title"], d["sha256"]))
        identity = tuple(row["name"].split("@"))
        if identity in STANDARD_LICENSE_EXCEPTIONS:
            row["selected_license"] = STANDARD_LICENSE_EXCEPTIONS[identity]
            row["license_evidence"] = "Published source archive declares this license but contains no standalone license document; standard text supplied. Original author metadata and source copyright notices are retained."
        if row["name"].startswith("self_cell@"):
            row["selected_license"] = "Apache-2.0"
    manifest = {"schema": 1, "scope": "Desktop normal-dependency graphs across supported Windows, Linux and macOS architectures, plus bundled icon assets. Includes proc-macros conservatively; excludes test dependencies and build tools.",
                "targets": TARGETS, "components": components, "documents": {digest: {"sha256": digest} for digest in sorted(collector.DOCUMENTS)}}
    lines = ["Miaominal Desktop - Third-party licenses and notices", "", "Copyright (c) 2026 akko.",
             "Miaominal is GPL-3.0-only. Third-party components retain their original licenses.",
             "OR in upstream declarations denotes alternatives, not cumulative obligations.",
             "Selected alternatives and standard-text exceptions are identified per component.",
             "MPL-covered files retain their MPL terms and source availability.",
             "This inventory covers all supported platforms and includes proc-macros conservatively.",
             "Source archives and repositories are identified for each component.", ""]
    for row in components:
        lines.extend(["=" * 78, row["name"], "License: " + row["declared_license"], "Source: " + row["source"]])
        if row.get("selected_license"):
            lines.append("Selected license: " + row["selected_license"])
        if row.get("upstream"):
            lines.append("Upstream: " + row["upstream"])
        if row.get("developers"):
            lines.append("Upstream authors/developers: " + "; ".join(row["developers"]))
        if row.get("license_evidence"):
            lines.append("License evidence: " + row["license_evidence"])
        lines.extend(row["copyrights"])
        for doc in row["documents"]:
            lines.extend(["Document: " + doc["title"], "Origin: " + doc["source"], "Full text SHA-256: " + doc["sha256"]])
        lines.append("")
    lines.extend(["=" * 78, "FULL LICENSE AND NOTICE TEXTS", "Identical texts appear once; SHA-256 references above identify them.", ""])
    for digest, content in sorted(collector.DOCUMENTS.items()):
        lines.extend(["=" * 78, "SHA-256: " + digest, "", content])
    return {"THIRD_PARTY_NOTICES.txt": "\n".join(lines).rstrip() + "\n", "manifest.json": json.dumps(manifest, ensure_ascii=False, indent=2) + "\n"}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    outputs = generate()
    if args.check:
        stale = [name for name, value in outputs.items() if not (OUTPUT / name).is_file() or (OUTPUT / name).read_text(encoding="utf-8") != value]
        if stale:
            raise SystemExit("Desktop license assets need regeneration: " + ", ".join(stale))
        print("Desktop license assets match resolved dependencies and source notices.")
    else:
        OUTPUT.mkdir(parents=True, exist_ok=True)
        for name, value in outputs.items():
            (OUTPUT / name).write_text(value, encoding="utf-8", newline="\n")
        print(f"Wrote desktop license assets to {OUTPUT}")


if __name__ == "__main__":
    main()
