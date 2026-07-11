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
saved_umask=$(umask)
cleanup_launch() {
    rm -f "$status" "$out" "$err" "$pid_file" "$ready" "$runner" "$command_file" "$child_meta" "$stop_file" "$error_file"
    rm -f "$root"/status.tmp.* "$root"/pid.tmp.* "$root"/ready.tmp.* "$root"/error.tmp.* "$root"/child.tmp.* 2>/dev/null || true
    rmdir "$root" 2>/dev/null || true
}
umask 077
if ! mkdir "$root" 2>/dev/null; then
    printf '%s\n' 'job directory already exists or cannot be created' >&2
    exit 1
fi
chmod 700 "$root" || { cleanup_launch; exit 1; }
: >"$out" && : >"$err" || { cleanup_launch; exit 1; }
printf '%s' @@RUNNER_SOURCE@@ >"$runner" || { cleanup_launch; exit 1; }
printf '%s' @@CHILD_SOURCE@@ >"$command_file" || { cleanup_launch; exit 1; }
chmod 600 "$out" "$err" "$runner" "$command_file" || { cleanup_launch; exit 1; }
umask "$saved_umask"
nohup sh "$runner" </dev/null >/dev/null 2>&1 &
monitor_launch_pid=$!
launch_wait=0
while [ ! -f "$ready" ] && [ ! -f "$status" ] && [ ! -f "$error_file" ] && [ "$launch_wait" -lt @@READY_ATTEMPTS@@ ]; do
    if ! kill -0 "$monitor_launch_pid" 2>/dev/null; then break; fi
    sleep 0.1
    launch_wait=$((launch_wait + 1))
done
if [ -f "$error_file" ]; then
    launch_error=$(head -c 4096 "$error_file" 2>/dev/null)
    kill -TERM "$monitor_launch_pid" 2>/dev/null || true
    sleep 0.1
    kill -KILL "$monitor_launch_pid" 2>/dev/null || true
    cleanup_launch
    printf '%s\n' "$launch_error" >&2
    exit 1
fi
if [ ! -f "$ready" ] && [ ! -f "$status" ]; then
    kill -TERM "$monitor_launch_pid" 2>/dev/null || true
    cleanup_wait=0
    while kill -0 "$monitor_launch_pid" 2>/dev/null && [ "$cleanup_wait" -lt @@GRACE_ATTEMPTS@@ ]; do
        sleep 0.1
        cleanup_wait=$((cleanup_wait + 1))
    done
    kill -KILL "$monitor_launch_pid" 2>/dev/null || true
    cleanup_launch
    printf '%s\n' 'job monitor failed to become ready' >&2
    exit 1
fi
printf '%s\n' "$status"
