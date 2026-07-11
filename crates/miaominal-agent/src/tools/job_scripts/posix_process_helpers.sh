process_identity() {
    identity_pid=$1
    case "$identity_pid" in ''|*[!0-9]*) return 1 ;; esac
    if [ -r "/proc/$identity_pid/stat" ]; then
        identity_start=$(sed 's/^[0-9][0-9]* (.*) //' "/proc/$identity_pid/stat" 2>/dev/null | awk '{print $20}') || return 1
        [ -n "$identity_start" ] || return 1
        printf 'proc:%s\n' "$identity_start"
    else
        identity_start=$(ps -p "$identity_pid" -o lstart= 2>/dev/null | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')
        [ -n "$identity_start" ] || return 1
        printf 'ps:%s\n' "$identity_start"
    fi
}
process_pgid() {
    process_pgid_value=$(ps -p "$1" -o pgid= 2>/dev/null | tr -d '[:space:]')
    case "$process_pgid_value" in ''|*[!0-9]*) return 1 ;; esac
    printf '%s\n' "$process_pgid_value"
}
process_uid() {
    process_uid_value=$(ps -p "$1" -o uid= 2>/dev/null | tr -d '[:space:]')
    case "$process_uid_value" in ''|*[!0-9]*) return 1 ;; esac
    printf '%s\n' "$process_uid_value"
}
process_alive() {
    process_alive_pid=$1
    case "$process_alive_pid" in ''|*[!0-9]*) return 1 ;; esac
    kill -0 "$process_alive_pid" 2>/dev/null && return 0
    [ -d "/proc/$process_alive_pid" ] && return 0
    process_alive_value=$(ps -p "$process_alive_pid" -o pid= 2>/dev/null | tr -d '[:space:]')
    [ "$process_alive_value" = "$process_alive_pid" ]
}
group_alive() {
    group_alive_pgid=$1
    case "$group_alive_pgid" in ''|*[!0-9]*) return 1 ;; esac
    kill -0 -- "-$group_alive_pgid" 2>/dev/null && return 0
    ps -e -o pgid= 2>/dev/null | grep -q "^[[:space:]]*$group_alive_pgid[[:space:]]*$"
}
terminate_process() {
    terminate_pid=$1
    process_alive "$terminate_pid" || return 0
    kill -TERM "$terminate_pid" 2>/dev/null || true
    terminate_i=0
    while process_alive "$terminate_pid" && [ "$terminate_i" -lt 20 ]; do
        sleep 0.1
        terminate_i=$((terminate_i + 1))
    done
    if process_alive "$terminate_pid"; then
        kill -KILL "$terminate_pid" 2>/dev/null || true
        terminate_i=0
        while process_alive "$terminate_pid" && [ "$terminate_i" -lt 50 ]; do
            sleep 0.1
            terminate_i=$((terminate_i + 1))
        done
    fi
    ! process_alive "$terminate_pid"
}
terminate_group() {
    terminate_pgid=$1
    group_alive "$terminate_pgid" || return 0
    kill -TERM -- "-$terminate_pgid" 2>/dev/null || true
    terminate_i=0
    while group_alive "$terminate_pgid" && [ "$terminate_i" -lt 20 ]; do
        sleep 0.1
        terminate_i=$((terminate_i + 1))
    done
    if group_alive "$terminate_pgid"; then
        kill -KILL -- "-$terminate_pgid" 2>/dev/null || true
        terminate_i=0
        while group_alive "$terminate_pgid" && [ "$terminate_i" -lt 50 ]; do
            sleep 0.1
            terminate_i=$((terminate_i + 1))
        done
    fi
    ! group_alive "$terminate_pgid"
}
