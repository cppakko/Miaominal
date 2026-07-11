$marker=[Environment]::ExpandEnvironmentVariables(@@MARKER@@)
$out=$marker+'.out'; $err=$marker+'.err'; $pidFile=$marker+'.pid'
$ctlOut=$marker+'.ctl.out'; $ctlErr=$marker+'.ctl.err'; $runner=$marker+'.runner.ps1'
function Read-MiaominalTail([string]$path,[int]$limit) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return [pscustomobject]@{Bytes=[byte[]]::new(0);Count=0;Text='';Truncated=$false} }
    $stream=$null
    try {
        $share=[IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
        $stream=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,$share)
        $length=$stream.Length; $count=[int][Math]::Min([int64]$limit,$length)
        $bytes=[byte[]]::new($count); $total=0
        if ($count -gt 0) { $stream.Seek($length-$count,[IO.SeekOrigin]::Begin) *> $null }
        while ($total -lt $count) { $read=$stream.Read($bytes,$total,$count-$total); if ($read -le 0) { break }; $total+=$read }
        $text=(New-Object Text.UTF8Encoding($false,$false)).GetString($bytes,0,$total)
        return [pscustomobject]@{Bytes=$bytes;Count=$total;Text=$text;Truncated=($length -gt $limit)}
    } catch { return [pscustomobject]@{Bytes=[byte[]]::new(0);Count=0;Text='';Truncated=$false} }
    finally { if ($null -ne $stream) { $stream.Dispose() } }
}
function Get-MiaominalProcessState {
    if (-not (Test-Path -LiteralPath $pidFile -PathType Leaf)) { return [pscustomobject]@{Alive=$false;Diagnostic='job pid metadata was missing'} }
    try {
        $metadata=(Read-MiaominalTail $pidFile 4096).Text | ConvertFrom-Json
        $process=Get-Process -Id ([int]$metadata.pid) -ErrorAction Stop
        $actualTicks=$process.StartTime.ToUniversalTime().Ticks; $expectedTicks=[int64]$metadata.start_ticks
        if ($actualTicks -eq $expectedTicks) { return [pscustomobject]@{Alive=$true;Diagnostic=''} }
        return [pscustomobject]@{Alive=$false;Diagnostic='job pid identity mismatch'}
    } catch { return [pscustomobject]@{Alive=$false;Diagnostic=('job process lookup failed: '+($_.Exception.Message))} }
}
$diagnostic=''; $hasOutput=$false; $processState=Get-MiaominalProcessState
if (Test-Path -LiteralPath $marker -PathType Leaf) {
    $statusResult=Read-MiaominalTail $marker 64
    for ($i=0; $i -lt 20 -and -not $statusResult.Text; $i++) { Start-Sleep -Milliseconds 10; $statusResult=Read-MiaominalTail $marker 64 }
    $status=$statusResult.Text.Trim()
    if ($status -eq 'stopped') { Write-Output 'status=stopped' }
    elseif ($status -match '^-?[0-9]+$') { Write-Output 'status=exited'; Write-Output ('exit='+$status) }
    else { Write-Output 'status=exited'; $statusBytes=(New-Object Text.UTF8Encoding($false)).GetBytes($status); $diagnostic=('job status file was invalid: '+[Convert]::ToBase64String($statusBytes)) }
    $hasOutput=$true
} elseif ($processState.Alive) { Write-Output 'status=running'; $hasOutput=$true
} elseif ((Test-Path -LiteralPath $out) -or (Test-Path -LiteralPath $err) -or (Test-Path -LiteralPath $pidFile)) {
    Write-Output 'status=exited'; $diagnostic='job process disappeared before writing an exit status'
    if ($processState.Diagnostic) { $diagnostic+=': '+$processState.Diagnostic }
    $hasOutput=$true
} else { Write-Output 'status=not_found' }
if ($hasOutput) {
    $stdout=Read-MiaominalTail $out @@MAX@@; $stderr=Read-MiaominalTail $err @@MAX@@
    $truncated=$stdout.Truncated -or $stderr.Truncated
    Write-Output ('truncated='+[int]$truncated)
    Write-Output ('stdout_b64='+[Convert]::ToBase64String($stdout.Bytes,0,$stdout.Count))
    Write-Output ('stderr_b64='+[Convert]::ToBase64String($stderr.Bytes,0,$stderr.Count))
    if ($diagnostic) {
        $diagnosticBytes=(New-Object Text.UTF8Encoding($false)).GetBytes($diagnostic)
        Write-Output ('diagnostic_b64='+[Convert]::ToBase64String($diagnosticBytes))
    }
}
