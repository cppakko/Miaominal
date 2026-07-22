export LC_ALL=C
export LANG=C

hostname 2>/dev/null | awk 'NR == 1 { printf("host %s\n", $0) }'

cores=$(sysctl -n hw.logicalcpu 2>/dev/null)
boot=$(sysctl -n kern.boottime 2>/dev/null | awk -F'[=,]' '{ gsub(/ /, "", $2); print $2 }')
now=$(date +%s)
uptime=0
if test -n "$boot" && test "$now" -ge "$boot" 2>/dev/null; then
    uptime=$((now - boot))
fi
printf 'meta %s %s\n' "${cores:-0}" "$uptime"

sysctl -n hw.memsize 2>/dev/null | awk 'NR == 1 { printf("memtotal %s\n", $1) }'

cpu_before=$(sysctl -n kern.cp_time 2>/dev/null)
sleep 1
cpu_after=$(sysctl -n kern.cp_time 2>/dev/null)
if test -n "$cpu_before" && test -n "$cpu_after"; then
    printf 'cputicks %s %s\n' "$cpu_before" "$cpu_after"
fi

top -l 1 -n 0 | awk '
    /CPU usage/ { printf("topline %s\n", $0) }
    /PhysMem:/ {
        printf("physmem %s\n", $0)
        exit
    }
'

internal_pages=$(sysctl -n vm.page_pageable_internal_count 2>/dev/null)
vm_stat 2>/dev/null | awk -v internal_pages="$internal_pages" '
    function counter(value) {
        gsub(/\./, "", value)
        return value
    }
    NR == 1 {
        page_size = $0
        sub(/^.*page size of /, "", page_size)
        sub(/ bytes.*$/, "", page_size)
    }
    /Pages purgeable:/ { purgeable_pages = counter($NF) }
    /Pages wired down:/ { wired_pages = counter($NF) }
    /Pages occupied by compressor:/ { compressor_pages = counter($NF) }
    END {
        if (page_size ~ /^[0-9]+$/ &&
            internal_pages ~ /^[0-9]+$/ &&
            purgeable_pages ~ /^[0-9]+$/ &&
            wired_pages ~ /^[0-9]+$/ &&
            compressor_pages ~ /^[0-9]+$/) {
            printf("memstats %s %s %s %s %s\n",
                page_size,
                internal_pages,
                purgeable_pages,
                wired_pages,
                compressor_pages)
        }
    }
'

sysctl -n vm.swapusage 2>/dev/null | awk 'NR == 1 { printf("swapraw %s\n", $0) }'

netstat -ibn | awk '
    BEGIN { rx = 0; tx = 0 }
    NR > 1 && $1 !~ /^lo/ && $7 ~ /^[0-9]+$/ && $10 ~ /^[0-9]+$/ {
        rx += $7
        tx += $10
    }
    END { printf("net %.0f %.0f\n", rx, tx) }
'

sysctl -n vm.loadavg | awk '{
    gsub(/[{}]/, "")
    printf("load %s\n", $1)
}'

df -Pk / 2>/dev/null | awk 'NR == 2 {
    gsub(/%/, "", $5)
    printf("disk %s %s %s\n", $2, $3, $5)
}'
