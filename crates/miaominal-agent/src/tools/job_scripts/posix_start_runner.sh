#!/bin/sh
@@HELPERS@@
root=@@ROOT@@
status=@@STATUS@@
out=@@STDOUT@@
err=@@STDERR@@
pid_file=@@PID@@
ready=@@READY@@
runner=@@RUNNER@@
command_file=@@COMMAND_FILE@@
child_meta=@@CHILD@@
stop_file=@@STOP@@
error_file=@@ERROR@@
token=@@TOKEN@@
saved_umask=$(umask)
child_pid=
child_pgid=
fail_launch() {
    umask 077
    error_tmp="$error_file.tmp.$$"
    printf '%s\n' "$1" >"$error_tmp" && mv -f "$error_tmp" "$error_file"
    exit 1
}
cleanup_signal() {
    if [ -n "$child_pgid" ]; then terminate_group "$child_pgid" || true
    elif [ -n "$child_pid" ]; then terminate_process "$child_pid" || true
    fi
    exit 143
}
trap cleanup_signal TERM INT
monitor_pid=$$
monitor_uid=$(id -u 2>/dev/null) || fail_launch 'failed to determine monitor uid'
monitor_identity=$(process_identity "$monitor_pid") || fail_launch 'failed to capture monitor identity'
rm -f "$child_meta" "$ready" "$error_file"
umask "$saved_umask"
mode=
if command -v setsid >/dev/null 2>&1; then
    mode=setsid
    setsid sh "$command_file" >"$out" 2>"$err" &
    launch_pid=$!
else
    if ! set -m 2>/dev/null; then
        fail_launch 'setsid is unavailable and the shell cannot enable job control'
    fi
    mode=job_control
    sh "$command_file" >"$out" 2>"$err" &
    launch_pid=$!
fi
child_wait=0
while [ ! -f "$child_meta" ] && process_alive "$launch_pid" && [ "$child_wait" -lt @@READY_ATTEMPTS@@ ]; do
    sleep 0.1
    child_wait=$((child_wait + 1))
done
if [ ! -f "$child_meta" ]; then
    terminate_process "$launch_pid" || true
    fail_launch 'job child failed to publish process metadata'
fi
metadata_value() { sed -n "s/^$1=//p" "$child_meta" 2>/dev/null | head -n 1; }
child_pid=$(metadata_value pid)
child_uid=$(metadata_value uid)
child_pgid=$(metadata_value pgid)
child_identity=$(metadata_value identity)
for metadata_number in "$child_pid" "$child_uid" "$child_pgid"; do
    case "$metadata_number" in ''|*[!0-9]*)
        terminate_process "$launch_pid" || true
        fail_launch 'job child metadata was invalid'
        ;;
    esac
done
if [ "$child_pid" != "$launch_pid" ] || [ "$child_uid" != "$monitor_uid" ] || [ "$child_pgid" != "$child_pid" ]; then
    terminate_process "$launch_pid" || true
    fail_launch 'job child did not enter a verified private process group'
fi
actual_identity=$(process_identity "$child_pid") || { terminate_group "$child_pgid" || true; fail_launch 'job child disappeared before ready'; }
actual_pgid=$(process_pgid "$child_pid") || { terminate_group "$child_pgid" || true; fail_launch 'failed to verify job process group'; }
if [ "$actual_identity" != "$child_identity" ] || [ "$actual_pgid" != "$child_pgid" ]; then
    terminate_group "$child_pgid" || true
    fail_launch 'job child identity changed before ready'
fi
umask 077
pid_tmp="$pid_file.tmp.$$"
{
    printf 'version=1\n'
    printf 'token=%s\n' "$token"
    printf 'uid=%s\n' "$monitor_uid"
    printf 'mode=%s\n' "$mode"
    printf 'monitor_pid=%s\n' "$monitor_pid"
    printf 'monitor_identity=%s\n' "$monitor_identity"
    printf 'child_pid=%s\n' "$child_pid"
    printf 'child_identity=%s\n' "$child_identity"
    printf 'child_pgid=%s\n' "$child_pgid"
} >"$pid_tmp" && mv -f "$pid_tmp" "$pid_file"
ready_tmp="$ready.tmp.$$"
printf 'ready\n' >"$ready_tmp" && mv -f "$ready_tmp" "$ready"
umask "$saved_umask"
set +e
wait "$launch_pid"
exit_code=$?
set -e
if [ -f "$stop_file" ]; then exit 0; fi
if group_alive "$child_pgid" && ! terminate_group "$child_pgid"; then
    printf '%s\n' 'job process group survived natural command exit' >>"$err"
    exit 1
fi
umask 077
status_tmp="$status.tmp.$$"
printf '%s' "$exit_code" >"$status_tmp"
if ln "$status_tmp" "$status" 2>/dev/null; then :; fi
rm -f "$status_tmp" "$pid_file" "$ready" "$runner" "$command_file" "$child_meta" "$stop_file" "$error_file"
exit "$exit_code"
