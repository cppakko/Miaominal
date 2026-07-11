@@HELPERS@@
current_uid=$(id -u 2>/dev/null) || exit 0
metadata_value() { sed -n "s/^$1=//p" "$2" 2>/dev/null | head -n 1; }
cleanup_root() {
    cleanup_root_path=$1
    rm -f "$cleanup_root_path/status" "$cleanup_root_path/stdout" "$cleanup_root_path/stderr" \
        "$cleanup_root_path/pid" "$cleanup_root_path/ready" "$cleanup_root_path/runner" \
        "$cleanup_root_path/command" "$cleanup_root_path/child" "$cleanup_root_path/stop" "$cleanup_root_path/error"
    rm -f "$cleanup_root_path"/status.tmp.* "$cleanup_root_path"/pid.tmp.* \
        "$cleanup_root_path"/ready.tmp.* "$cleanup_root_path"/error.tmp.* "$cleanup_root_path"/child.tmp.* 2>/dev/null || true
    rmdir "$cleanup_root_path" 2>/dev/null
}
for root in /tmp/miaominal-agent-*; do
    [ -d "$root" ] || continue
    [ ! -L "$root" ] || continue
    name=${root##*/}
    id=${name#miaominal-agent-}
    case "$id" in ????????-????-????-????-????????????) ;; *) continue ;; esac
    case "$id" in *[!0-9a-fA-F-]*) continue ;; esac
    owner=$(ls -dn "$root" 2>/dev/null | awk '{print $3}')
    [ "$owner" = "$current_uid" ] || continue
    old=$(find "$root" -prune -mmin +@@MINUTES@@ -print 2>/dev/null)
    [ -n "$old" ] || continue
    pid_file="$root/pid"
    if [ ! -f "$root/status" ] && [ -f "$pid_file" ]; then
        monitor_pid=$(metadata_value monitor_pid "$pid_file")
        monitor_identity=$(metadata_value monitor_identity "$pid_file")
        child_pgid=$(metadata_value child_pgid "$pid_file")
        live=0
        case "$monitor_pid" in ''|*[!0-9]*) : ;; *)
            if process_alive "$monitor_pid" && [ "$(process_identity "$monitor_pid" 2>/dev/null || true)" = "$monitor_identity" ]; then live=1; fi
            ;;
        esac
        case "$child_pgid" in ''|*[!0-9]*) : ;; *) if group_alive "$child_pgid"; then live=1; fi ;; esac
        [ "$live" -eq 0 ] || continue
    fi
    if cleanup_root "$root"; then printf 'cleaned=%s\n' "$id"; fi
done
for marker in /tmp/miaominal-agent-*.status; do
    [ -f "$marker" ] || continue
    [ ! -L "$marker" ] || continue
    name=${marker##*/}
    id=${name#miaominal-agent-}
    id=${id%.status}
    case "$id" in ????????-????-????-????-????????????) ;; *) continue ;; esac
    case "$id" in *[!0-9a-fA-F-]*) continue ;; esac
    owner=$(ls -ln "$marker" 2>/dev/null | awk '{print $3}')
    [ "$owner" = "$current_uid" ] || continue
    old=$(find "$marker" -prune -mmin +@@MINUTES@@ -print 2>/dev/null)
    [ -n "$old" ] || continue
    rm -f "$marker" "$marker.out" "$marker.err" "$marker.pid" "$marker.ctl.out" "$marker.ctl.err" "$marker.runner.ps1" "$marker".tmp-*
    printf 'cleaned=%s\n' "$id"
done
