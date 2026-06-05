#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 3 ]]; then
    echo "Usage: $0 <path-to-app-bundle> [path-to-icon-package] [app-icon-name]" >&2
    exit 1
fi

resolve_path() {
    local path="$1"

    if [[ -d "$path" ]]; then
        (
            cd "$path"
            pwd
        )
        return
    fi

    (
        cd "$(dirname "$path")"
        printf '%s/%s\n' "$(pwd)" "$(basename "$path")"
    )
}

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
app_bundle="$(resolve_path "$1")"
icon_package_input="${2:-$repo_root/assets/macos.icon}"
icon_package="$(resolve_path "$icon_package_input")"
app_icon_name="${3:-$(basename "$icon_package" .icon)}"
minimum_target="${MIAOMINAL_MACOS_ICON_MIN_TARGET:-26.0}"
info_plist="$app_bundle/Contents/Info.plist"
resources_dir="$app_bundle/Contents/Resources"

if [[ ! -d "$app_bundle" ]]; then
    echo "App bundle not found: $app_bundle" >&2
    exit 1
fi

if [[ ! -d "$icon_package" ]]; then
    echo "Icon Composer package not found: $icon_package" >&2
    exit 1
fi

if [[ ! -f "$info_plist" ]]; then
    echo "Info.plist not found: $info_plist" >&2
    exit 1
fi

if ! xcrun --find actool >/dev/null 2>&1; then
    echo "actool not found in the active Xcode toolchain" >&2
    exit 1
fi

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/${app_icon_name}-icon.XXXXXX")"
partial_plist="$tmp_dir/partial.plist"
assets_car="$tmp_dir/Assets.car"

cleanup() {
    rm -rf "$tmp_dir"
}

trap cleanup EXIT

xcrun actool \
    --compile "$tmp_dir" \
    --platform macosx \
    --minimum-deployment-target "$minimum_target" \
    --app-icon "$app_icon_name" \
    --output-partial-info-plist "$partial_plist" \
    "$icon_package" \
    >/dev/null

if [[ ! -f "$assets_car" ]]; then
    echo "actool did not produce Assets.car" >&2
    exit 1
fi

mkdir -p "$resources_dir"
cp "$assets_car" "$resources_dir/Assets.car"

if ! /usr/libexec/PlistBuddy -c "Set :CFBundleIconName $app_icon_name" "$info_plist" >/dev/null 2>&1; then
    /usr/libexec/PlistBuddy -c "Add :CFBundleIconName string $app_icon_name" "$info_plist" >/dev/null
fi

/usr/libexec/PlistBuddy -c "Delete :CFBundleIconFile" "$info_plist" >/dev/null 2>&1 || true

echo "Applied Icon Composer asset '$app_icon_name' to $app_bundle"
