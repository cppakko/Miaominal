@@HELPERS@@
root=@@ROOT@@
status=@@STATUS@@
out=@@STDOUT@@
err=@@STDERR@@
pid_file=@@PID@@
ready=@@READY@@
runner=@@RUNNER@@
command_file=@@COMMAND@@
child_meta=@@CHILD@@
stop_file=@@STOP@@
error_file=@@ERROR@@
expected_token=@@TOKEN@@
if [ -f "$status" ]; then printf 'already_finished\n'; exit 0; fi
if [ ! -d "$root" ] || [ -L "$root" ]; then printf 'not_found\n'; exit 0; fi
if [ ! -f "$pid_file" ]; then
    printf '%s\n' 'job process metadata is missing; refusing to report the job stopped' >&2
    exit 1
fi
metadata_value() { sed -n "s/^$1=//p" "$pid_file" 2>/dev/null | head -n 1; }
version=$(metadata_value version)
token=$(metadata_value token)
uid=$(metadata_value uid)
monitor_pid=$(metadata_value monitor_pid)
monitor_identity=$(metadata_value monitor_identity)
child_pid=$(metadata_value child_pid)
child_identity=$(metadata_value child_identity)
child_pgid=$(metadata_value child_pgid)
if [ "$version" != 1 ] || [ "$token" != "$expected_token" ] || [ "$uid" != "$(id -u 2>/dev/null)" ]; then
    printf '%s\n' 'job process metadata identity is invalid' >&2
    exit 1
fi
for value in "$monitor_pid" "$child_pid" "$child_pgid"; do
    case "$value" in ''|*[!0-9]*) printf '%s\n' 'job process metadata contains an invalid pid' >&2; exit 1 ;; esac
done
monitor_alive=0
verified_identity=0
if process_alive "$monitor_pid"; then
    actual_monitor_identity=$(process_identity "$monitor_pid" 2>/dev/null || true)
    monitor_command=$(ps -ww -p "$monitor_pid" -o command= 2>/dev/null || true)
    case "$monitor_command" in *"$runner"*) : ;; *) printf '%s\n' 'job monitor command identity mismatch' >&2; exit 1 ;; esac
    if [ "$actual_monitor_identity" != "$monitor_identity" ]; then
        printf '%s\n' 'job monitor start identity mismatch' >&2
        exit 1
    fi
    monitor_alive=1
    verified_identity=1
fi
if process_alive "$child_pid"; then
    actual_child_identity=$(process_identity "$child_pid" 2>/dev/null || true)
    actual_child_pgid=$(process_pgid "$child_pid" 2>/dev/null || true)
    if [ "$actual_child_identity" != "$child_identity" ] || [ "$actual_child_pgid" != "$child_pgid" ]; then
        printf '%s\n' 'job child process identity mismatch' >&2
        exit 1
    fi
    verified_identity=1
fi
if [ "$verified_identity" -ne 1 ]; then
    printf '%s\n' 'job monitor and child identities are no longer verifiable; refusing to signal the historical process group' >&2
    exit 1
fi
saved_umask=$(umask)
umask 077
stop_tmp="$stop_file.tmp.$$"
printf 'stop\n' >"$stop_tmp" && mv -f "$stop_tmp" "$stop_file"
umask "$saved_umask"
if group_alive "$child_pgid" && ! terminate_group "$child_pgid"; then
    printf '%s\n' 'failed to stop job process group' >&2
    exit 1
fi
if [ "$monitor_alive" -eq 1 ] && ! terminate_process "$monitor_pid"; then
    printf '%s\n' 'failed to stop job monitor process' >&2
    exit 1
fi
if group_alive "$child_pgid" || process_alive "$monitor_pid"; then
    printf '%s\n' 'job processes survived stop verification' >&2
    exit 1
fi
umask 077
status_tmp="$status.tmp.$$"
printf 'stopped' >"$status_tmp"
if ln "$status_tmp" "$status" 2>/dev/null; then
    rm -f "$status_tmp" "$out" "$err" "$pid_file" "$ready" "$runner" "$command_file" "$child_meta" "$stop_file" "$error_file"
    printf 'stopped\n'
else
    rm -f "$status_tmp" "$stop_file"
    printf 'already_finished\n'
fi
