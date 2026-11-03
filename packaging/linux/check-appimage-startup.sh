#!/usr/bin/env bash
# Run in release CI's minimal Linux container, without GTK or a display server.
# Reaching the graphical-session guard proves the real application has loaded
# and initialized its data directory; no GUI or FUSE mount is needed.
set -euo pipefail

image=$(realpath -- "${1:?usage: check-appimage-startup.sh IMAGE}")
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

unset DISPLAY WAYLAND_DISPLAY LD_LIBRARY_PATH LD_PRELOAD LD_AUDIT LD_TRACE_LOADED_OBJECTS
unset APPIMAGE APPDIR MIAOMINAL_APPIMAGE_CHECK_RUNTIME
export HOME="$work/home"
export XDG_CONFIG_HOME="$HOME/.config" XDG_CACHE_HOME="$HOME/.cache"
export XDG_DATA_HOME="$HOME/.local/share" XDG_RUNTIME_DIR="$work/runtime"
export LD_BIND_NOW=1
mkdir -p "$HOME" "$XDG_RUNTIME_DIR"
chmod 0700 "$XDG_RUNTIME_DIR"

check_startup() {
    local mode=$1
    shift
    local root="$work/$mode"
    mkdir -p "$root"
    cp -- "$image" "$root/Miaominal.AppImage"
    chmod 0755 "$root/Miaominal.AppImage"
    if [ "$mode" = flag ]; then
        : > "$root/portable.flag"
    fi
    local status=0
    timeout 60 "$root/Miaominal.AppImage" --appimage-extract-and-run "$@" \
        > "$root/startup.log" 2>&1 || status=$?
    cat -- "$root/startup.log"
    if [ "$status" -ne 1 ]; then
        echo "Expected the headless application exit (1), got $status" >&2
        return 1
    fi
    grep -Fq 'Miaominal requires a graphical Linux desktop session.' "$root/startup.log"
    # Extraction is writable too: checking this exact path catches accidentally
    # placing portable data in the temporary AppDir instead of beside the image.
    test -d "$root/data"
    echo "AppImage startup and portable $mode passed"
}

check_startup argument --portable
check_startup flag
