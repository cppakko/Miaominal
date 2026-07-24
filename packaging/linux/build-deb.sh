#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "error: deb packages must be built on Linux." >&2
  exit 1
fi

build_release=true
if [[ "${1:-}" == "--no-build" ]]; then
  build_release=false
  shift
fi

if (( $# > 0 )); then
  echo "usage: $0 [--no-build]" >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: Cargo is required. Install Rust with rustup before continuing." >&2
  exit 1
fi

required_cargo_deb_version="3.7.0"
cargo_deb_version="$(cargo deb --version 2>/dev/null || true)"

if [[ "$cargo_deb_version" != *" $required_cargo_deb_version"* ]]; then
  echo "error: cargo-deb 3.7.0 is required." >&2
  if [[ -n "$cargo_deb_version" ]]; then
    echo "found: $cargo_deb_version" >&2
  fi
  echo "install it with: cargo install cargo-deb --version 3.7.0 --locked" >&2
  exit 1
fi

missing_debian_tools=()
for tool in dpkg-deb dpkg-shlibdeps; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    missing_debian_tools+=("$tool")
  fi
done

if (( ${#missing_debian_tools[@]} > 0 )); then
  echo "error: missing Debian packaging tools: ${missing_debian_tools[*]}" >&2
  echo "install them with: sudo apt-get install dpkg dpkg-dev" >&2
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

if [[ "$build_release" == true ]]; then
  cargo build --release --locked
fi
cargo deb --no-build --locked

shopt -s nullglob
packages=(target/debian/miaominal_*.deb)

if (( ${#packages[@]} == 0 )); then
  echo "error: cargo-deb completed without producing target/debian/miaominal_*.deb" >&2
  exit 1
fi

echo "Debian package created:"
printf '  %s\n' "${packages[@]}"
