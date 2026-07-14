Add-Type -AssemblyName Microsoft.VisualBasic

$info = [Microsoft.VisualBasic.Devices.ComputerInfo]::new()
$counters = Get-Counter -Counter @(
    '\Processor(_Total)\% Processor Time'
    '\Paging File(_Total)\% Usage'
    '\Network Interface(*)\Bytes Received/sec'
    '\Network Interface(*)\Bytes Sent/sec'
    '\System\Processor Queue Length'
    '\System\System Up Time'
) -ErrorAction SilentlyContinue
$samples = @($counters.CounterSamples)
$ErrorActionPreference = 'Stop'

function Get-CounterValue($pattern, $sum, $required) {
    $matched = @($samples | Where-Object { $_.Path -match $pattern })
    if ($matched.Count -eq 0) {
        if ($required) { throw "Missing performance counter: $pattern" }
        return [double]0
    }
    $value = if ($sum) {
        ($matched | Measure-Object -Property CookedValue -Sum).Sum
    } else {
        $matched[0].CookedValue
    }
    $value = [double]$value
    if ([double]::IsNaN($value) -or [double]::IsInfinity($value)) {
        throw "Invalid performance counter: $pattern"
    }
    $value
}

$cpu = Get-CounterValue '\\processor\(_total\)\\% processor time$' $false $true
$swap = Get-CounterValue '\\paging file\(_total\)\\% usage$' $false $false
$load = Get-CounterValue '\\system\\processor queue length$' $false $true
$uptime = Get-CounterValue '\\system\\system up time$' $false $true
$rx = Get-CounterValue '\\bytes received/sec$' $true $true
$tx = Get-CounterValue '\\bytes sent/sec$' $true $true

$memTotal = [double]$info.TotalPhysicalMemory
$memUsed = [Math]::Max(
    [double]0,
    $memTotal - [double]$info.AvailablePhysicalMemory
)
$mem = if ($memTotal -gt 0) { ($memUsed / $memTotal) * 100 } else { 0 }

$diskTotal = $null
$diskUsed = $null
try {
    $drive = [System.IO.DriveInfo]::new($env:SystemDrive)
    $diskTotal = [double]$drive.TotalSize
    $diskUsed = [Math]::Max(
        [double]0,
        $diskTotal - [double]$drive.AvailableFreeSpace
    )
} catch {}
$disk = if ($diskTotal -and $diskTotal -gt 0) {
    ($diskUsed / $diskTotal) * 100
} else {
    0
}

[pscustomobject]@{
    hostname   = [Environment]::MachineName
    cores      = [Environment]::ProcessorCount
    uptime     = [int64]$uptime
    cpu        = $cpu
    mem        = $mem
    mem_used   = $memUsed
    mem_total  = $memTotal
    swap       = $swap
    swap_used  = 0
    swap_total = 0
    disk       = $disk
    disk_used  = $diskUsed
    disk_total = $diskTotal
    rx         = $rx
    tx         = $tx
    load       = $load
} | ConvertTo-Json -Compress
