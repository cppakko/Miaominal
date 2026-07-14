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

top -l 1 -n 0 | awk -F'[:,% ]+' '
    /CPU usage/ { printf("cpu %.2f\n", 100 - $(NF - 1)) }
    /PhysMem:/ { printf("physmem %s\n", $0); exit }
'

sysctl -n vm.swapusage 2>/dev/null | awk 'NR == 1 { printf("swapraw %s\n", $0) }'

netstat -ibn | awk '
    NR > 1 && $1 !~ /^lo/ && $7 ~ /^[0-9]+$/ && $10 ~ /^[0-9]+$/ {
        rx += $7
        tx += $10
    }
    END { printf("net %s %s\n", rx, tx) }
'

sysctl -n vm.loadavg | awk '{
    gsub(/[{}]/, "")
    printf("load %s\n", $1)
}'

df -Pk / 2>/dev/null | awk 'NR == 2 {
    gsub(/%/, "", $5)
    printf("disk %s %s %s\n", $2, $3, $5)
}'
