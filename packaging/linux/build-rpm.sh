#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "error: RPM packages must be built on Linux." >&2
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

required_generate_rpm_version="0.21.0"
generate_rpm_version="$(cargo generate-rpm --version 2>/dev/null || true)"

if [[ "$generate_rpm_version" != *" $required_generate_rpm_version"* ]]; then
  echo "error: cargo-generate-rpm 0.21.0 is required." >&2
  if [[ -n "$generate_rpm_version" ]]; then
    echo "found: $generate_rpm_version" >&2
  fi
  echo "install it with: cargo install cargo-generate-rpm --version 0.21.0 --locked" >&2
  exit 1
fi

if ! command -v ldd >/dev/null 2>&1; then
  echo "error: ldd is required for automatic RPM dependency detection." >&2
  echo "install it with: sudo apt-get install libc-bin" >&2
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

if [[ "$build_release" == true ]]; then
  cargo build --release --locked
fi
cargo generate-rpm --auto-req builtin

shopt -s nullglob
packages=(target/generate-rpm/miaominal-*.rpm)

if (( ${#packages[@]} == 0 )); then
  echo "error: cargo-generate-rpm completed without producing target/generate-rpm/miaominal-*.rpm" >&2
  exit 1
fi

echo "RPM package created:"
printf '  %s\n' "${packages[@]}"

if command -v rpm >/dev/null 2>&1; then
  echo
  echo "RPM package metadata:"
  rpm -qip "${packages[0]}"
fi
