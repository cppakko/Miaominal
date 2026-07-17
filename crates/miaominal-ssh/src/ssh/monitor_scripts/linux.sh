hostname 2>/dev/null | awk 'NR == 1 { printf("host %s\n", $0) }'

cores=$(getconf _NPROCESSORS_ONLN 2>/dev/null)
uptime=$(awk '{ printf("%.0f", $1) }' /proc/uptime 2>/dev/null)
printf 'meta %s %s\n' "${cores:-0}" "${uptime:-0}"

awk 'NR == 1 {
    printf("cpu %s %s %s %s %s %s %s %s\n", $2, $3, $4, $5, $6, $7, $8, $9)
}' /proc/stat

awk '
    /MemTotal:/ { total = $2 }
    /MemAvailable:/ { available = $2 }
    /SwapTotal:/ { swap_total = $2 }
    /SwapFree:/ { swap_free = $2 }
    END {
        printf("mem %s %s %s %s\n", total, available, swap_total, swap_free)
    }
' /proc/meminfo

awk 'BEGIN { rx = 0; tx = 0 }
    NR > 2 && $1 !~ /^lo:/ { rx += $2; tx += $10 }
    END { printf("net %.0f %.0f\n", rx, tx) }
' /proc/net/dev

awk '{ printf("load %s\n", $1) }' /proc/loadavg

df -Pk / 2>/dev/null | awk 'NR == 2 {
    gsub(/%/, "", $5)
    printf("disk %s %s %s\n", $2, $3, $5)
}'
