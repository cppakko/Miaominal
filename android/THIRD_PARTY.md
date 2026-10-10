# Android third-party licenses

The application packages the following files through Android's standard assets
directory. They are included in debug and release APKs without requiring Python
or network access during an ordinary build:

- [`THIRD_PARTY_NOTICES.txt`](app/src/main/assets/licenses/THIRD_PARTY_NOTICES.txt):
  dependency versions, upstream sources, copyright notices, full license texts,
  and applicable upstream NOTICE files. Identical texts are included once and
  referenced by SHA-256.
- [`manifest.json`](app/src/main/assets/licenses/manifest.json): machine-readable
  component inventory and mappings to the full texts.
- [`PROJECT_LICENSE.txt`](app/src/main/assets/licenses/PROJECT_LICENSE.txt):
  Miaominal's GPLv3 license.

Inside an APK these files are located under `assets/licenses/`.

**Settings > About > Licenses** provides entries for the project license and
third-party notices. Both open an offline native text reader inside the app.
Documents are read from assets on an IO dispatcher and rendered in bounded text
blocks using a lazy list, so scrolling does not lay out the entire notice bundle.
The Material 3 reading screen uses readable body typography, a width-limited
layout on larger screens, and a scrolling top app bar. Text supports selection
and copying; the close button or system back gesture returns to About. Loading
and retry states remain dismissible. Each license entry is a clickable Material
card, and these entries are also available in settings search.

## Scope and license choices

The inventory covers the resolved release and debug application runtime graphs,
the normal Cargo dependency graph for `aarch64-linux-android` and
`x86_64-linux-android`, embedded native components, and the UniFFI templates used
to generate Kotlin helper code. Including debug-only and proc-macro components
is conservative; not every listed component is linked into a release APK.

JNA and its native dispatch code are used under their Apache 2.0 option. The
embedded libffi code retains its MIT notice. UniFFI, its generated helper code,
and option-ext retain their applicable MPL 2.0 terms and source availability.
Other dual-license declarations are recorded as published; retaining multiple
license texts does not turn an `OR` expression into an `AND` expression.

Test APKs and test-only dependencies are outside this application notice bundle.
In particular, JUnit 4's EPL 1.0 declaration is not a production dependency and
is not replaced by the application's GPLv3 license.

## Updating the inventory

Run from the repository root after changing Android dependencies, Cargo
dependencies, or the project license:

```sh
python android/scripts/generate_third_party_notices.py
python android/scripts/generate_third_party_notices.py --check
```

Use a JDK supported by the Gradle wrapper (set `JAVA_HOME` when necessary),
Python 3.10 or newer, and the Rust toolchain. Resolve/build the Android and Cargo
dependencies first: dependency graph resolution is offline and locked, while
missing published POMs, source notices, and versioned upstream license files are
downloaded when generating the inventory. Downloads are cached in the ignored
`android/build/third-party-license-cache/` directory. Commit all three generated
assets together with dependency changes.

The generator reads licenses and NOTICE files from the resolved Maven artifacts
and source JARs, and from downloaded Cargo packages. For crates whose license
files are outside their published crate directory, it uses the upstream Git
revision recorded in `.cargo_vcs_info.json`. The embedded JVM support crate for
rustls uses the matching parent crate's project license. Unexpected Maven license
declarations and missing native license documents stop generation for review.

## Distribution

Keep these assets in distributed APKs. An independently distributed APK should
be accompanied by the corresponding source and required build material under
GPLv3, preserving third-party licenses and MPL-covered source. The source links
in the notice bundle identify upstream versions; they do not replace the
project's obligation to provide its own corresponding release source.
