# Desktop third-party licenses

The desktop application's **Settings > About > Licenses** section opens the
project's GPLv3 license and the complete third-party notice bundle in the default
text viewer. Both documents are embedded in the executable. Standalone EXEs,
portable builds, and installed applications can view them offline, without
requiring adjacent license files. Viewing a document writes a copy to the active
application data directory's `licenses` subdirectory.

`THIRD_PARTY_NOTICES.txt` contains component versions, source archive links,
upstream author and copyright notices, full license texts, and upstream NOTICE
files. Identical texts are stored once and referenced by SHA-256. `manifest.json`
records the same inventory for maintenance; it is not required at runtime.

## Inventory scope

The generator resolves the normal Cargo dependency graph for `miaominal` on
x86_64 and aarch64 Windows/MSVC, Linux/GNU, and macOS. It conservatively includes
proc-macros, plus bundled Lucide/Feather icons and SQLite's public-domain
dedication. Test-only dependencies and build tools are excluded. Components
retain their own licenses; Miaominal's GPLv3 does not replace those terms.

`self_cell` is used under Apache 2.0. Where an older crate lacks license files,
the inventory explicitly identifies the chosen standard text and preserves the
license declaration from the published source archive. These reviewed exceptions
are restricted to specific crate versions in the generator. Their original
author metadata and source copyright notices are retained, and no copyright
years are invented. For MIT's standard text, SPDX's placeholder copyright line
is omitted because it is a template rather than an upstream notice.

The inventory covers application dependencies and assets. Libraries added by
distribution tooling, such as shared libraries copied into a Linux AppImage,
must also retain their notices and any applicable source obligations when
packaging that release.

## Updating

Run from the repository root after changing dependencies or bundled assets:

```sh
python scripts/generate_desktop_third_party_notices.py
python scripts/generate_desktop_third_party_notices.py --check
```

Use Python 3.10 or newer, Git, and the Rust toolchain. Fetch locked Cargo
dependencies for all inventory targets first. Graph resolution is locked and
offline. Generation reads local package notices and retrieves missing documents
from pinned upstream Git revisions, with explicit handling of renamed
repositories. Cache data stays under the ignored
`target/third-party-license-cache` directory. Ordinary application builds use the
committed documents and require no license-generation network access. Commit
both generated files along with dependency changes.

The desktop script reuses the source-notice collectors in
`android/scripts/generate_third_party_notices.py`; it does not run Gradle or need
an Android SDK. Android's separate APK notice assets are documented in
`android/THIRD_PARTY.md`.

## Release source

Distribute the corresponding source and required build material for each release
under GPLv3, preserving third-party notices and MPL-covered source. Upstream
source links in this inventory identify dependency versions; they do not replace
the obligation to provide Miaominal's corresponding release source.
