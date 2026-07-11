@@HELPERS@@
root=@@ROOT@@
status=@@STATUS@@
out=@@STDOUT@@
err=@@STDERR@@
pid_file=@@PID@@
runner=@@RUNNER@@
expected_token=@@TOKEN@@
diagnostic=
emit_streams() {
    stdout_bytes=0
    stderr_bytes=0
    truncated=0
    if [ -f "$out" ]; then stdout_bytes=$(wc -c <"$out" 2>/dev/null || printf 0); fi
    if [ -f "$err" ]; then stderr_bytes=$(wc -c <"$err" 2>/dev/null || printf 0); fi
    if [ "$stdout_bytes" -gt @@MAX@@ ] 2>/dev/null || [ "$stderr_bytes" -gt @@MAX@@ ] 2>/dev/null; then truncated=1; fi
    printf 'truncated=%s\nstdout_b64=' "$truncated"
    if [ -f "$out" ]; then tail -c @@MAX@@ "$out" 2>/dev/null | base64 | tr -d '\r\n'; fi
    printf '\nstderr_b64='
    if [ -f "$err" ]; then tail -c @@MAX@@ "$err" 2>/dev/null | base64 | tr -d '\r\n'; fi
    printf '\n'
    if [ -n "$diagnostic" ]; then
        printf 'diagnostic_b64='
        printf '%s' "$diagnostic" | base64 | tr -d '\r\n'
        printf '\n'
    fi
}
metadata_value() { sed -n "s/^$1=//p" "$pid_file" 2>/dev/null | head -n 1; }
if [ -f "$status" ]; then
    exit_status=$(head -c 32 "$status" 2>/dev/null)
    if [ "$exit_status" = stopped ]; then printf 'status=stopped\n'
    elif case "$exit_status" in ''|*[!0-9-]*) false ;; *) true ;; esac; then
        printf 'status=exited\nexit=%s\n' "$exit_status"
    else
        printf 'status=exited\n'
        diagnostic='job status file was invalid'
    fi
    emit_streams
elif [ -f "$pid_file" ]; then
    version=$(metadata_value version)
    token=$(metadata_value token)
    uid=$(metadata_value uid)
    monitor_pid=$(metadata_value monitor_pid)
    monitor_identity=$(metadata_value monitor_identity)
    child_pid=$(metadata_value child_pid)
    child_identity=$(metadata_value child_identity)
    child_pgid=$(metadata_value child_pgid)
    metadata_valid=1
    [ "$version" = 1 ] || metadata_valid=0
    [ "$token" = "$expected_token" ] || metadata_valid=0
    [ "$uid" = "$(id -u 2>/dev/null)" ] || metadata_valid=0
    for value in "$monitor_pid" "$child_pid" "$child_pgid"; do case "$value" in ''|*[!0-9]*) metadata_valid=0 ;; esac; done
    if [ "$metadata_valid" -ne 1 ]; then
        printf 'status=running\n'
        diagnostic='job process metadata is invalid; refusing to assume the job exited'
        emit_streams
        exit 0
    fi
    monitor_alive=0
    group_is_alive=0
    identity_mismatch=0
    if process_alive "$monitor_pid"; then
        actual_monitor_identity=$(process_identity "$monitor_pid" 2>/dev/null || true)
        monitor_command=$(ps -ww -p "$monitor_pid" -o command= 2>/dev/null || true)
        case "$monitor_command" in *"$runner"*) : ;; *) identity_mismatch=1 ;; esac
        if [ "$actual_monitor_identity" = "$monitor_identity" ]; then monitor_alive=1; else identity_mismatch=1; fi
    fi
    if group_alive "$child_pgid"; then group_is_alive=1; fi
    if process_alive "$child_pid"; then
        actual_child_identity=$(process_identity "$child_pid" 2>/dev/null || true)
        actual_child_pgid=$(process_pgid "$child_pid" 2>/dev/null || true)
        if [ "$actual_child_identity" != "$child_identity" ] || [ "$actual_child_pgid" != "$child_pgid" ]; then identity_mismatch=1; fi
    fi
    if [ "$identity_mismatch" -eq 1 ]; then
        printf 'status=running\n'
        diagnostic='job process identity could not be verified; refusing to assume the job exited'
    elif [ "$monitor_alive" -eq 1 ] || [ "$group_is_alive" -eq 1 ]; then
        printf 'status=running\n'
    else
        printf 'status=exited\n'
        diagnostic='job processes disappeared before writing an exit status'
    fi
    emit_streams
elif [ -f "$out" ] || [ -f "$err" ]; then
    printf 'status=exited\n'
    diagnostic='job process metadata was missing'
    emit_streams
else
    printf 'status=not_found\n'
fi
