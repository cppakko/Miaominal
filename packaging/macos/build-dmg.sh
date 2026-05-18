#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
    echo "Usage: $0 <path-to-app-bundle> <output-dmg> <volume-name>" >&2
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

detach_device() {
    local device="$1"

    if [[ -z "$device" ]]; then
        return
    fi

    for _ in 1 2 3; do
        if hdiutil detach "$device" >/dev/null 2>&1; then
            return
        fi
        sleep 1
    done

    hdiutil detach -force "$device" >/dev/null 2>&1 || true
}

app_bundle="$(resolve_path "$1")"
output_dmg="$(resolve_path "$2")"
volume_name="$3"
app_bundle_name="$(basename "$app_bundle")"
app_name="${app_bundle_name%.app}"
output_dir="$(dirname "$output_dmg")"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/${app_name}-dmg.XXXXXX")"
staging_dir="$work_dir/staging"
readwrite_dmg="$work_dir/${app_name}.dmg"
compressed_dmg="$work_dir/${app_name}-compressed.dmg"
mount_path="/Volumes/${volume_name}"
device=""

cleanup() {
    detach_device "$device"
    rm -rf "$work_dir"
}

trap cleanup EXIT

if [[ ! -d "$app_bundle" ]]; then
    echo "App bundle not found: $app_bundle" >&2
    exit 1
fi

mkdir -p "$staging_dir" "$output_dir"
cp -R "$app_bundle" "$staging_dir/"
ln -s /Applications "$staging_dir/Applications"

staging_size_kb="$(du -sk "$staging_dir" | awk '{print $1}')"
dmg_size_mb=$(( (staging_size_kb / 1024) + 64 ))

hdiutil create \
    -volname "$volume_name" \
    -srcfolder "$staging_dir" \
    -fs "HFS+" \
    -format UDRW \
    -size "${dmg_size_mb}m" \
    "$readwrite_dmg" \
    >/dev/null

attach_output="$(hdiutil attach -readwrite -noverify -noautoopen "$readwrite_dmg")"
device="$(printf '%s\n' "$attach_output" | awk 'NR == 1 { print $1; exit }')"

if [[ -z "$device" ]]; then
    echo "Failed to determine mounted device for $readwrite_dmg" >&2
    exit 1
fi

osascript <<EOF
tell application "Finder"
    tell disk "${volume_name}"
        open
        set current view of container window to icon view
        set toolbar visible of container window to false
        set statusbar visible of container window to false
        set bounds of container window to {120, 120, 700, 440}
        set view_options to the icon view options of container window
        set arrangement of view_options to not arranged
        set icon size of view_options to 128
        set text size of view_options to 14
        set position of item "${app_bundle_name}" of container window to {180, 190}
        set position of item "Applications" of container window to {500, 190}
        close
        open
        update without registering applications
        delay 2
    end tell
end tell
EOF

bless --folder "$mount_path" --openfolder "$mount_path" >/dev/null 2>&1 || true

detach_device "$device"
device=""
sync
sleep 2

hdiutil convert \
    "$readwrite_dmg" \
    -format UDZO \
    -imagekey zlib-level=9 \
    -ov \
    -o "$compressed_dmg" \
    >/dev/null

mv "$compressed_dmg" "$output_dmg"