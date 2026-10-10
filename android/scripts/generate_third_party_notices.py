#!/usr/bin/env python3
"""Collect versioned Android dependency licenses into APK assets (stdlib only)."""

from __future__ import annotations

import argparse
import concurrent.futures
import hashlib
import io
import json
import os
import re
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import xml.etree.ElementTree as ET
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ANDROID = ROOT / "android"
ASSETS = ANDROID / "app/src/main/assets/licenses"
CACHE = ANDROID / "build/third-party-license-cache"
LEGAL_NAME = re.compile(r"^(?:LICEN[CS]E|COPYING|NOTICE|COPYRIGHT)(?:[-._]|$)", re.I)
COPYRIGHT = re.compile(r"Copyright\s*(?:\([cC]\)|©|\d{4})[^\r\n]*", re.I)
NS = {"m": "http://maven.apache.org/POM/4.0.0"}
DOCUMENTS: dict[str, str] = {}


def run(args: list[str], cwd: Path = ROOT) -> str:
    result = subprocess.run(args, cwd=cwd, check=True, capture_output=True, text=True, encoding="utf-8")
    return result.stdout


def download(url: str) -> bytes:
    path = CACHE / hashlib.sha256(url.encode()).hexdigest()
    if path.exists():
        return path.read_bytes()
    missing = path.with_suffix(".404")
    if missing.exists():
        raise urllib.error.HTTPError(url, 404, "Previously verified missing file", None, None)
    request = urllib.request.Request(url, headers={"User-Agent": "Miaominal-Third-Party-Notices/1.0"})
    for attempt in range(3):
        try:
            with urllib.request.urlopen(request, timeout=15) as response:
                data = response.read()
            break
        except urllib.error.HTTPError as error:
            if error.code == 404:
                CACHE.mkdir(parents=True, exist_ok=True)
                missing.write_text("404\n", encoding="ascii")
            raise
        except (urllib.error.URLError, TimeoutError) as error:
            if attempt == 2:
                raise RuntimeError(f"Could not download license evidence: {url}") from error
            time.sleep(attempt + 1)
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=CACHE, delete=False) as file:
        file.write(data)
        temporary = Path(file.name)
    temporary.replace(path)
    return data


def text(data: bytes) -> str:
    return data.decode("utf-8-sig", errors="replace").replace("\r\n", "\n")


def document(label: str, source: str, content: str) -> dict:
    content = "\n".join(line.rstrip() for line in content.splitlines()).strip() + "\n"
    if not content.strip() or content.lstrip().lower().startswith(("<!doctype html", "<html")):
        raise ValueError(f"Invalid license text: {source}")
    digest = hashlib.sha256(content.encode()).hexdigest()
    DOCUMENTS[digest] = content
    return {"title": label, "source": source, "sha256": digest}


def gradle_components() -> dict[str, list[str]]:
    script = """
gradle.projectsEvaluated {
    def app = gradle.rootProject.findProject(':app')
    if (app == null) return
    app.tasks.register('exportThirdPartyLicenseDependencies') {
        doLast {
            def variants = [:]
            ['releaseRuntimeClasspath', 'debugRuntimeClasspath'].each { name ->
                def result = app.configurations.getByName(name).incoming.resolutionResult
                def failed = result.allDependencies.findAll { it instanceof org.gradle.api.artifacts.result.UnresolvedDependencyResult }
                if (!failed.empty) throw new GradleException("Unresolved dependencies: " + failed.collect { it.attempted })
                variants[name] = result.allComponents.collect { it.id }.findAll { it instanceof org.gradle.api.artifacts.component.ModuleComponentIdentifier }.collect { "${it.group}:${it.module}:${it.version}" }.unique().sort()
            }
            println 'MIAOMINAL_LICENSE_GRAPH=' + groovy.json.JsonOutput.toJson(variants)
        }
    }
}
"""
    with tempfile.TemporaryDirectory(prefix="miaominal-licenses-") as temp:
        init = Path(temp) / "licenses.init.gradle"
        init.write_text(script, encoding="utf-8")
        wrapper = str(ANDROID / ("gradlew.bat" if os.name == "nt" else "gradlew"))
        output = run([wrapper, "-I", str(init), ":app:exportThirdPartyLicenseDependencies", "--offline", "--no-configuration-cache", "--console=plain"], ANDROID)
    for line in output.splitlines():
        if line.startswith("MIAOMINAL_LICENSE_GRAPH="):
            return json.loads(line.split("=", 1)[1])
    raise ValueError("Gradle did not export dependency coordinates")


def archive_notices(data: bytes, source: str, copyrights: set[str], sources: bool = False) -> list[dict]:
    result = []
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        for entry in archive.infolist():
            if entry.is_dir():
                continue
            if LEGAL_NAME.match(Path(entry.filename).name):
                result.append(document(entry.filename, source + "!/" + entry.filename, text(archive.read(entry))))
            elif entry.filename == "classes.jar":
                result.extend(archive_notices(archive.read(entry), source + "!/classes.jar", copyrights))
            elif sources and entry.filename.endswith((".java", ".kt", ".c", ".h", ".cpp")):
                copyrights.update(COPYRIGHT.findall(text(archive.read(entry))[:5000]))
    return result


def pom_licenses(pom: ET.Element, depth: int = 0) -> list[str]:
    licenses = [node.findtext("m:name", namespaces=NS) for node in pom.findall("m:licenses/m:license", NS)]
    if licenses:
        return licenses
    parent = pom.find("m:parent", NS)
    if parent is None or depth >= 8:
        return []
    group, name, version = [parent.findtext("m:" + key, namespaces=NS) for key in ("groupId", "artifactId", "version")]
    base = "https://repo.maven.apache.org/maven2/"
    url = base + "/".join((group.replace(".", "/"), name, version, name + "-" + version + ".pom"))
    return pom_licenses(ET.fromstring(download(url)), depth + 1)


def maven_component(coord: str, variants: dict[str, list[str]]) -> dict:
    group, name, version = coord.split(":")
    base = "https://dl.google.com/dl/android/maven2/" if group.startswith("androidx.") or group == "com.google.android.material" else "https://repo.maven.apache.org/maven2/"
    base += "/".join((group.replace(".", "/"), name, version)) + "/" + name + "-" + version
    pom_url = base + ".pom"
    cache = Path(os.environ.get("GRADLE_USER_HOME", str(Path.home() / ".gradle"))) / "caches/modules-2/files-2.1" / group / name / version
    cached_poms = sorted(cache.glob("*/*.pom"))
    pom = ET.fromstring(cached_poms[0].read_bytes() if cached_poms else download(pom_url))
    declared = pom_licenses(pom)
    if not declared:
        raise ValueError(f"No declared Maven license: {coord}")
    if coord.startswith("net.java.dev.jna:jna:") and "Apache-2.0" in declared:
        selected = "Apache-2.0"
    elif all("Apache" in license for license in declared):
        selected = "Apache-2.0"
    else:
        raise ValueError(f"Review new Maven license before generating: {coord}: {declared}")
    row = {"name": coord, "kind": "maven", "declared_license": " OR ".join(declared), "selected_license": selected,
           "configurations": sorted(key for key, ids in variants.items() if coord in ids), "source": pom_url,
           "upstream": pom.findtext("m:scm/m:url", namespaces=NS) or pom.findtext("m:url", namespaces=NS),
           "developers": sorted(set(node.text.strip() for node in pom.findall("m:developers/m:developer/m:name", NS) if node.text)),
           "documents": [], "copyrights": []}
    copyrights: set[str] = set()
    for path in sorted(cache.glob("*/*")):
        if path.suffix in (".aar", ".jar") and not path.name.endswith(("-sources.jar", "-javadoc.jar")):
            row["documents"].extend(archive_notices(path.read_bytes(), base + path.suffix, copyrights))
    # Gradle also caches classified artifacts such as samples-sources.jar.
    # Only the library's exact Maven sources artifact matches the URL below.
    sources = sorted(cache.glob(f"*/{name}-{version}-sources.jar"))
    if sources:
        source_data = sources[0].read_bytes()
    else:
        try:
            source_data = download(base + "-sources.jar")
        except urllib.error.HTTPError as error:
            if error.code != 404:
                raise
            source_data = None  # BOMs and forwarding modules often publish no sources.
    if source_data:
        row["documents"].extend(archive_notices(source_data, base + "-sources.jar", copyrights, sources=True))
    row["copyrights"] = sorted(copyrights)
    row["documents"].append(document("Apache License 2.0", "https://www.apache.org/licenses/LICENSE-2.0.txt", text(download("https://www.apache.org/licenses/LICENSE-2.0.txt"))))
    return row


def missing_crate_notices(pkg: dict) -> list[dict]:
    directory = Path(pkg["manifest_path"]).parent
    vcs_file = directory / ".cargo_vcs_info.json"
    repository = (pkg.get("repository") or "").removesuffix(".git").rstrip("/")
    if not repository.startswith("https://github.com/"):
        raise ValueError(f"Cannot retrieve versioned license for {pkg['name']}")
    if vcs_file.exists():
        vcs = json.loads(vcs_file.read_text())
        revision, subdir = vcs["git"]["sha1"], vcs.get("path_in_vcs", "")
    elif pkg["name"] == "rustls-platform-verifier-android":
        # This crate embeds the JVM AAR and omits VCS metadata; its parent crate
        # ships the identical project license and a revision for the release.
        parent = next(p for p in METADATA["packages"] if p["name"] == "rustls-platform-verifier")
        return local_crate_notices(parent)
    else:
        raise ValueError(f"Missing source revision for {pkg['name']}")
    raw = repository.replace("https://github.com/", "https://raw.githubusercontent.com/") + "/" + revision + "/"
    result = []
    # License and NOTICE files in a workspace may live above the crate directory.
    directories = list(dict.fromkeys([subdir] + [str(p) for p in Path(subdir).parents] + [""]))
    for filename in ("LICENSE", "LICENSE.txt", "LICENSE.md", "LICENSE-MIT", "LICENSE-APACHE", "LICENSE-2.0.txt", "LICENCE", "NOTICE", "NOTICE.txt", "COPYRIGHT"):
        for prefix in directories:
            url = raw + (prefix.strip("./") + "/" if prefix.strip("./") else "") + filename
            try:
                content = download(url)
            except urllib.error.HTTPError as error:
                if error.code == 404:
                    continue
                raise
            result.append(document(filename, url, text(content)))
            break
    if not result:
        raise ValueError(f"No upstream license found for {pkg['name']} {pkg['version']}")
    return result


def local_crate_notices(pkg: dict) -> list[dict]:
    directory = Path(pkg["manifest_path"]).parent
    source = f"https://crates.io/api/v1/crates/{pkg['name']}/{pkg['version']}/download"
    result = []
    for path in sorted(directory.rglob("*")):
        if path.is_file() and LEGAL_NAME.match(path.name):
            relative = path.relative_to(directory).as_posix()
            result.append(document(relative, source + "!/" + relative, text(path.read_bytes())))
    return result


def native_component(pkg: dict, targets: list[str]) -> dict:
    directory = Path(pkg["manifest_path"]).parent
    documents = local_crate_notices(pkg) or missing_crate_notices(pkg)
    copyrights: set[str] = set()
    for path in sorted(directory.glob("src/**/*.rs")):
        copyrights.update(COPYRIGHT.findall(text(path.read_bytes())[:5000]))
    # Some crates ship embedded JVM code; retain its own license/NOTICE entries.
    for path in sorted(directory.rglob("*.aar")):
        documents.extend(archive_notices(path.read_bytes(), f"{pkg['repository']}!/{path.relative_to(directory).as_posix()}", copyrights))
    return {"name": pkg["name"] + "@" + pkg["version"], "kind": "cargo", "declared_license": pkg["license"],
            "targets": targets, "source": f"https://crates.io/api/v1/crates/{pkg['name']}/{pkg['version']}/download",
            "upstream": pkg.get("repository"), "developers": pkg.get("authors", []),
            "documents": documents, "copyrights": sorted(copyrights)}


METADATA: dict = {}


def generate() -> dict[str, str]:
    global METADATA
    METADATA = json.loads(run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"]))
    variants = gradle_components()
    coordinates = sorted(set().union(*variants.values()))
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        components = list(pool.map(lambda coord: maven_component(coord, variants), coordinates))
    print(f"Collected notices for {len(components)} Android Maven modules.", flush=True)
    target_packages: dict[tuple[str, str], list[str]] = {}
    for target in ("aarch64-linux-android", "x86_64-linux-android"):
        tree = run(["cargo", "tree", "-p", "miaominal-android", "--target", target, "--edges", "normal", "--prefix", "none", "--format", "{p}", "--locked", "--offline"])
        for line in tree.splitlines():
            match = re.match(r"^([^ ]+) v([^ |]+)", line)
            if match:
                ids = target_packages.setdefault((match[1], match[2]), [])
                if target not in ids:
                    ids.append(target)
    packages = [pkg for pkg in METADATA["packages"] if pkg["source"] and (pkg["name"], pkg["version"]) in target_packages]
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        components.extend(pool.map(lambda pkg: native_component(pkg, target_packages[(pkg["name"], pkg["version"])]), packages))
    print(f"Collected notices for {len(packages)} Android Cargo dependencies.", flush=True)
    # The generated Kotlin helper code comes from the bindgen templates, even
    # though the generator itself is not linked into the APK.
    bindgen = next(pkg for pkg in METADATA["packages"] if pkg["name"] == "uniffi_bindgen")
    row = native_component(bindgen, [])
    row["kind"] = "generated-code"
    row["usage"] = "UniFFI Kotlin helper code and templates included in generated bindings."
    components.append(row)
    jna_version = next(coord.split(":")[2] for coord in coordinates if coord.startswith("net.java.dev.jna:jna:"))
    jna_source = "https://raw.githubusercontent.com/java-native-access/jna/" + jna_version
    extra = [
        ("JNA native dispatch " + jna_version, "Apache-2.0", jna_source + "/LICENSE"),
        ("libffi (bundled by JNA " + jna_version + ")", "MIT", jna_source + "/native/libffi/LICENSE"),
    ]
    for name, license, url in extra:
        components.append({"name": name, "kind": "embedded-native", "declared_license": license, "selected_license": license,
                           "source": url, "documents": [document(name, url, text(download(url)))], "copyrights": []})
    # sqlite3.c carries SQLite's public-domain dedication in its header.
    sqlite = next((pkg for pkg in packages if pkg["name"] == "libsqlite3-sys"), None)
    if sqlite is not None:
        sqlite_file = Path(sqlite["manifest_path"]).parent / "sqlite3/sqlite3.c"
        if not sqlite_file.is_file():
            raise ValueError("Missing bundled SQLite source/dedication")
        dedication = text(sqlite_file.read_bytes()).split("*/", 1)[0] + "*/\n"
        components.append({"name": "SQLite (bundled by libsqlite3-sys " + sqlite["version"] + ")", "kind": "embedded-native", "declared_license": "Public domain",
                           "source": "https://www.sqlite.org/copyright.html", "documents": [document("SQLite public-domain dedication", sqlite["repository"] + "!/sqlite3/sqlite3.c", dedication)], "copyrights": []})
    components.sort(key=lambda component: (component["kind"], component["name"]))
    for row in components:
        # The same notice may occur in both an AAR and its source JAR.
        row["documents"] = sorted({(item["sha256"], item["title"]): item for item in row["documents"]}.values(), key=lambda item: (item["title"], item["sha256"]))
    manifest = {"schema": 1, "scope": "Android release and debug application dependencies, both supported ABIs; excludes test APKs and build tools except generated UniFFI helper code.",
                "components": components, "documents": {digest: {"sha256": digest} for digest in sorted(DOCUMENTS)}}
    notice = ["Miaominal Android - Third-party licenses and notices", "",
              "Copyright (c) 2026 akko.",
              "Miaominal is GPL-3.0-only. Third-party components retain their original licenses.",
              "OR in an upstream declaration denotes alternatives, not cumulative obligations.",
              "JNA is used under Apache 2.0; its bundled libffi retains its MIT license.",
              "MPL-covered UniFFI source/templates and option-ext remain available under MPL 2.0.",
              "Source archives and repositories for each component are identified below.",
              "This inventory also includes debug-only and proc-macro dependencies conservatively.", ""]
    for row in components:
        notice.extend(["=" * 78, row["name"], "License: " + row["declared_license"], "Source: " + row["source"]])
        if row.get("selected_license"):
            notice.append("Selected license: " + row["selected_license"])
        if row.get("upstream"):
            notice.append("Upstream: " + row["upstream"])
        if row.get("developers"):
            notice.append("Upstream authors/developers: " + "; ".join(row["developers"]))
        notice.extend(row.get("copyrights", []))
        for item in row["documents"]:
            notice.extend(["Document: " + item["title"], "Origin: " + item["source"], "Full text SHA-256: " + item["sha256"]])
        notice.append("")
    notice.extend(["=" * 78, "FULL LICENSE AND NOTICE TEXTS", "Shared identical texts appear once; SHA-256 references above identify them.", ""])
    for digest, content in sorted(DOCUMENTS.items()):
        notice.extend(["=" * 78, "SHA-256: " + digest, "", content])
    return {"THIRD_PARTY_NOTICES.txt": "\n".join(notice).rstrip() + "\n",
            "manifest.json": json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
            "PROJECT_LICENSE.txt": (ROOT / "LICENSE").read_text(encoding="utf-8")}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="Regenerate and fail if committed assets differ")
    args = parser.parse_args()
    outputs = generate()  # Finish collection before replacing any asset.
    if args.check:
        stale = [name for name, value in outputs.items() if not (ASSETS / name).is_file() or (ASSETS / name).read_text(encoding="utf-8") != value]
        if stale:
            raise SystemExit("License assets need regeneration: " + ", ".join(stale))
        print("All license assets match the resolved dependencies and source notices.")
    else:
        ASSETS.mkdir(parents=True, exist_ok=True)
        for name, value in outputs.items():
            (ASSETS / name).write_text(value, encoding="utf-8", newline="\n")
        print(f"Wrote {len(outputs)} APK license assets to {ASSETS}")


if __name__ == "__main__":
    main()
